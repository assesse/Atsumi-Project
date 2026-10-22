//! Pinterest-owned official documents retain all credentials. Only validated
//! content metadata and boolean authentication proof cross the native boundary.
use crate::{
    interface::AppState,
    pinterest::{PinterestDiscovery, PinterestPin},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State, Webview, WebviewUrl, WebviewWindowBuilder};

const ACCOUNT: &str = "pinterest-account";
const ERROR: &str =
    "Pinterest 세션 목록을 확인하지 못했습니다. 공식 페이지에서 로그인 상태를 확인해 주세요.";
const STALE: &str = "Pinterest 로그인 상태가 변경되었습니다. 다시 불러와 주세요.";
const MAX_RESULT: usize = 512 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    pub status: &'static str,
    pub busy: bool,
    pub open: bool,
    pub generation: u64,
    pub error: Option<String>,
}
impl Default for AccountSnapshot {
    fn default() -> Self {
        Self {
            status: "unknown",
            busy: false,
            open: false,
            generation: 0,
            error: None,
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    resource: String,
    options: Value,
    bookmark: String,
}
#[derive(Clone)]
struct Cursor {
    generation: u64,
    created: Instant,
    kind: String,
    input: String,
    path: String,
    plan: Plan,
    seen: HashSet<String>,
    hops: u16,
}
#[derive(Clone, Default)]
pub struct PinterestSession {
    generation: Arc<AtomicU64>,
    epoch: Arc<Mutex<()>>,
    account: Arc<Mutex<AccountSnapshot>>,
    cursors: Arc<Mutex<HashMap<String, Cursor>>>,
    account_probe: Arc<AtomicBool>,
    account_proof: Arc<AtomicU64>,
    account_proof_generation: Arc<AtomicU64>,
    account_flow: Arc<AtomicU64>,
    account_expected: Arc<Mutex<Option<bool>>>,
    account_lane: Arc<tokio::sync::Mutex<()>>,
}
pub struct SessionPage {
    pub discovery: PinterestDiscovery,
    pub cursor: Option<String>,
    pub auth_status: &'static str,
    pub generation: u64,
}
fn allowed(url: &tauri::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(
            url.host_str(),
            Some("pinterest.com" | "www.pinterest.com" | "kr.pinterest.com")
        )
}
fn equivalent(expected: &tauri::Url, candidate: &tauri::Url) -> bool {
    allowed(expected)
        && allowed(candidate)
        && expected.path() == candidate.path()
        && expected.query() == candidate.query()
}
fn trusted(window: &Webview) -> Result<(), String> {
    let url = window.url().map_err(|_| ERROR)?;
    if crate::frontend_origin::trusted_main(
        window.label(),
        &url,
        cfg!(debug_assertions),
        window.app_handle().config().build.dev_url.as_ref(),
    ) {
        Ok(())
    } else {
        Err(ERROR.into())
    }
}
fn profile(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    crate::browser_identity::profile(app).map_err(|_| ERROR.into())
}
fn valid_plan(plan: &Plan) -> bool {
    matches!(
        plan.resource.as_str(),
        "HomefeedResource"
            | "UserHomefeedResource"
            | "BaseSearchResource"
            | "UserPinsResource"
            | "BoardFeedResource"
            | "BoardsFeedResource"
    ) && !plan.bookmark.is_empty()
        && plan.bookmark != "-end-"
        && plan.bookmark.len() <= 8192
        && !plan.bookmark.chars().any(char::is_control)
        && plan.options.as_object().is_some_and(|options| {
            options.len() <= 32
                && options.iter().all(|(key, value)| {
                    matches!(
                        key.as_str(),
                        "add_vase"
                            | "field_set_key"
                            | "gated"
                            | "is_own_profile_pins"
                            | "page_size"
                            | "username"
                            | "board_id"
                            | "query"
                            | "scope"
                            | "rs"
                            | "source_pos"
                            | "redux_normalize_feed"
                            | "sort"
                            | "filter_stories"
                            | "in_nux"
                            | "prepend"
                            | "is_prefetch"
                            | "num_pins"
                    ) && (value.is_boolean()
                        || value
                            .as_f64()
                            .is_some_and(|n| n.is_finite() && n.abs() <= 10000.0)
                        || value
                            .as_str()
                            .is_some_and(|s| s.len() <= 1200 && !s.chars().any(char::is_control)))
                        && (!matches!(key.as_str(), "page_size" | "num_pins")
                            || value.as_u64().is_some_and(|n| (1..=60).contains(&n)))
                })
        })
}


/// Bind an observed plan to the native request; the renderer never supplies it.
fn plan_matches_request(plan: &Plan, kind: &str, input: &str, path: &str) -> bool {
    if kind == "saved" {
        return input.is_empty() && valid_plan(plan) && plan.resource == "BoardsFeedResource"
            && saved_owner(path).is_some_and(|owner| plan.options["username"].as_str() == Some(owner));
    }
    if !valid_plan(plan)
        || !crate::pinterest::session_discovery_path(input, kind)
            .is_ok_and(|expected| expected == path)
    {
        return false;
    }
    match kind {
        "home" => matches!(plan.resource.as_str(), "HomefeedResource" | "UserHomefeedResource"),
        "keyword" => {
            plan.resource == "BaseSearchResource"
                && plan.options["query"].as_str() == Some(input.trim())
                && plan.options["scope"].as_str() == Some("pins")
        }
        "profile" => matches!(plan.resource.as_str(), "UserPinsResource" | "BoardFeedResource"),
        _ => false,
    }
}
fn saved_owner(path: &str) -> Option<&str> {
    let owner = path.strip_prefix('/')?.strip_suffix("/_saved/")?;
    if owner.is_empty() || owner.len() > 100 || !owner.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') {
        return None;
    }
    crate::pinterest::session_discovery_path(path, "profile")
        .is_ok_and(|canonical| canonical == path).then_some(owner)
}
fn empty_discovery_is_error(pins_empty: bool, boards_empty: bool, exhausted: bool, has_next: bool, followup: bool) -> bool {
    pins_empty && boards_empty && !exhausted && !has_next && !followup
}

const PROJECTION_DIAGNOSTIC_LIMIT: u64 = 64;
static PROJECTION_DIAGNOSTICS: AtomicU64 = AtomicU64::new(0);

fn claim_projection_diagnostic(counter: &AtomicU64) -> bool {
    counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        (count < PROJECTION_DIAGNOSTIC_LIMIT).then_some(count.saturating_add(1))
    }).is_ok()
}

