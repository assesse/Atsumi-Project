//! SOOP watch-only VOD document. No media lease, capture, scheduler or account authority.
#[path = "original_vod_policy.rs"]
mod policy;
#[path = "original_vod_input_diagnostics.rs"]
mod input_diagnostics;
#[path = "original_vod_input_intent.rs"]
mod input_intent;
#[path = "original_vod_escape_intent.rs"]
mod escape_intent;
#[cfg(debug_assertions)]
#[path = "original_vod_click_probe.rs"]
mod click_probe;
use super::{
    documents::{Documents, Permit},
    hidden_viewport, lock, profile,
};
use crate::platform_native_geometry::{self, NativeViewport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager, Webview, WebviewBuilder, WebviewUrl};

const PREFIX: &str = "ATSUMI_SOOP_VOD_V1:";
const STALE: &str = "SOOP 영상 상태가 변경되었습니다. 다시 열어 주세요.";
const UNAVAILABLE: &str = "SOOP 원본 영상 플레이어를 준비하지 못했습니다.";
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub token: String,
    pub video_id: String,
    pub state: String,
    pub chat_state: String,
    pub fullscreen: bool,
    pub error: Option<String>,
}
#[derive(Deserialize)]
struct Envelope {
    id: String,
    epoch: u64,
    #[serde(rename = "type")]
    kind: String,
    #[serde(flatten)]
    fields: Value,
}
#[derive(Default)]
struct Registry {
    sessions: HashMap<String, Arc<Session>>,
    tokens: HashMap<String, String>,
    requests: HashMap<String, (String, String)>,
}
struct Session {
    id: String,
    label: String,
    video: String,
    document: Permit,
    closed: AtomicBool,
    opening: AtomicBool,
    epoch: AtomicU64,
    paint: Arc<AtomicU64>,
    status: Mutex<Snapshot>,
    fullscreen_grant: Mutex<Option<super::Grant>>,
    viewport: Mutex<Option<NativeViewport>>,
    sequences: Mutex<HashMap<String, u64>>,
    viewers: Mutex<Vec<String>>,
    mouse_intent: Arc<input_intent::Intent>,
    escape_intent: Arc<escape_intent::Intent>,
}
#[derive(Clone, Default)]
pub(super) struct WatchHost {
    registry: Arc<Mutex<Registry>>,
}
impl WatchHost {
    pub fn open(
        &self,
        app: &AppHandle,
        documents: &Documents,
        video: String,
        request: String,
        closing: Arc<AtomicBool>,
        tray: Arc<AtomicBool>,
    ) -> Result<Snapshot, String> {
        if closing.load(Ordering::Acquire)
            || !policy::valid_video(&video)
            || uuid::Uuid::parse_str(&request).is_err()
        {
            return Err(STALE.into());
        }
        let (session, token, created) = {
            let mut registry = lock(&self.registry);
            if closing.load(Ordering::Acquire) {
                return Err(STALE.into());
            }
            if let Some((bound, token)) = registry.requests.get(&request) {
                if bound != &video {
                    return Err(STALE.into());
                }
                return snapshot_locked(&registry, token);
            }
            let token = format!("soop-vod-{}", uuid::Uuid::new_v4());
            let existing = registry
                .sessions
                .values()
                .find(|s| s.video == video && !s.closed.load(Ordering::Acquire))
                .cloned();
            let (session, created) = if let Some(session) = existing {
                (session, false)
            } else {
                let document = documents.acquire()?;
                let id = uuid::Uuid::new_v4().to_string();
                let session = Arc::new(Session {
                    id: id.clone(),
                    label: format!("soop-original-vod-{}", uuid::Uuid::new_v4().simple()),
                    video: video.clone(),
                    document,
                    closed: AtomicBool::new(false),
                    opening: AtomicBool::new(true),
                    epoch: AtomicU64::new(0),
                    paint: Arc::new(AtomicU64::new(0)),
                    status: Mutex::new(Snapshot {
                        token: token.clone(),
                        video_id: video.clone(),
                        state: "loading".into(),
                        chat_state: "pending".into(),
                        fullscreen: false,
                        error: None,
                    }),
                    fullscreen_grant: Mutex::new(None),
                    viewport: Mutex::new(None),
                    sequences: Mutex::new(HashMap::new()),
                    viewers: Mutex::new(Vec::new()),
                    mouse_intent: Arc::new(input_intent::Intent::default()),
                    escape_intent: Arc::new(escape_intent::Intent::default()),
                });
                registry.sessions.insert(id, session.clone());
                (session, true)
            };
            registry.tokens.insert(token.clone(), session.id.clone());
            registry.requests.insert(request, (video, token.clone()));
            {
                let mut viewers = lock(&session.viewers);
                invalidate_input_intents(&session);
                viewers.push(token.clone());
            }
            (session, token, created)
        };
        if created {
            let result = self.create_view(app, &session, tray);
            session.opening.store(false, Ordering::Release);
            if let Err(error) = result {
                let _ = self.dispose(app, &session);
                return Err(error);
            }
            if closing.load(Ordering::Acquire) || session.closed.load(Ordering::Acquire) {
                let _ = self.dispose(app, &session);
                return Err(STALE.into());
            }
        }
        let mut result = lock(&session.status).clone();
        result.token = token;
        Ok(result)
    }
    fn session(&self, token: &str) -> Result<Arc<Session>, String> {
        let registry = lock(&self.registry);
        registry
            .tokens
            .get(token)
            .and_then(|id| registry.sessions.get(id))
            .cloned()
            .filter(|s| !s.closed.load(Ordering::Acquire))
            .ok_or_else(|| STALE.into())
    }
    pub fn snapshot(&self, token: &str) -> Result<Snapshot, String> {
        snapshot_locked(&lock(&self.registry), token)
    }
    pub fn viewport(
        &self,
        app: &AppHandle,
        token: &str,
        mut viewport: NativeViewport,
        tray: bool,
    ) -> Result<(), String> {
        viewport.validate()?;
        let session = self.session(token)?;
        let viewers = lock(&session.viewers);
        if !is_presenter(&viewers, token) {
            return Ok(());
        }
        let mut sequences = lock(&session.sequences);
        let previous = sequences.entry(token.into()).or_insert(0);
        if viewport.sequence <= *previous {
            return Ok(());
        }
        *previous = viewport.sequence;
        {
            let mut previous = lock(&session.viewport);
            if !input_intent::same_geometry(previous.as_ref(), Some(&viewport)) {
                invalidate_input_intents(&session);
            }
            *previous = Some(viewport.clone());
        }
        viewport.muted |= tray;
        paint(app, &session, viewport)
    }
    pub fn fullscreen(&self, app: &AppHandle, token: &str, enabled: bool) -> Result<(), String> {
        let session = self.session(token)?;
        let viewers = lock(&session.viewers);
        if !is_presenter(&viewers, token) {
            return Ok(());
        }
        drop(viewers);
        fullscreen_for(app, &session, enabled, Some(token.to_owned()))
    }
    pub fn close(&self, app: &AppHandle, token: &str) -> Result<(), String> {
        let session = {
            let mut registry = lock(&self.registry);
            release_viewer(&mut registry, token)
        };
        if let Some(session) = session {
            self.dispose(app, &session)?;
        }
        Ok(())
    }
    fn dispose(&self, app: &AppHandle, session: &Arc<Session>) -> Result<(), String> {
        session.closed.store(true, Ordering::Release);
        invalidate_input_intents(session);
        session.paint.fetch_add(1, Ordering::AcqRel);
        // Do not release capacity while add_child may still finish creating this document.
        if session.opening.load(Ordering::Acquire) {
            return Ok(());
        }
        if lock(&session.status).fullscreen {
            let _ = fullscreen(app, session, false);
        }
        if let Some(view) = app.get_webview(&session.label) {
            view.close().map_err(|_| {
                "SOOP 영상 창을 닫지 못했습니다. 앱을 정상 종료해 주세요.".to_string()
            })?;
        }
        session.document.release();
        let mut registry = lock(&self.registry);
        registry.sessions.remove(&session.id);
        registry.tokens.retain(|_, id| id != &session.id);
        let tokens = registry
            .tokens
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        registry
            .requests
            .retain(|_, (_, token)| tokens.contains(token));
        Ok(())
    }
    pub fn detach(&self, app: &AppHandle) {
        let sessions = lock(&self.registry)
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            let _ = self.dispose(app, &session);
        }
    }
    pub fn mute(&self, app: &AppHandle, muted: bool) {
        let sessions = lock(&self.registry)
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            let mut viewport = lock(&session.viewport)
                .clone()
                .unwrap_or_else(hidden_viewport);
            viewport.muted |= muted;
            let _ = paint(app, &session, viewport);
        }
    }
    pub fn refresh_account(&self, app: &AppHandle) {
        let sessions = lock(&self.registry)
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            if session.closed.load(Ordering::Acquire) || session.opening.load(Ordering::Acquire) {
                continue;
            }
            session.epoch.fetch_add(1, Ordering::AcqRel);
            invalidate_input_intents(&session);
            let _ = fullscreen(app, &session, false);
            {
                let mut status = lock(&session.status);
                status.state = "loading".into();
                status.chat_state = "pending".into();
                status.error = None;
            }
            if !app
                .get_webview(&session.label)
                .is_some_and(|view| view.reload().is_ok())
            {
                lock(&session.status).error = Some("계정 변경 후 영상을 다시 열어 주세요.".into());
            }
        }
    }
    fn create_view(
        &self,
        app: &AppHandle,
        session: &Arc<Session>,
        tray: Arc<AtomicBool>,
    ) -> Result<(), String> {
        if session.closed.load(Ordering::Acquire) {
            return Err(STALE.into());
        }
        let url = policy::player_url(&session.video)?;
        let parent = app.get_window("main").ok_or(UNAVAILABLE)?;
        let navigation = session.clone();
        let page = session.clone();
        let page_app = app.clone();
        let popup_app = app.clone();
        let nonce = uuid::Uuid::new_v4().to_string();
        #[cfg(not(debug_assertions))]
        let page_script = include_str!("original_vod_page.js");
        #[cfg(debug_assertions)]
        let page_script = click_probe::initialization(&app.config().identifier, &session.video);
        let builder=WebviewBuilder::new(&session.label,WebviewUrl::External("about:blank".parse().unwrap()))
            .data_directory(profile(app)?).focused(false).browser_extensions_enabled(false)
            .initialization_script(platform_native_geometry::pointer_script(url.as_str(),&nonce))
            .initialization_script(page_script)
            .additional_browser_args("--autoplay-policy=no-user-gesture-required --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows")
            .on_navigation(move|next|next.as_str()=="about:blank"&&navigation.epoch.load(Ordering::Acquire)==0||policy::matches_document(next,&navigation.video))
            .on_new_window(move|url,features|super::open_popup(&popup_app,url,features,0)).on_download(|_,_|false)
            .on_page_load(move|view,payload|{
                if !policy::matches_document(payload.url(),&page.video)||page.closed.load(Ordering::Acquire){return;}
                if matches!(payload.event(),tauri::webview::PageLoadEvent::Started){invalidate_input_intents(&page);let _=fullscreen(&page_app,&page,false);page.epoch.fetch_add(1,Ordering::AcqRel);let mut status=lock(&page.status);status.state="loading".into();status.chat_state="pending".into();status.error=None;}
                else{configure(&view,&page);let mut viewport=lock(&page.viewport).clone().unwrap_or_else(hidden_viewport);viewport.muted|=tray.load(Ordering::Acquire);let _=paint(&page_app,&page,viewport);}
            });
        let view = parent
            .add_child(
                builder,
                tauri::LogicalPosition::new(0., 0.),
                tauri::LogicalSize::new(1., 1.),
            )
            .map_err(|_| UNAVAILABLE)?;
        if session.closed.load(Ordering::Acquire) {
            return Err(STALE.into());
        }
        view.hide().map_err(|_| UNAVAILABLE)?;
        platform_native_geometry::paint(
            &view,
            &hidden_viewport(),
            session.paint.clone(),
            session.paint.fetch_add(1, Ordering::AcqRel) + 1,
        )?;
        platform_native_geometry::install_pointer_focus(&view, url.to_string(), nonce)
            .map_err(|_| UNAVAILABLE)?;
        attach_bridge(&view, session.clone())?;
        view.navigate(url).map_err(|_| UNAVAILABLE)?;
        Ok(())
    }
}
fn snapshot_locked(registry: &Registry, token: &str) -> Result<Snapshot, String> {
    let session = registry
        .tokens
        .get(token)
        .and_then(|id| registry.sessions.get(id))
        .filter(|s| !s.closed.load(Ordering::Acquire))
        .ok_or(STALE)?;
    let mut value = lock(&session.status).clone();
    value.token = token.into();
    Ok(value)
}
fn release_viewer(registry: &mut Registry, token: &str) -> Option<Arc<Session>> {
    let id = registry.tokens.remove(token)?;
    registry.requests.retain(|_, (_, bound)| bound != token);
    if let Some(session) = registry.sessions.get(&id) {
        {
            let mut viewers = lock(&session.viewers);
            invalidate_input_intents(session);
            viewers.retain(|viewer| viewer != token);
        }
        lock(&session.sequences).remove(token);
    }
    if registry.tokens.values().any(|value| value == &id) {
        None
    } else {
        let session = registry.sessions.get(&id).cloned()?;
        // Retire under the same registry gate used by open's document reuse.
        session.closed.store(true, Ordering::Release);
        Some(session)
    }
}
fn is_presenter(viewers: &[String], token: &str) -> bool {
    viewers.last().is_some_and(|current| current == token)
}
fn invalidate_input_intents(session: &Session) -> u64 {
    session.escape_intent.clear();
    session.mouse_intent.clear()
}
fn escape_binding(session: &Session) -> Option<escape_intent::Binding> {
    let generation=session.escape_intent.generation();
    let epoch=session.epoch.load(Ordering::Acquire);
    if epoch==0 || session.closed.load(Ordering::Acquire) { return None; }
    let viewers=session.viewers.try_lock().ok()?;
    let presenter=viewers.last()?.clone();
    let grant=session.fullscreen_grant.try_lock().ok()?.clone()?;
    if grant.owner!=presenter || grant.document_epoch!=epoch { return None; }
    let viewport=session.viewport.try_lock().ok()?;
    let viewport=viewport.as_ref()?;
    if !viewport.visible || viewport.validate().is_err() || !viewport.player.as_ref().is_some_and(|p|p.width>0.&&p.height>0.)
        || generation!=session.escape_intent.generation() || epoch!=session.epoch.load(Ordering::Acquire)
        || session.closed.load(Ordering::Acquire) { return None; }
    Some(escape_intent::Binding{epoch,presenter,generation,grant})
}
fn configure(view: &Webview, session: &Session) {
    let _=view.eval(&format!("window.__atsumiSoopVod?.configure({})",json!({"epoch":session.epoch.load(Ordering::Acquire),"videoId":session.video,"fullscreen":lock(&session.status).fullscreen})));
}
fn paint(app: &AppHandle, session: &Arc<Session>, viewport: NativeViewport) -> Result<(), String> {
    if session.closed.load(Ordering::Acquire) {
        return Err(STALE.into());
    }
    // A replacement viewer can submit geometry while its shared native document is still creating.
    let Some(view) = app.get_webview(&session.label) else {
        return Ok(());
    };
    configure(&view, session);
    view.eval(&format!(
        "window.__atsumiSoopVod?.layout({})",
        json!(viewport)
    ))
    .map_err(|_| UNAVAILABLE)?;
    let revision = session.paint.fetch_add(1, Ordering::AcqRel) + 1;
    platform_native_geometry::paint(&view, &viewport, session.paint.clone(), revision)
}
fn fullscreen(app: &AppHandle, session: &Arc<Session>, enabled: bool) -> Result<(), String> {
    fullscreen_for(app, session, enabled, None)
}
fn fullscreen_for(
    app: &AppHandle,
    session: &Arc<Session>,
    enabled: bool,
    presenter: Option<String>,
) -> Result<(), String> {
    if !enabled {
        let grant = {
            let viewers = lock(&session.viewers);
            if presenter
                .as_ref()
                .is_some_and(|token| !is_presenter(&viewers, token))
            {
                return Ok(());
            }
            let grant = lock(&session.fullscreen_grant).clone();
            grant.filter(|grant| presenter.as_ref().is_none_or(|token| grant.owner == *token))
        };
        if let Some(grant) = grant {
            super::window_fullscreen::release(app, grant)?;
        }
        return Ok(());
    }
    let presenter = presenter
        .or_else(|| lock(&session.viewers).last().cloned())
        .ok_or(STALE)?;
    let owner = presenter.clone();
    let epoch = session.epoch.load(Ordering::Acquire);
    let valid = Arc::downgrade(session);
    let acquired = Arc::downgrade(session);
    let lost = Arc::downgrade(session);
    let acquired_app = app.clone();
    let lost_app = app.clone();
    super::window_fullscreen::acquire(
        app,
        owner,
        epoch,
        Arc::new(move || {
            valid.upgrade().is_some_and(|session| {
                !session.closed.load(Ordering::Acquire)
                    && session.epoch.load(Ordering::Acquire) == epoch
                    && is_presenter(&lock(&session.viewers), &presenter)
                    && lock(&session.viewport)
                        .as_ref()
                        .is_some_and(|v| v.visible && v.player.is_some())
            })
        }),
        Arc::new(move |grant| {
            if let Some(session) = acquired.upgrade() {
                let _ = publish_fullscreen(&acquired_app, &session, grant, true);
            }
        }),
        Arc::new(move |grant| {
            if let Some(session) = lost.upgrade() {
                let _ = publish_fullscreen(&lost_app, &session, grant, false);
            }
        }),
    )
    .map_err(|error| {
        if let Some(grant) = error.retained_grant {
            let _ = super::window_fullscreen::release(app, grant);
        }
        error.message
    })?;
    Ok(())
}
fn publish_fullscreen(app: &AppHandle, session: &Session, grant: super::Grant, enabled: bool) -> Option<u64> {
    let generation = {
        let mut current = lock(&session.fullscreen_grant);
        if !enabled && current.as_ref() != Some(&grant) { return None; }
        let generation = invalidate_input_intents(session);
        if enabled {
            *current = Some(grant);
        } else {
            *current = None;
        }
        generation
    };
    lock(&session.status).fullscreen = enabled;
    if let Some(view) = app.get_webview(&session.label) {
        let _ = view.eval(&format!("window.__atsumiSoopVod?.fullscreen({enabled})"));
    }
    let _ = app.emit_to(
        "main",
        "soop-vod:fullscreen",
        json!({"videoId":session.video,"enabled":enabled}),
    );
    Some(generation)
}
/// This path cannot select a newer presenter or release a replacement grant.
/// Pre-dispatch uses the consumed binding; post-acquire recognizes only the
/// grant published by this exact request and its own invalidation generation.
#[cfg(windows)]
fn fullscreen_from_mouse(app: &AppHandle, session: &Arc<Session>, binding: input_intent::Binding) -> Result<bool, String> {
    if input_intent::current(session).as_ref() != Some(&binding) { return Err(STALE.into()); }
    if let Some(grant) = binding.grant.clone() { return super::window_fullscreen::release(app, grant); }
    let accepted = Arc::new(Mutex::new(None::<(super::Grant,u64)>));
    let valid = Arc::downgrade(session);
    let acquired = Arc::downgrade(session);
    let lost = Arc::downgrade(session);
    let valid_accepted = accepted.clone();
    let valid_binding = binding.clone();
    let acquired_app = app.clone();
    let lost_app = app.clone();
    super::window_fullscreen::acquire(app, binding.presenter.clone(), binding.epoch,
        Arc::new(move || {
            let Some(current) = valid.upgrade().and_then(|session| input_intent::current(&session)) else { return false; };
            let Ok(accepted) = valid_accepted.try_lock() else { return false; };
            accepted.as_ref().map_or(current == valid_binding, |grant| valid_binding.after_acquire(&current,grant))
        }),
        Arc::new(move |grant| {
            if let Some(session) = acquired.upgrade() {
                if let Some(generation) = publish_fullscreen(&acquired_app,&session,grant.clone(),true) {
                    *lock(&accepted) = Some((grant,generation));
                }
            }
        }),
        Arc::new(move |grant| { if let Some(session) = lost.upgrade() { let _ = publish_fullscreen(&lost_app,&session,grant,false); } }),
    ).map_err(|error| {
        if let Some(grant) = error.retained_grant { let _ = super::window_fullscreen::release(app,grant); }
        error.message
    })?;
    Ok(true)
}
fn accepts(source: &str, label: &str, session: &Session, message: &Envelope) -> bool {
    !session.closed.load(Ordering::Acquire)
        && label == session.label
        && uuid::Uuid::parse_str(&message.id).is_ok()
        && message.epoch > 0
        && message.epoch == session.epoch.load(Ordering::Acquire)
        && matches!(message.kind.as_str(), "status" | "fullscreen")
        && tauri::Url::parse(source)
            .ok()
            .is_some_and(|url| policy::matches_document(&url, &session.video))
}
fn update_status(session: &Session, message: &Envelope) {
    if message.epoch != session.epoch.load(Ordering::Acquire)
        || session.closed.load(Ordering::Acquire)
    {
        return;
    }
    let mut status = lock(&session.status);
    if let Some(state) = message.fields["state"]
        .as_str()
        .filter(|v| matches!(*v, "loading" | "playing" | "paused" | "ended" | "error"))
    {
        status.state = state.into();
    }
    if let Some(chat) = message.fields["chatState"]
        .as_str()
        .filter(|v| matches!(*v, "pending" | "available" | "unavailable"))
    {
        status.chat_state = chat.into();
    }
    status.error = if message.fields["unavailable"].as_bool() == Some(true) {
        Some("SOOP에서 이 영상을 재생할 수 없습니다. 원본 플레이어의 안내를 확인해 주세요.".into())
    } else {
        None
    };
}
#[cfg(windows)]
fn attach_bridge(view: &Webview, session: Arc<Session>) -> Result<(), String> {
    use webview2_com::{CoTaskMemPWSTR, WebMessageReceivedEventHandler};
    use windows::core::PWSTR;
    let app = view.app_handle().clone();
    let native = view.clone();
    let root = app
        .get_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as usize)
        .ok_or(UNAVAILABLE)?;
    view.with_webview(move |platform| unsafe {
        if let Ok(core) = platform.controller().CoreWebView2() {
            let mut pane = windows::Win32::Foundation::HWND::default();
            if platform.controller().ParentWindow(&mut pane).is_err() {
                return;
            }
            let pane = pane.0 as usize;
            if input_intent::install(&platform.controller(), root, pane, &session).is_err() {
                // No receiver destruction or permissive fallback on hook failure.
                tracing::warn!(diag_stage = "soop_vod_mouse_intent_unavailable");
            }
            let current_escape=Arc::downgrade(&session);
            if escape_intent::install(&platform.controller(),root,pane,session.escape_intent.clone(),
                Arc::new(move || current_escape.upgrade().and_then(|session|escape_binding(&session)))).is_err() {
                tracing::warn!(diag_stage="soop_vod_escape_intent_unavailable");
            }
            let mut event_token = 0;
            let input_diagnostics = input_diagnostics::Budget::default();
            #[cfg(debug_assertions)]
            let click_probe = click_probe::Budget::new(click_probe::enabled(&app.config().identifier, &session.video));
            let _ = core.add_WebMessageReceived(
                &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else {
                        return Ok(());
                    };
                    let mut source = PWSTR::null();
                    args.Source(&mut source)?;
                    let source = CoTaskMemPWSTR::from(source).to_string();
                    let mut body = PWSTR::null();
                    if args.TryGetWebMessageAsString(&mut body).is_err() {
                        return Ok(());
                    }
                    let body = CoTaskMemPWSTR::from(body).to_string();
                    #[cfg(debug_assertions)]
                    if click_probe.enabled() && click_probe.observe_dom(&body,
                        !session.closed.load(Ordering::Acquire) && native.label() == session.label
                        && tauri::Url::parse(&source).ok().is_some_and(|url|policy::matches_document(&url,&session.video))) {
                        return Ok(());
                    }
                    let Some(raw) = body.strip_prefix(PREFIX).filter(|raw| raw.len() <= 4096)
                    else {
                        return Ok(());
                    };
                    let Ok(message) = serde_json::from_str::<Envelope>(raw) else {
                        return Ok(());
                    };
                    let accepted = accepts(&source, native.label(), &session, &message);
                    #[cfg(debug_assertions)]
                    if click_probe.enabled() && message.kind == "fullscreen" && message.fields["inputKey"].as_i64() == Some(1) {
                        click_probe.observe_bridge(native.label() == session.label
                            && tauri::Url::parse(&source).ok().is_some_and(|url|policy::matches_document(&url,&session.video)),
                            message.epoch > 0 && message.epoch == session.epoch.load(Ordering::Acquire), accepted);
                    }
                    if !accepted {
                        return Ok(());
                    }
                    if message.kind == "status" {
                        update_status(&session, &message);
                    } else {
                        let key = message.fields["inputKey"].as_i64().unwrap_or(0) as i32;
                        if key == 1 {
                            let (gate,binding,reason) = input_intent::consume(&session,root,pane,message.fields["enabled"].as_bool());
                            let observation = input_diagnostics.observe_receipt(key,gate,gate.allowed(),Some(reason));
                            if let Some(binding) = binding {
                                let outcome = fullscreen_from_mouse(&app,&session,binding);
                                if let Some(observation) = observation { observation.finish(matches!(outcome, Ok(true))); }
                            }
                        } else if key==0x1B {
                            if message.fields["enabled"].as_bool()!=Some(false) {
                                session.escape_intent.clear();
                                let _=input_diagnostics.observe(key,platform_native_geometry::InputGate::InvalidInput);
                            } else {
                                let gate=platform_native_geometry::trusted_input_gate(root,pane,key);
                                let (gate,grant,receipt_used)=session.escape_intent.exit(escape_binding(&session),gate,
                                    ||platform_native_geometry::trusted_escape_receipt_context(root,pane),std::time::Instant::now());
                                let observation=input_diagnostics.observe_receipt(key,gate,receipt_used,None);
                                if let Some(grant)=grant {
                                    let outcome=super::window_fullscreen::release(&app,grant);
                                    if let Some(observation)=observation {observation.finish(matches!(outcome,Ok(true)));}
                                }
                            }
                        } else if key==0x46 {
                            let gate = platform_native_geometry::trusted_input_gate(root, pane, key);
                            let observation = input_diagnostics.observe(key, gate);
                            if gate.allowed() {
                                if let Some(enabled) = message.fields["enabled"].as_bool() {
                                    let outcome = fullscreen(&app, &session, enabled);
                                    if let Some(observation) = observation {
                                        observation.finish(outcome.is_ok());
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                })),
                &mut event_token,
            );
        }
    })
    .map_err(|_| UNAVAILABLE)?;
    Ok(())
}
#[cfg(not(windows))]
fn attach_bridge(_: &Webview, _: Arc<Session>) -> Result<(), String> {
    Err("SOOP 원본 플레이어는 Windows에서 사용할 수 있습니다.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        Session {
            id: "session".into(),
            label: "soop-vod-test".into(),
            video: "209244227".into(),
            document: Documents::default().acquire().unwrap(),
            closed: AtomicBool::new(false),
            opening: AtomicBool::new(false),
            epoch: AtomicU64::new(2),
            paint: Arc::new(AtomicU64::new(0)),
            status: Mutex::new(Snapshot {
                token: "old".into(),
                video_id: "209244227".into(),
                state: "loading".into(),
                chat_state: "pending".into(),
                fullscreen: false,
                error: None,
            }),
            fullscreen_grant: Mutex::new(None),
            viewport: Mutex::new(None),
            sequences: Mutex::new(HashMap::new()),
            viewers: Mutex::new(Vec::new()),
            mouse_intent: Arc::new(input_intent::Intent::default()),
            escape_intent: Arc::new(escape_intent::Intent::default()),
        }
    }
    fn message(kind: &str) -> Envelope {
        serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"epoch":2,"type":kind,"state":"ended","chatState":"available","authenticated":true,"captureReady":true,"recordingJob":"forged"})).unwrap()
    }
    #[test]
    fn watch_only_bridge_has_no_capture_or_account_authority() {
        let s = session();
        let url = policy::player_url(&s.video).unwrap();
        assert!(accepts(url.as_str(), &s.label, &s, &message("status")));
        for kind in [
            "encoded_begin",
            "encoded_append",
            "encoded_finish",
            "chat_status",
            "screenshot",
            "account",
            "record",
        ] {
            assert!(!accepts(url.as_str(), &s.label, &s, &message(kind)));
        }
        assert!(!accepts(url.as_str(), "wrong", &s, &message("status")));
        let mut m = message("status");
        m.epoch = 1;
        assert!(!accepts(url.as_str(), &s.label, &s, &m));
    }
    #[test]
    fn end_retains_document_and_later_play_state() {
        let s = session();
        update_status(&s, &message("status"));
        assert_eq!(lock(&s.status).state, "ended");
        assert!(!s.closed.load(Ordering::Acquire));
        let mut m = message("status");
        m.fields["state"] = json!("playing");
        m.fields["chatState"] = json!("unsupported-guess");
        update_status(&s, &m);
        assert_eq!(lock(&s.status).state, "playing");
        assert_eq!(lock(&s.status).chat_state, "available");
    }
    #[test]
    fn delayed_close_cannot_release_replacement_document() {
        let s = Arc::new(session());
        let mut r = Registry::default();
        r.sessions.insert(s.id.clone(), s.clone());
        for token in ["old", "replacement"] {
            r.tokens.insert(token.into(), s.id.clone());
            r.requests
                .insert(format!("request-{token}"), (s.video.clone(), token.into()));
            lock(&s.viewers).push(token.into());
        }
        assert!(release_viewer(&mut r, "old").is_none());
        assert!(r.tokens.contains_key("replacement"));
        assert!(release_viewer(&mut r, "old").is_none());
        assert_eq!(release_viewer(&mut r, "replacement").unwrap().id, s.id);
        assert!(s.closed.load(Ordering::Acquire));
    }
    #[test]
    fn last_close_retires_before_reopen_and_old_viewer_cannot_control_presentation() {
        let s = Arc::new(session());
        let mut r = Registry::default();
        r.sessions.insert(s.id.clone(), s.clone());
        r.tokens.insert("old".into(), s.id.clone());
        lock(&s.viewers).push("old".into());
        let retiring = release_viewer(&mut r, "old").unwrap();
        assert!(retiring.closed.load(Ordering::Acquire));
        assert!(!r.sessions.values().any(
            |candidate| candidate.video == s.video && !candidate.closed.load(Ordering::Acquire)
        ));
        assert!(!is_presenter(&["old".into(), "replacement".into()], "old"));
        assert!(is_presenter(
            &["old".into(), "replacement".into()],
            "replacement"
        ));
    }
    #[test]
    fn closing_an_opening_viewer_keeps_document_capacity_until_creation_is_cleaned_up() {
        let budget = Documents::default();
        let mut s = session();
        s.document = budget.acquire().unwrap();
        s.opening.store(true, Ordering::Release);
        let s = Arc::new(s);
        let mut r = Registry::default();
        r.sessions.insert(s.id.clone(), s.clone());
        r.tokens.insert("old".into(), s.id.clone());
        lock(&s.viewers).push("old".into());
        let retiring = release_viewer(&mut r, "old").unwrap();
        assert!(retiring.closed.load(Ordering::Acquire));
        assert!(retiring.opening.load(Ordering::Acquire));
        let _other = (0..3)
            .map(|_| budget.acquire().unwrap())
            .collect::<Vec<_>>();
        assert!(budget.acquire().is_err());
        retiring.opening.store(false, Ordering::Release);
        retiring.document.release();
        assert!(budget.acquire().is_ok());
    }
    #[test]
    fn mouse_snapshot_requires_current_presenter_visible_source_and_nonblocking_locks() {
        let s = session();
        assert!(input_intent::current(&s).is_none());
        lock(&s.viewers).push("viewer".into());
        *lock(&s.viewport) = Some(NativeViewport { sequence:1, visible:true, muted:false,
            player:Some(platform_native_geometry::Rect{x:10.,y:10.,width:100.,height:100.}),chat:None,occlusions:vec![] });
        assert!(input_intent::current(&s).is_some());
        let held=lock(&s.viewers); assert!(input_intent::current(&s).is_none()); drop(held);
        lock(&s.viewport).as_mut().unwrap().visible=false;
        assert!(input_intent::current(&s).is_none());
        lock(&s.viewport).as_mut().unwrap().visible=true;
        *lock(&s.fullscreen_grant)=Some(super::super::Grant{owner:"foreign".into(),document_epoch:2,revision:1,window_key:"main".into()});
        assert!(input_intent::current(&s).is_none());
        *lock(&s.fullscreen_grant)=None;
        s.closed.store(true,Ordering::Release); assert!(input_intent::current(&s).is_none());
    }
    #[test]
    fn escape_snapshot_requires_current_grant_and_preserves_source_generation() {
        let s=session();lock(&s.viewers).push("viewer".into());
        *lock(&s.viewport)=Some(NativeViewport{sequence:1,visible:true,muted:false,player:Some(platform_native_geometry::Rect{x:10.,y:10.,width:100.,height:100.}),chat:None,occlusions:vec![]});
        assert!(escape_binding(&s).is_none());
        *lock(&s.fullscreen_grant)=Some(super::super::Grant{owner:"viewer".into(),document_epoch:2,revision:1,window_key:"main".into()});
        let initial=escape_binding(&s).unwrap();assert_eq!(initial.presenter,"viewer");
        let guard=lock(&s.viewers);assert!(escape_binding(&s).is_none());drop(guard);
        invalidate_input_intents(&s);assert_ne!(initial.generation,escape_binding(&s).unwrap().generation);
        lock(&s.viewport).as_mut().unwrap().visible=false;assert!(escape_binding(&s).is_none());
        lock(&s.viewport).as_mut().unwrap().visible=true;lock(&s.viewers).push("replacement".into());assert!(escape_binding(&s).is_none());
    }
}