/// Only closed codes/counts cross into tracing. Never forward the document's map.
#[derive(Debug, Serialize)]
struct ProjectionDiagnostic {
    stage: &'static str,
    source: &'static str,
    reason: &'static str,
    initial: u64,
    key: u64,
    data: u64,
    entry: u64,
    bookmark: u64,
    locations: u64,
    resources: u64,
    entries: u64,
    structured: u64,
    dom: u64,
    cursor: bool,
    exhausted: bool,
    limited: bool,
}
impl ProjectionDiagnostic {
    fn from_value(value: &Value, kind: &str, previous: Option<&Plan>, cursor: bool) -> Option<Self> {
        let stage = match kind {
            "home" => "pinterest_home_projection",
            "keyword" => "pinterest_keyword_projection",
            "profile" => "pinterest_profile_projection",
            _ => return None,
        };
        let diagnostic = value.get("diagnostics")?.as_object()?;
        let item = |key: &str| diagnostic.get(key).unwrap_or(&Value::Null);
        let code = |key: &str, allowed: &[&str]| {
            item(key).as_str().and_then(|text| allowed.iter().position(|known| *known == text))
                .map_or(0, |index| index as u64 + 1)
        };
        let count = |key: &str, max| item(key).as_u64().filter(|n| *n <= max).unwrap_or(0);
        let source = match item("sourceBranch").as_str() {
            Some("hydration") => "hydration",
            Some("dom_fallback") => "dom_fallback",
            Some("resource_response") => "resource_response",
            _ => "none",
        };
        let mut reason = match item("planReject").as_str() {
            Some("none") => "NONE",
            Some("missing_bookmark") => "MISSING_BOOKMARK",
            Some("end") => "END",
            Some("key_shape") => "KEY_SHAPE",
            Some("too_many_options") => "TOO_MANY_OPTIONS",
            Some("unknown_option") => "UNKNOWN_OPTION",
            Some("option_type") => "OPTION_TYPE",
            Some("option_size") => "OPTION_SIZE",
            Some("context_mismatch") => "CONTEXT_MISMATCH",
            Some("same_bookmark") => "SAME_BOOKMARK",
            _ => "UNKNOWN",
        };
        if !value["continuation"].is_null() {
            reason = match serde_json::from_value::<Plan>(value["continuation"].clone()) {
                Ok(next) if valid_plan(&next) => {
                    if previous.is_some_and(|old| old.bookmark == next.bookmark) {
                        "SAME_BOOKMARK"
                    } else if !cursor {
                        "NATIVE_BOUNDS"
                    } else {
                        "NONE"
                    }
                }
                _ => "NATIVE_BOUNDS",
            };
        }
        let exhausted = value["exhausted"] == true;
        Some(Self {
            stage, source, reason,
            initial: code("initialShape", &["absent", "over_limit", "invalid_json", "redux_object", "redux_missing"]),
            key: code("keyShape", &["pairs", "object", "other", "invalid_json"]),
            data: code("dataShape", &["array", "results_array", "other", "missing"]),
            entry: code("entryShape", &["pin_objects", "id_only", "mixed", "empty", "other"]),
            bookmark: code("bookmarkShape", &["absent", "empty", "end", "string", "array", "other"]),
            locations: count("bookmarkLocationMask", 15),
            resources: count("knownResourceMask", 31),
            entries: count("resourceEntryCount", 4),
            structured: count("structuredPinCount", 60),
            dom: count("domPinCount", 60),
            cursor, exhausted, limited: !cursor && !exhausted,
        })
    }
    fn emit(&self) {
        // Existing SafeFields only; numeric field mapping is documented in the audit.
        tracing::info!(
            diag_stage = self.stage, diag_operation = self.source, error_code = self.reason,
            schema_version = 1u64, status = self.initial, attempt = self.key,
            width = self.data, height = self.entry, track_index = self.bookmark,
            init_bytes = self.locations, metadata_requests = self.resources,
            pages = self.entries, count = self.structured, eligible = self.dom,
            success = self.cursor, finished = self.exhausted, incremental = self.limited,
            "Pinterest bounded projection shape"
        );
    }
}
fn log_projection_diagnostic(value: &Value, kind: &str, previous: Option<&Plan>, cursor: bool) {
    if let Some(diagnostic) = ProjectionDiagnostic::from_value(value, kind, previous, cursor) {
        if claim_projection_diagnostic(&PROJECTION_DIAGNOSTICS) {
            diagnostic.emit();
        }
    }
}

fn resource_url(path: &str, plan: &Plan) -> Result<tauri::Url, String> {
    if !valid_plan(plan) {
        return Err(ERROR.into());
    }
    let mut options = plan.options.clone();
    options["bookmarks"] = json!([plan.bookmark]);
    let mut url: tauri::Url = format!("https://www.pinterest.com/resource/{}/get/", plan.resource)
        .parse()
        .map_err(|_| ERROR)?;
    url.query_pairs_mut()
        .append_pair("source_url", path)
        .append_pair(
            "data",
            &json!({ "options": options, "context": {} }).to_string(),
        );
    Ok(url)
}
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Pinterest 요청을 취소했습니다.".into())
    } else {
        Ok(())
    }
}
fn continuation_within_limits(hops: u16, seen: usize) -> bool {
    hops <= 100 && seen <= 6000
}
fn window_is_current(expected: u64, current: u64, admitted: bool, requested: bool) -> bool {
    expected == current && admitted && requested
}

impl PinterestSession {
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    pub fn detach(&self, app: &AppHandle) {
        self.account_flow.fetch_add(1, Ordering::AcqRel);
        self.invalidate();
        *self
            .account_expected
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
        for (label, view) in app.webview_windows() {
            if label.starts_with("pinterest-query-") || label == ACCOUNT || label.starts_with("pinterest-account-social-") {
                let _ = view.close();
            }
        }
    }
    pub(crate) fn with_generation<T>(
        &self,
        generation: u64,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let _epoch = self.epoch.lock().unwrap_or_else(|p| p.into_inner());
        if self.generation() != generation {
            return Err(STALE.into());
        }
        action()
    }
    fn snapshot(&self, app: &AppHandle) -> AccountSnapshot {
        let mut value = self
            .account
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        value.open = app.get_webview_window(ACCOUNT).is_some();
        if !value.open {
            value.busy = false;
        }
        value.generation = self.generation();
        value
    }
    fn invalidate(&self) -> u64 {
        let _epoch = self.epoch.lock().unwrap_or_else(|p| p.into_inner());
        self.invalidate_locked()
    }
    fn invalidate_locked(&self) -> u64 {
        self.cursors
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        crate::pinterest::clear_remote_cache();
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }
    fn proof(
        &self,
        generation: u64,
        authenticated: Option<bool>,
    ) -> Result<(&'static str, u64), String> {
        let _epoch = self.epoch.lock().unwrap_or_else(|p| p.into_inner());
        if self.generation() != generation {
            return Err(STALE.into());
        }
        let status = match authenticated {
            Some(true) => "signedIn",
            Some(false) => "signedOut",
            None => "unknown",
        };
        let mut account = self.account.lock().unwrap_or_else(|p| p.into_inner());
        if status != account.status {
            account.generation = self.invalidate_locked();
            account.status = status;
        }
        if status == "unknown" {
            account.error = Some(ERROR.into());
        } else {
            account.error = None;
            self.account_proof_generation
                .store(self.generation(), Ordering::Release);
            self.account_proof.fetch_add(1, Ordering::AcqRel);
        }
        Ok((status, self.generation()))
    }
    fn failed_proof(&self, generation: u64, revision: u64) {
        let _epoch = self.epoch.lock().unwrap_or_else(|p| p.into_inner());
        // A late failing request must never revoke a newer verified session.
        if self.generation() != generation || self.account_proof.load(Ordering::Acquire) != revision
        {
            return;
        }
        let mut account = self.account.lock().unwrap_or_else(|p| p.into_inner());
        if account.status != "unknown" {
            account.generation = self.invalidate_locked();
            account.status = "unknown";
        }
        account.error = Some(ERROR.into());
    }
    fn pin_proof(&self, generation: u64, authenticated: Option<bool>) -> Result<bool, String> {
        let Some(authenticated) = authenticated else {
            return Ok(false);
        };
        self.proof(generation, Some(authenticated))?;
        // Losing a session revokes the current request as well as its cache.
        self.with_generation(generation, || Ok(true))
    }
    /// One disposable, hidden official document. No credentials leave its browser profile.
    fn observe(
        &self,
        app: &AppHandle,
        url: tauri::Url,
        request: Value,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        check(cancel)?;
        if !allowed(&url) {
            return Err(ERROR.into());
        }
        let generation = self.generation();
        let nonce = uuid::Uuid::new_v4().to_string();
        let label = format!("pinterest-query-{nonce}");
        let mut request = request;
        request["nonce"] = json!(nonce);
        let script = include_str!("pinterest_session.js")
            .replace("__ATSUMI_PINTEREST_REQUEST__", &request.to_string());
        let directory = profile(app)?;
        let creating = Arc::new(AtomicBool::new(true));
        let creation = creating.clone();
        let target = url.clone();
        let owned = app.clone();
        let name = label.clone();
        let owner = self.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            if !creation.load(Ordering::Acquire) || owner.generation() != generation {
                return;
            }
            if owned
                .state::<AppState>()
                .begin_platform_work(|| Ok(()))
                .is_err()
            {
                let _ = tx.send(Err(ERROR.to_string()));
                return;
            }
            let navigation = target.clone();
            let built = WebviewWindowBuilder::new(&owned, &name, WebviewUrl::External(target))
                .title("Pinterest 목록 조회")
                .visible(false)
                .focused(false)
                .skip_taskbar(true)
                .data_directory(directory)
                .browser_extensions_enabled(false)
                .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
                .on_navigation(move |url| {
                    equivalent(&navigation, url) || url.as_str() == "about:blank"
                })
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .on_download(|_, _| false)
                .build();
            if !window_is_current(
                generation,
                owner.generation(),
                owned
                    .state::<AppState>()
                    .begin_platform_work(|| Ok(()))
                    .is_ok(),
                creation.load(Ordering::Acquire),
            ) {
                if let Ok(window) = built {
                    let _ = window.close();
                }
                let _ = tx.send(Err(STALE.to_string()));
                return;
            }
            let _ = tx.send(built.map(|_| ()).map_err(|_| ERROR.to_string()));
        })
        .map_err(|_| ERROR)?;
        let outcome = (|| {
            rx.recv_timeout(Duration::from_secs(5))
                .map_err(|_| ERROR)??;
            let view = app.get_webview_window(&label).ok_or(ERROR)?;
            let started = Instant::now();
            let mut last_auth = None;
            while started.elapsed() < Duration::from_secs(12) {
                check(cancel)?;
                if generation != self.generation() {
                    return Err(STALE.into());
                }
                if !equivalent(&url, &view.url().map_err(|_| ERROR)?) {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                view.eval_with_callback(script.clone(), move |value| {
                    let _ = tx.send(value);
                })
                .map_err(|_| ERROR)?;
                let raw = rx.recv_timeout(Duration::from_secs(2)).map_err(|_| ERROR)?;
                if raw.len() > MAX_RESULT
                    || generation != self.generation()
                    || !equivalent(&url, &view.url().map_err(|_| ERROR)?)
                {
                    return Err(STALE.into());
                }
                let value: Value = serde_json::from_str(&raw).map_err(|_| ERROR)?;
                if value["nonce"] != nonce {
                    return Err(ERROR.into());
                }
                last_auth = value["authenticated"].as_bool().or(last_auth);
                if value["state"] == "ready" || value["state"] == "unavailable" {
                    return Ok(value);
                }
                std::thread::sleep(Duration::from_millis(180));
            }
            if last_auth == Some(false) {
                self.proof(generation, last_auth)?;
                return Err("Pinterest 목록을 확인하지 못했습니다. 홈 피드·검색은 로그인 후 다시 시도해 주세요.".into());
            }
            Err(ERROR.into())
        })();
        creating.store(false, Ordering::Release);
        if let Some(view) = app.get_webview_window(&label) {
            let _ = view.close();
        }
        outcome
    }
    pub fn discover(
        &self,
        app: &AppHandle,
        input: &str,
        kind: &str,
        cursor: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<SessionPage, String> {
        let requested_generation = self.generation();
        let path = if kind == "saved" {
            if !input.is_empty() { return Err(ERROR.into()); }
            let owner = self.observe(app, "https://www.pinterest.com/".parse().unwrap(), json!({"kind":"owner"}), cancel)?;
            let (status, owner_generation) = self.proof(requested_generation, owner["authenticated"].as_bool())?;
            if status != "signedIn" || owner_generation != requested_generation {
                return Err("내 보드는 Pinterest 로그인 후 이용할 수 있습니다.".into());
            }
            let path = owner["ownerPath"].as_str().filter(|path| saved_owner(path).is_some())
                .ok_or("로그인한 계정의 보드 주소를 확인하지 못했습니다. Pinterest에서 내 프로필을 확인해 주세요.")?;
            path.to_owned()
        } else {
            crate::pinterest::session_discovery_path(input, kind)?
        };
        let previous = if let Some(id) = cursor {
            if uuid::Uuid::parse_str(id).is_err() {
                return Err(STALE.into());
            }
            let cursors = self.cursors.lock().unwrap_or_else(|p| p.into_inner());
            let value = cursors.get(id).ok_or(STALE)?.clone();
            if value.generation != self.generation()
                || value.created.elapsed() > Duration::from_secs(300)
                || value.kind != kind
                || value.input != input
                || value.path != path
            {
                return Err(STALE.into());
            }
            Some(value)
        } else {
            None
        };
        let plan = previous.as_ref().map(|value| &value.plan);
        if plan.is_some_and(|plan| !plan_matches_request(plan, kind, input, &path)) {
            return Err(STALE.into());
        }
        if plan.is_some() {
            // Resource documents contain only pin data, not authentication
            // proof. Verify the same profile independently before continuing.
            if !self.refresh_account(app, cancel)? {
                return Err(ERROR.into());
            }
            self.with_generation(requested_generation, || Ok(()))?;
        }
        let url = match plan {
            Some(plan) => resource_url(&path, plan)?,
            None => format!("https://www.pinterest.com{path}")
                .parse()
                .map_err(|_| ERROR)?,
        };
        let value = self.observe(app, url, json!({ "kind": kind, "input": input, "owner": if kind == "saved" { saved_owner(&path) } else { None }, "resource": plan.as_ref().map(|p| &p.resource), "options": plan.as_ref().map(|p| &p.options), "bookmark": plan.as_ref().map(|p| &p.bookmark) }), cancel)?;
        let (auth_status, generation) = if plan.is_none() {
            self.proof(requested_generation, value["authenticated"].as_bool())?
        } else {
            self.with_generation(requested_generation, || {
                Ok((self.snapshot(app).status, requested_generation))
            })?
        };
        let mut pins = crate::pinterest::session_pins(&value["pins"], None)?;
        let boards = crate::pinterest::session_boards(&value["boards"]);
        let mut seen = previous
            .as_ref()
            .map(|c| c.seen.clone())
            .unwrap_or_default();
        pins.retain(|pin| seen.insert(pin.id.clone()));
        let next = serde_json::from_value::<Plan>(value["continuation"].clone())
            .ok()
            .filter(|next| plan_matches_request(next, kind, input, &path))
            .filter(|next| {
                plan.as_ref()
                    .is_none_or(|old| old.bookmark != next.bookmark)
            });
        if empty_discovery_is_error(pins.is_empty(), boards.is_empty(), value["exhausted"] == true, next.is_some(), previous.is_some()) {
            return Err(ERROR.into());
        }
        let hops = previous.as_ref().map_or(1, |c| c.hops.saturating_add(1));
        let cursor = self.with_generation(generation, || {
            Ok(next
                .filter(|_| continuation_within_limits(hops, seen.len()))
                .map(|plan| {
                    let id = uuid::Uuid::new_v4().to_string();
                    let mut cursors = self.cursors.lock().unwrap_or_else(|p| p.into_inner());
                    cursors.retain(|_, c| {
                        c.generation == generation && c.created.elapsed() < Duration::from_secs(300)
                    });
                    if cursors.len() >= 32 {
                        if let Some(oldest) = cursors
                            .iter()
                            .min_by_key(|(_, c)| c.created)
                            .map(|(id, _)| id.clone())
                        {
                            cursors.remove(&oldest);
                        }
                    }
                    cursors.insert(
                        id.clone(),
                        Cursor {
                            generation,
                            created: Instant::now(),
                            kind: kind.into(),
                            input: input.into(),
                            path: path.clone(),
                            plan,
                            seen,
                            hops,
                        },
                    );
                    id
                }))
        })?;
        self.with_generation(generation, || {
            crate::pinterest::remember_session_discovery(&pins);
            Ok(())
        })?;
        log_projection_diagnostic(&value, kind, plan, cursor.is_some());
        Ok(SessionPage {
            discovery: PinterestDiscovery {
                title: if kind == "saved" {
                    "내 보드".into()
                } else if kind == "home" {
                    "홈 피드".into()
                } else {
                    value["title"]
                        .as_str()
                        .filter(|t| !t.is_empty())
                        .unwrap_or(input)
                        .chars()
                        .take(200)
                        .collect()
                },
                path,
                pins,
                boards,
                limited: cursor.is_none() && value["exhausted"] != true,
                cursor: None,
                auth_status: None,
                generation: None,
            },
            cursor,
            auth_status,
            generation,
        })
    }
    pub fn pin(
        &self,
        app: &AppHandle,
        id: &str,
        cancel: &AtomicBool,
    ) -> Result<PinterestPin, String> {
        let generation = self.generation();
        let id = crate::pinterest::session_pin_id(id)?;
        let value = self.observe(
            app,
            format!("https://www.pinterest.com/pin/{id}/")
                .parse()
                .map_err(|_| ERROR)?,
            json!({ "kind": "pin", "id": id }),
            cancel,
        )?;
        if !self.pin_proof(generation, value["authenticated"].as_bool())?
            && !self.refresh_account(app, cancel)?
        {
            return Err(ERROR.into());
        }
        self.with_generation(generation, || {
            crate::pinterest::session_pins(&value["pins"], Some(&id))?
                .into_iter()
                .next()
                .ok_or_else(|| ERROR.into())
        })
    }
    fn refresh_account(&self, app: &AppHandle, cancel: &AtomicBool) -> Result<bool, String> {
        let previous_proof = self.account_proof.load(Ordering::Acquire);
        if self.account_probe.swap(true, Ordering::AcqRel) {
            // Reuse a concurrently completed fresh proof, not a remembered
            // login badge. Status polling must not randomly fail pagination.
            return self.wait_account_proof(previous_proof, cancel, Duration::from_secs(15));
        }
        struct Reset(Arc<AtomicBool>);
        impl Drop for Reset {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _reset = Reset(self.account_probe.clone());
        let generation = self.generation();
        let value = match self.observe(
            app,
            "https://www.pinterest.com/".parse().unwrap(),
            json!({ "kind": "status" }),
            cancel,
        ) {
            Ok(value) => value,
            Err(error) => {
                if !cancel.load(Ordering::Acquire) {
                    self.failed_proof(generation, previous_proof);
                }
                return Err(error);
            }
        };
        let (status, _) = self.proof(generation, value["authenticated"].as_bool())?;
        if status == "unknown" {
            return Ok(false);
        }
        Ok(true)
    }
    fn wait_account_proof(
        &self,
        previous_proof: u64,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<bool, String> {
        let started = Instant::now();
        while self.account_probe.load(Ordering::Acquire)
            && self.account_proof.load(Ordering::Acquire) == previous_proof
        {
            check(cancel)?;
            if started.elapsed() > timeout {
                return Err(ERROR.into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        check(cancel)?;
        Ok(self.account_proof.load(Ordering::Acquire) != previous_proof
            && self.account_proof_generation.load(Ordering::Acquire) == self.generation())
    }
    fn finish_account(&self, app: &AppHandle, flow: u64) {
        // The same lane owns creation and closure; a late old completion must
        // not close a replacement account window with the same stable label.
        let Ok(_lane) = self.account_lane.try_lock() else {
            return;
        };
        if flow != self.account_flow.load(Ordering::Acquire) {
            return;
        }
        let expected = *self
            .account_expected
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let Some(expected) = expected else {
            return;
        };
        let status = self.snapshot(app).status;
        if status != if expected { "signedIn" } else { "signedOut" } {
            return;
        }
        if let Some(account) = app.get_webview_window(ACCOUNT) {
            if account
                .url()
                .is_ok_and(|url| allowed(&url) && url.path() == "/")
            {
                if account.close().is_ok() {
                    self.clear_account_flow(flow, None, false);
                } else if flow == self.account_flow.load(Ordering::Acquire) {
                    self.account.lock().unwrap_or_else(|p| p.into_inner()).error = Some(
                        "Pinterest 로그인 창을 닫지 못했습니다. 창의 닫기 버튼으로 종료해 주세요."
                            .into(),
                    );
                }
            }
        }
    }
    fn clear_account_flow(&self, flow: u64, error: Option<String>, destroyed: bool) {
        if flow != self.account_flow.load(Ordering::Acquire) {
            return;
        }
        if destroyed {
            self.account_flow.fetch_add(1, Ordering::AcqRel);
        }
        let mut account = self.account.lock().unwrap_or_else(|p| p.into_inner());
        account.busy = false;
        account.error = error;
        *self
            .account_expected
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }
}

#[tauri::command]
pub async fn pinterest_account_status(
    window: Webview,
    state: State<'_, AppState>,
    session: State<'_, PinterestSession>,
) -> Result<AccountSnapshot, String> {
    trusted(&window)?;
    state.begin_platform_work(|| Ok(()))?;
    let app = window.app_handle().clone();
    let worker = session.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let flow = worker.account_flow.load(Ordering::Acquire);
        match worker.refresh_account(&app, &AtomicBool::new(false)) {
            Ok(true) => worker.finish_account(&app, flow),
            Err(_) | Ok(false) => {}
        }
        Ok(worker.snapshot(&app))
    })
    .await
    .map_err(|_| ERROR)?
}
#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_account_open(
    window: Webview,
    state: State<'_, AppState>,
    session: State<'_, PinterestSession>,
    action: String,
) -> Result<AccountSnapshot, String> {
    trusted(&window)?;
    if !matches!(action.as_str(), "login" | "logout") {
        return Err(ERROR.into());
    }
    let _lane = session.account_lane.lock().await;
    let app = window.app_handle().clone();
    if app.get_webview_window(ACCOUNT).is_some() {
        return Ok(session.snapshot(&app));
    }
    let directory = profile(&app)?;
    state.begin_platform_work(|| Ok(session.invalidate()))?;
    let flow = session.account_flow.fetch_add(1, Ordering::AcqRel) + 1;
    {
        let mut account = session.account.lock().unwrap_or_else(|p| p.into_inner());
        account.status = "unknown";
        account.busy = true;
        account.error = None;
    }
    let expected = action == "login";
    let host = session.inner().clone();
    let lifecycle = host.clone();
    let owned = app.clone();
    *host
        .account_expected
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(expected);
    let url = if expected {
        "https://www.pinterest.com/login/"
    } else {
        "https://www.pinterest.com/logout/"
    };
    let current = host.clone();
    let popups = crate::browser_identity::LoginPopups::new(
        crate::browser_identity::Service::Pinterest, ACCOUNT,
        move || current.account_flow.load(Ordering::Acquire) == flow,
    );
    let popup_open = popups.clone();
    let popup_app = app.clone();
    let navigation = host.clone();
    let built =
        WebviewWindowBuilder::new(&app, ACCOUNT, WebviewUrl::External(url.parse().unwrap()))
            .title("Pinterest 로그인")
            .inner_size(900.0, 740.0)
            .data_directory(directory)
            .browser_extensions_enabled(false)
            .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
            .on_navigation(move |url| navigation.account_flow.load(Ordering::Acquire) == flow
                && crate::browser_identity::account_url(crate::browser_identity::Service::Pinterest, url))
            .on_new_window(move |url, features| popup_open.open(&popup_app, url, features))
            .on_download(|_, _| false)
            .on_page_load(move |view, event| {
                if !matches!(event.event(), tauri::webview::PageLoadEvent::Finished)
                    || event.url().path() != "/"
                    || !allowed(event.url())
                {
                    return;
                }
                let host = lifecycle.clone();
                let app = view.app_handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if host.account_flow.load(Ordering::Acquire) != flow {
                        return;
                    }
                    if matches!(
                        host.refresh_account(&app, &AtomicBool::new(false)),
                        Ok(true)
                    ) {
                        host.finish_account(&app, flow);
                    }
                });
            })
            .build();
    let built = match built {
        Ok(window) => window,
        Err(_) => {
            host.clear_account_flow(flow, Some(ERROR.into()), false);
            return Err(ERROR.into());
        }
    };
    if !window_is_current(
        flow,
        host.account_flow.load(Ordering::Acquire),
        state.begin_platform_work(|| Ok(())).is_ok(),
        true,
    ) {
        let _ = built.close();
        return Err(STALE.into());
    }
    let closed = host.clone();
    let close_app = app.clone();
    built.on_window_event(move |event| {
        if !matches!(event, tauri::WindowEvent::Destroyed) {
            return;
        }
        popups.close(&close_app);
        let closed = closed.clone();
        tauri::async_runtime::spawn(async move {
            let _lane = closed.account_lane.lock().await;
            closed.clear_account_flow(flow, None, true);
        });
    });
    Ok(host.snapshot(&owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_official_origins_only() {
        for url in [
            "https://www.pinterest.com/",
            "https://kr.pinterest.com/login/",
        ] {
            assert!(allowed(&url.parse().unwrap()));
        }
        for url in [
            "http://www.pinterest.com/",
            "https://user@www.pinterest.com/",
            "https://www.pinterest.com.evil.test/",
            "https://www.pinterest.com:444/",
            "https://api.pinterest.com/",
            "https://accounts.google.com/",
            "https://consent.google.com/",
            "https://www.youtube.com/",
        ] {
            assert!(!allowed(&url.parse().unwrap()));
        }
    }
    #[test]
    fn continuation_cannot_be_a_url_or_secret_carrier() {
        let mut plan = Plan {
            resource: "UserPinsResource".into(),
            options: json!({"username":"pinterest","gated":true}),
            bookmark: "opaque-next".into(),
        };
        assert!(valid_plan(&plan));
        assert!(resource_url("/pinterest/", &plan)
            .unwrap()
            .as_str()
            .starts_with("https://www.pinterest.com/resource/UserPinsResource/get/"));
        plan.resource = "../../logout".into();
        assert!(!valid_plan(&plan));
        plan.resource = "UserPinsResource".into();
        plan.options["access_token"] = json!("not-permitted");
        assert!(!valid_plan(&plan));
    }
    #[test]
    fn projection_never_reads_credentials_or_invokes_privileged_actions() {
        let script = include_str!("pinterest_session.js");
        for word in [
            "document.cookie",
            "localStorage",
            "sessionStorage",
            "fetch(",
            "XMLHttpRequest",
            "__TAURI",
            "document.forms",
            "outerHTML",
        ] {
            assert!(!script.contains(word), "{word}");
        }
        assert!(script.contains("isAuthenticated"));
        assert!(script.contains("pins.length < 60"));
    }
    #[test]
    fn stale_proof_cannot_restore_a_previous_session() {
        let session = PinterestSession::default();
        let original = session.generation();
        let (_, signed_in) = session.proof(original, Some(true)).unwrap();
        assert!(signed_in > original);
        assert!(session.proof(original, Some(false)).is_err());
        assert_eq!(session.account.lock().unwrap().status, "signedIn");
        let (status, unknown) = session.proof(signed_in, None).unwrap();
        assert_eq!(status, "unknown");
        assert!(unknown > signed_in);
        assert!(session.with_generation(signed_in, || Ok(())).is_err());
    }
    #[test]
    fn publication_is_ordered_before_generation_invalidation() {
        let session = PinterestSession::default();
        let generation = session.generation();
        session
            .with_generation(generation, || {
                assert!(
                    session.epoch.try_lock().is_err(),
                    "publish must retain the generation fence"
                );
                Ok(())
            })
            .unwrap();
        session.invalidate();
        let invoked = AtomicBool::new(false);
        assert!(session
            .with_generation(generation, || {
                invoked.store(true, Ordering::Release);
                Ok(())
            })
            .is_err());
        assert!(!invoked.load(Ordering::Acquire));
    }
    #[test]
    fn concurrent_fresh_status_is_coalesced_and_waiting_can_be_cancelled() {
        let session = PinterestSession::default();
        session.account_probe.store(true, Ordering::Release);
        let worker = session.clone();
        let handle = std::thread::spawn(move || {
            worker
                .account_proof_generation
                .store(worker.generation(), Ordering::Release);
            worker.account_proof.fetch_add(1, Ordering::AcqRel);
            worker.account_probe.store(false, Ordering::Release);
        });
        assert!(session
            .wait_account_proof(0, &AtomicBool::new(false), Duration::from_secs(2))
            .unwrap());
        handle.join().unwrap();
        session.account_probe.store(true, Ordering::Release);
        assert!(session
            .wait_account_proof(1, &AtomicBool::new(true), Duration::from_secs(2))
            .is_err());
        session.invalidate();
        session.account_probe.store(false, Ordering::Release);
        assert!(!session
            .wait_account_proof(0, &AtomicBool::new(false), Duration::from_secs(2))
            .unwrap());
    }
    #[test]
    fn current_failure_revokes_proof_but_stale_failure_cannot_undo_newer_success() {
        let session = PinterestSession::default();
        let (_, generation) = session.proof(0, Some(true)).unwrap();
        session.account_proof.store(1, Ordering::Release);
        session.failed_proof(generation, 0);
        assert_eq!(session.account.lock().unwrap().status, "signedIn");
        session.failed_proof(generation, 1);
        assert_eq!(session.account.lock().unwrap().status, "unknown");
        assert!(session.with_generation(generation, || Ok(())).is_err());
        let (_, next) = session.proof(session.generation(), Some(true)).unwrap();
        session.failed_proof(generation, 1);
        assert_eq!(session.generation(), next);
        assert_eq!(session.account.lock().unwrap().status, "signedIn");
        assert!(session.account.lock().unwrap().error.is_none());
    }
    #[test]
    fn pin_document_session_loss_revokes_current_metadata_and_publish_lease() {
        let session = PinterestSession::default();
        let (_, generation) = session.proof(0, Some(true)).unwrap();
        assert!(session.pin_proof(generation, Some(true)).unwrap());
        assert!(!session.pin_proof(generation, None).unwrap());
        assert!(session.pin_proof(generation, Some(false)).is_err());
        assert_eq!(session.account.lock().unwrap().status, "signedOut");
        assert!(session.with_generation(generation, || Ok(())).is_err());
    }
    #[test]
    fn duplicate_only_page_hops_are_bounded_independently_of_unique_pin_count() {
        assert!(continuation_within_limits(100, 1));
        assert!(!continuation_within_limits(101, 1));
        assert!(continuation_within_limits(1, 6000));
        assert!(!continuation_within_limits(1, 6001));
    }
    #[test]
    fn post_build_guards_reject_shutdown_cancellation_and_replacement() {
        assert!(window_is_current(4, 4, true, true));
        assert!(!window_is_current(4, 5, true, true));
        assert!(!window_is_current(4, 4, false, true));
        assert!(!window_is_current(4, 4, true, false));
    }
    #[test]
    fn account_lifecycle_lane_and_flow_protect_a_replacement_window() {
        let session = PinterestSession::default();
        let _lane = session.account_lane.try_lock().unwrap();
        assert!(session.account_lane.try_lock().is_err());
        session.account_flow.store(2, Ordering::Release);
        session.account.lock().unwrap().busy = true;
        *session.account_expected.lock().unwrap() = Some(true);
        session.clear_account_flow(1, Some("stale failure".into()), true);
        assert!(session.account.lock().unwrap().busy);
        assert!(session.account.lock().unwrap().error.is_none());
        assert_eq!(*session.account_expected.lock().unwrap(), Some(true));
        session.clear_account_flow(2, None, true);
        assert!(!session.account.lock().unwrap().busy);
        assert_eq!(session.account_flow.load(Ordering::Acquire), 3);
        assert_eq!(*session.account_expected.lock().unwrap(), None);
    }
    #[test]
    fn projection_diagnostic_reconstructs_only_closed_bounded_fields() {
        let raw = json!({
            "diagnostics": {
                "sourceBranch": "dom_fallback", "initialShape": "redux_object",
                "knownResourceMask": 4, "resourceEntryCount": 2, "structuredPinCount": 0, "domPinCount": 12,
                "dataShape": "other", "entryShape": "id_only", "keyShape": "invalid_json",
                "bookmarkShape": "absent", "bookmarkLocationMask": 2, "planReject": "missing_bookmark",
                "account": "SECRET_ACCOUNT", "query": "SECRET_QUERY", "bookmark": "SECRET_BOOKMARK",
                "options": {"SECRET_OPTION": "SECRET_VALUE"}
            },
            "nonce": "SECRET_NONCE", "authenticated": true, "exhausted": false, "continuation": null
        });
        let diagnostic = ProjectionDiagnostic::from_value(&raw, "keyword", None, false).unwrap();
        let safe = serde_json::to_value(&diagnostic).unwrap();
        assert_eq!(safe["stage"], "pinterest_keyword_projection");
        assert_eq!(safe["source"], "dom_fallback");
        assert_eq!(safe["dom"], 12);
        assert_eq!(safe["reason"], "MISSING_BOOKMARK");
        assert_eq!(safe["limited"], true);
        assert!(!safe.to_string().contains("SECRET"));
        let hostile = json!({"diagnostics": {
            "sourceBranch":"SECRET_ACCOUNT", "initialShape":"SECRET_QUERY", "planReject":"SECRET_BOOKMARK",
            "knownResourceMask":32, "resourceEntryCount":5, "structuredPinCount":61, "domPinCount":-1,
            "bookmarkLocationMask":16, "dataShape":["SECRET_OPTION"], "keyShape":null
        }});
        let safe = serde_json::to_value(ProjectionDiagnostic::from_value(&hostile, "home", None, false).unwrap()).unwrap();
        assert_eq!(safe["source"], "none");
        assert_eq!(safe["reason"], "UNKNOWN");
        for name in ["initial", "resources", "entries", "structured", "dom", "locations", "data", "key"] {
            assert_eq!(safe[name], 0);
        }
        assert!(!safe.to_string().contains("SECRET"));
        assert!(ProjectionDiagnostic::from_value(&raw, "SECRET_KIND", None, false).is_none());
        assert!(ProjectionDiagnostic::from_value(&json!({}), "home", None, false).is_none());
    }
    #[test]
    fn projection_diagnostic_identifies_native_rejection_without_plan_values() {
        let plan = Plan { resource:"HomefeedResource".into(), options:json!({"page_size":25}), bookmark:"SECRET_BOOKMARK".into() };
        let value = json!({"diagnostics": {"planReject":"none"},
            "continuation":{"resource":"HomefeedResource","options":{"page_size":25,"query":"SECRET_QUERY"},"bookmark":"SECRET_BOOKMARK"}});
        assert_eq!(ProjectionDiagnostic::from_value(&value, "home", None, true).unwrap().reason, "NONE");
        assert_eq!(ProjectionDiagnostic::from_value(&value, "home", Some(&plan), false).unwrap().reason, "SAME_BOOKMARK");
        let invalid = json!({"diagnostics": {}, "continuation":{"resource":"HomefeedResource","options":{"page_size":61},"bookmark":"SECRET_BOOKMARK"}});
        let safe = ProjectionDiagnostic::from_value(&invalid, "home", None, false).unwrap();
        assert_eq!(safe.reason, "NATIVE_BOUNDS");
        assert!(!serde_json::to_string(&safe).unwrap().contains("SECRET"));
    }
    #[test]
    fn projection_diagnostic_trace_cannot_contain_arbitrary_document_values() {
        use tracing::{field::{Field, Visit}, Event, Subscriber};
        use tracing_subscriber::{layer::Context, prelude::*, Layer};
        #[derive(Default)]
        struct Fields(serde_json::Map<String, Value>);
        impl Visit for Fields {
            fn record_str(&mut self, field: &Field, value: &str) { self.0.insert(field.name().into(), json!(value)); }
            fn record_u64(&mut self, field: &Field, value: u64) { self.0.insert(field.name().into(), json!(value)); }
            fn record_bool(&mut self, field: &Field, value: bool) { self.0.insert(field.name().into(), json!(value)); }
            fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
        }
        struct Capture(Arc<Mutex<Vec<Value>>>);
        impl<S: Subscriber> Layer<S> for Capture {
            fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
                let mut fields = Fields::default();
                event.record(&mut fields);
                self.0.lock().unwrap().push(Value::Object(fields.0));
            }
        }
        let value = json!({"diagnostics": {
            "sourceBranch":"dom_fallback", "initialShape":"redux_object", "planReject":"missing_bookmark",
            "knownResourceMask":4, "domPinCount":12, "account":"SECRET_ACCOUNT", "query":"SECRET_QUERY",
            "bookmark":"SECRET_BOOKMARK", "options":{"SECRET_OPTION":"SECRET_VALUE"}
        }});
        let diagnostic = ProjectionDiagnostic::from_value(&value, "keyword", None, false).unwrap();
        let rows = Arc::new(Mutex::new(Vec::new()));
        tracing::subscriber::with_default(tracing_subscriber::registry().with(Capture(rows.clone())), || diagnostic.emit());
        let rows = rows.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], json!({
            "diag_stage":"pinterest_keyword_projection", "diag_operation":"dom_fallback", "error_code":"MISSING_BOOKMARK",
            "schema_version":1, "status":4, "attempt":0, "width":0, "height":0, "track_index":0, "init_bytes":0,
            "metadata_requests":4, "pages":0, "count":0, "eligible":12,
            "success":false, "finished":false, "incremental":true
        }));
        assert!(!serde_json::to_string(&*rows).unwrap().contains("SECRET"));
    }
    #[test]
    fn projection_diagnostic_budget_is_bounded_and_atomic() {
        let budget = Arc::new(AtomicU64::new(0));
        let handles: Vec<_> = (0..4).map(|_| {
            let budget = budget.clone();
            std::thread::spawn(move || (0..64).filter(|_| claim_projection_diagnostic(&budget)).count())
        }).collect();
        assert_eq!(handles.into_iter().map(|handle| handle.join().unwrap()).sum::<usize>(), 64);
        assert_eq!(budget.load(Ordering::Relaxed), 64);
        assert!(!claim_projection_diagnostic(&budget));
    }

    #[test]
    fn continuation_is_bound_to_native_kind_path_and_keyword_options() {
        let mut plan = Plan { resource: "BaseSearchResource".into(), options: json!({"query":"landscape","scope":"pins"}), bookmark:"opaque-next".into() };
        let path = crate::pinterest::session_discovery_path(" landscape ", "keyword").unwrap();
        assert!(plan_matches_request(&plan, "keyword", " landscape ", &path));
        assert!(!plan_matches_request(&plan, "keyword", "forest", &path));
        assert!(!plan_matches_request(&plan, "keyword", "landscape", "/"));
        assert!(!plan_matches_request(&plan, "home", "", "/"));
        plan.options["scope"] = json!("boards");
        assert!(!plan_matches_request(&plan, "keyword", "landscape", &path));
        plan.options = json!({});
        assert!(!plan_matches_request(&plan, "keyword", "landscape", &path));
        plan.resource = "UserHomefeedResource".into();
        plan.options = json!({"page_size":25});
        assert!(plan_matches_request(&plan, "home", "", "/"));
        assert!(!plan_matches_request(&plan, "profile", "pinterest", "/pinterest/"));
        plan.resource = "BoardFeedResource".into();
        assert!(!plan_matches_request(&plan, "home", "", "/"));
        assert!(plan_matches_request(&plan, "profile", "pinterest/board", "/pinterest/board/"));
    }
    #[test]
    fn unknown_transport_fields_still_fail_native_validation() {
        let plan = Plan { resource:"UserHomefeedResource".into(), options:json!({"page_size":25,"unknown_flag":true}), bookmark:"opaque".into() };
        assert!(!valid_plan(&plan));
        assert!(!plan_matches_request(&plan, "home", "", "/"));
    }
    #[test]
    fn empty_nonterminal_followup_keeps_prior_results_limited_not_finished() {
        assert!(empty_discovery_is_error(true, true, false, false, false));
        assert!(!empty_discovery_is_error(true, true, false, false, true));
        assert!(!empty_discovery_is_error(true, true, true, false, true));
        assert!(!empty_discovery_is_error(true, true, false, true, true));
        assert!(!empty_discovery_is_error(false, true, false, false, false));
    }

    #[test]
    fn saved_board_cursor_is_bound_to_the_verified_owner() {
        let plan = Plan { resource: "BoardsFeedResource".into(), options: json!({"username":"fixture","page_size":25}), bookmark:"next".into() };
        assert!(plan_matches_request(&plan, "saved", "", "/fixture/_saved/"));
        assert!(!plan_matches_request(&plan, "saved", "", "/different/_saved/"));
        assert!(!plan_matches_request(&plan, "saved", "fixture", "/fixture/_saved/"));
        assert!(!plan_matches_request(&plan, "home", "", "/"));
        for bad in ["/login/_saved/", "/fixture/board/", "/../_saved/", "/fixture%2Fother/_saved/", "https://evil.test/fixture/_saved/"] { assert!(saved_owner(bad).is_none()); }
    }

}
