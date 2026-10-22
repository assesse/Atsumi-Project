//! Watch-only sessions share YouTube's original account profile, not live capture authority.
use super::{
    lock, policy, profile, Envelope, OriginalSnapshot, YouTubeOriginalHost, STALE, UNAVAILABLE,
};
use crate::platform_native_geometry::{self, NativeViewport};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager, Webview, WebviewBuilder, WebviewUrl};

const PREFIX: &str = "ATSUMI_YOUTUBE_VOD_V1:";
const TOKEN_PREFIX: &str = "youtube-vod-";
const SITE_ERROR: &str =
    "YouTube에서 이 영상을 재생할 수 없습니다. 공식 플레이어의 안내를 확인해 주세요.";
const IDENTITY_ERROR: &str = "다른 영상으로의 자동 전환을 차단했습니다. 선택한 영상을 다시 열어 주세요.";
pub(super) fn is_token(token: &str) -> bool {
    token.starts_with(TOKEN_PREFIX)
}
#[derive(Default)]
struct Registry {
    sessions: HashMap<String, Arc<Session>>,
    tokens: HashMap<String, String>,
    requests: HashMap<String, (String, String)>,
}
struct Session {
    token: String,
    label: String,
    video: String,
    closed: AtomicBool,
    identity_blocked: AtomicBool,
    epoch: AtomicU64,
    account_revision: AtomicU64,
    paint: Arc<AtomicU64>,
    status: Mutex<OriginalSnapshot>,
    viewport: Mutex<Option<NativeViewport>>,
    sequences: Mutex<HashMap<String, u64>>,
    fullscreen_grant: Mutex<Option<super::Grant>>,
}
#[derive(Clone, Default)]
pub(super) struct WatchHost {
    registry: Arc<Mutex<Registry>>,
}
impl WatchHost {
    pub fn count(&self) -> usize {
        lock(&self.registry).sessions.len()
    }
    pub fn contains_video(&self, video: &str) -> bool {
        lock(&self.registry)
            .sessions
            .values()
            .any(|s| s.video == video)
    }
    pub fn existing(&self, request: &str, video: &str) -> Result<Option<OriginalSnapshot>, String> {
        let registry = lock(&self.registry);
        let Some((bound, token)) = registry.requests.get(request) else {
            return Ok(None);
        };
        if bound != video {
            return Err(STALE.into());
        }
        let session = registry
            .tokens
            .get(token)
            .and_then(|id| registry.sessions.get(id))
            .ok_or(STALE)?;
        let mut value = lock(&session.status).clone();
        value.token = token.clone();
        Ok(Some(value))
    }
    /// Parent holds its provider admission mutex. There is no media lease or download folder.
    pub fn open(
        &self,
        app: &AppHandle,
        host: YouTubeOriginalHost,
        video: String,
        request: String,
        can_create_document: bool,
    ) -> Result<OriginalSnapshot, String> {
        if !policy::valid_channel(&video) || uuid::Uuid::parse_str(&request).is_err() {
            return Err(STALE.into());
        }
        if let Some(value) = self.existing(&request, &video)? {
            return Ok(value);
        }
        let token = format!("{TOKEN_PREFIX}{}", uuid::Uuid::new_v4());
        // A replacement renderer may open before its old asynchronous close arrives.
        // Give it its own viewer token while retaining the exact same official document.
        {
            let mut registry = lock(&self.registry);
            if let Some(session) = reuse_or_admit(&registry, &video, can_create_document)? {
                registry.tokens.insert(token.clone(), session.token.clone());
                registry.requests.insert(request, (video, token.clone()));
                let mut value = lock(&session.status).clone();
                value.token = token;
                return Ok(value);
            }
        }
        let session = Arc::new(Session {
            label: format!("youtube-original-vod-{}", uuid::Uuid::new_v4().simple()),
            token: token.clone(),
            video: video.clone(),
            closed: AtomicBool::new(false),
            identity_blocked: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            account_revision: AtomicU64::new(lock(&host.authenticated).revision),
            paint: Arc::new(AtomicU64::new(0)),
            status: Mutex::new(OriginalSnapshot {
                token: token.clone(),
                channel_id: video.clone(),
                broadcast_id: video.clone(),
                canonical_channel_id: None,
                state: "loading".into(),
                authenticated: None,
                chat_visible: false,
                capture_ready: false,
                capture_unavailable_reason: Some(
                    "영상 다운로드는 상세보기에서 사용할 수 있습니다.".into(),
                ),
                recording_job: None,
                recording_stopping: false,
                fullscreen: false,
                error: None,
            }),
            viewport: Mutex::new(None),
            sequences: Mutex::new(HashMap::new()),
            fullscreen_grant: Mutex::new(None),
        });
        lock(&self.registry)
            .sessions
            .insert(token.clone(), session.clone());
        if let Err(error) = self.create_view(app, &session, host) {
            self.dispose(app, &session);
            return Err(error);
        }
        {
            let mut registry = lock(&self.registry);
            registry.tokens.insert(token.clone(), token.clone());
            registry.requests.insert(request, (video, token));
        }
        let value = lock(&session.status).clone();
        Ok(value)
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
    pub fn snapshot(&self, token: &str) -> Result<OriginalSnapshot, String> {
        let mut value = lock(&self.session(token)?.status).clone();
        value.token = token.into();
        Ok(value)
    }
    pub fn viewport(
        &self,
        app: &AppHandle,
        token: &str,
        mut viewport: NativeViewport,
        tray_muted: bool,
    ) -> Result<(), String> {
        viewport.validate()?;
        if viewport.chat.is_some() {
            return Err("일반 영상에는 라이브 채팅 영역이 없습니다.".into());
        }
        let session = self.session(token)?;
        let mut sequences = lock(&session.sequences);
        let sequence = sequences.entry(token.into()).or_insert(0);
        if !policy::sequence_newer(*sequence, viewport.sequence) {
            return Ok(());
        }
        *sequence = viewport.sequence;
        *lock(&session.viewport) = Some(viewport.clone());
        viewport.muted |= tray_muted;
        paint(app, &session, viewport)
    }
    pub fn fullscreen(&self, app: &AppHandle, token: &str, enabled: bool) -> Result<(), String> {
        fullscreen(app, &self.session(token)?, enabled)
    }
    /// Parent serializes opens and closes with its provider admission gate.
    pub fn close(&self, app: &AppHandle, token: &str) -> Result<(), String> {
        let session = {
            let mut registry = lock(&self.registry);
            release_viewer(&mut registry, token)
        };
        if let Some(session) = session {
            self.dispose(app, &session);
        }
        Ok(())
    }
    fn dispose(&self, app: &AppHandle, session: &Arc<Session>) {
        if session.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        session.paint.fetch_add(1, Ordering::AcqRel);
        if lock(&session.status).fullscreen {
            let _ = fullscreen(app, session, false);
        }
        if let Some(view) = app.get_webview(&session.label) {
            let _ = view.close();
        }
        let mut registry = lock(&self.registry);
        registry.sessions.remove(&session.token);
        registry.tokens.retain(|_, id| id != &session.token);
        let remaining = registry
            .tokens
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        registry
            .requests
            .retain(|_, (_, token)| remaining.contains(token));
    }
    pub fn detach(&self, app: &AppHandle) {
        let sessions = lock(&self.registry)
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            self.dispose(app, &session);
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
                .unwrap_or_else(super::hidden_viewport);
            viewport.muted |= muted;
            let _ = paint(app, &session, viewport);
        }
    }
    pub fn refresh_account(&self, app: &AppHandle, revision: u64) {
        let sessions = lock(&self.registry)
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            if session.closed.load(Ordering::Acquire) || session.identity_blocked.load(Ordering::Acquire) {
                continue;
            }
            session.account_revision.store(revision, Ordering::Release);
            session.epoch.fetch_add(1, Ordering::AcqRel);
            let _ = fullscreen(app, &session, false);
            {
                let mut status = lock(&session.status);
                if session.identity_blocked.load(Ordering::Acquire) { continue; }
                status.authenticated = None;
                status.state = "loading".into();
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
        host: YouTubeOriginalHost,
    ) -> Result<(), String> {
        let url = policy::player_url(&session.video, &session.video)?;
        let parent = app.get_window("main").ok_or(UNAVAILABLE)?;
        let navigation = session.clone();
        let page = session.clone();
        let page_app = app.clone();
        let page_host = host.clone();
        let nonce = uuid::Uuid::new_v4().to_string();
        let builder = WebviewBuilder::new(&session.label, WebviewUrl::External("about:blank".parse().unwrap()))
            .data_directory(profile(app)?).focused(false).browser_extensions_enabled(false)
            .initialization_script(platform_native_geometry::pointer_script(url.as_str(), &nonce))
            .initialization_script(include_str!("original_vod_page.js"))
            .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
            .on_navigation(move |next| {
                if navigation.identity_blocked.load(Ordering::Acquire) { return next.as_str() == "about:blank"; }
                if next.as_str() == "about:blank" { return navigation.epoch.load(Ordering::Acquire) == 0; }
                if policy::matches_document(next, &navigation.video, false) { return true; }
                if policy::account_url(next) { lock(&navigation.status).error = Some("방송 조작 창에서 YouTube 로그인을 진행해 주세요.".into()); }
                false
            })
            .on_new_window({ let app = app.clone(); move |url, features| super::open_popup(&app, url, features, 0) })
            .on_download(|_, _| false)
            .on_page_load(move |view, payload| {
                if page.identity_blocked.load(Ordering::Acquire) || page.closed.load(Ordering::Acquire) { return; }
                if !policy::matches_document(payload.url(), &page.video, false) { return; }
                if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                    let _=fullscreen(&page_app,&page,false);
                    page.epoch.fetch_add(1, Ordering::AcqRel); page.account_revision.store(lock(&page_host.authenticated).revision, Ordering::Release);
                    let mut status = lock(&page.status); status.state = "loading".into(); status.authenticated = None; status.error = None;
                } else {
                    configure(&view, &page);
                    let mut viewport = lock(&page.viewport).clone().unwrap_or_else(super::hidden_viewport); viewport.muted |= page_host.tray_muted.load(Ordering::Acquire);
                    let _ = paint(&page_app, &page, viewport);
                }
            });
        let view = parent
            .add_child(
                builder,
                tauri::LogicalPosition::new(0., 0.),
                tauri::LogicalSize::new(1., 1.),
            )
            .map_err(|_| UNAVAILABLE)?;
        view.hide().map_err(|_| UNAVAILABLE)?;
        platform_native_geometry::paint(
            &view,
            &super::hidden_viewport(),
            session.paint.clone(),
            session.paint.fetch_add(1, Ordering::AcqRel) + 1,
        )?;
        platform_native_geometry::install_pointer_focus(&view, url.to_string(), nonce)
            .map_err(|_| UNAVAILABLE)?;
        attach_bridge(&view, session.clone(), host)?;
        view.navigate(url).map_err(|_| UNAVAILABLE)?;
        Ok(())
    }
}
fn release_viewer(registry: &mut Registry, token: &str) -> Option<Arc<Session>> {
    let id = registry.tokens.remove(token)?;
    registry.requests.retain(|_, (_, bound)| bound != token);
    if let Some(session) = registry.sessions.get(&id) {
        lock(&session.sequences).remove(token);
    }
    if registry.tokens.values().any(|value| value == &id) {
        None
    } else {
        registry.sessions.get(&id).cloned()
    }
}
fn reusable(session: &Session) -> bool {
    !session.closed.load(Ordering::Acquire) && !session.identity_blocked.load(Ordering::Acquire)
}
fn reuse_or_admit(registry: &Registry, video: &str, can_create_document: bool) -> Result<Option<Arc<Session>>, String> {
    if let Some(session) = registry.sessions.values().find(|session| session.video == video && reusable(session)) {
        return Ok(Some(session.clone()));
    }
    if !can_create_document { return Err("YouTube 영상은 최대 4개까지 열 수 있습니다.".into()); }
    Ok(None)
}
fn configure(view: &Webview, session: &Session) {
    let _ = view.eval(&format!("window.__atsumiYouTubeVod?.configure({})", json!({"epoch":session.epoch.load(Ordering::Acquire),"videoId":session.video,"fullscreen":lock(&session.status).fullscreen})));
}
fn paint(app: &AppHandle, session: &Arc<Session>, viewport: NativeViewport) -> Result<(), String> {
    if session.closed.load(Ordering::Acquire) {
        return Err(STALE.into());
    }
    if session.identity_blocked.load(Ordering::Acquire) { return Ok(()); }
    let view = app.get_webview(&session.label).ok_or(UNAVAILABLE)?;
    configure(&view, session);
    view.eval(&format!(
        "window.__atsumiYouTubeVod?.layout({})",
        json!(viewport)
    ))
    .map_err(|_| UNAVAILABLE)?;
    let revision = session.paint.fetch_add(1, Ordering::AcqRel) + 1;
    // Check on both sides of reserving the paint revision. A block before this
    // reservation stops enqueue; a block after it invalidates the queued paint.
    if session.identity_blocked.load(Ordering::Acquire) { return Ok(()); }
    platform_native_geometry::paint(&view, &viewport, session.paint.clone(), revision)
}
fn fullscreen(app: &AppHandle, session: &Arc<Session>, enabled: bool) -> Result<(), String> {
    if !enabled {
        let grant = lock(&session.fullscreen_grant).clone();
        if let Some(grant) = grant {
            super::window_fullscreen::release(app, grant)?;
        }
        return Ok(());
    }
    if session.closed.load(Ordering::Acquire) || session.identity_blocked.load(Ordering::Acquire) { return Err(STALE.into()); }
    let epoch = session.epoch.load(Ordering::Acquire);
    let valid = Arc::downgrade(session);
    let acquired = Arc::downgrade(session);
    let lost = Arc::downgrade(session);
    let acquired_app = app.clone();
    let lost_app = app.clone();
    super::window_fullscreen::acquire(
        app,
        session.token.clone(),
        epoch,
        Arc::new(move || {
            valid.upgrade().is_some_and(|session| {
                !session.closed.load(Ordering::Acquire)
                    && !session.identity_blocked.load(Ordering::Acquire)
                    && session.epoch.load(Ordering::Acquire) == epoch
                    && lock(&session.viewport)
                        .as_ref()
                        .is_some_and(|v| v.visible && v.player.is_some())
            })
        }),
        Arc::new(move |grant| {
            if let Some(session) = acquired.upgrade() {
                publish_fullscreen(&acquired_app, &session, grant, true);
            }
        }),
        Arc::new(move |grant| {
            if let Some(session) = lost.upgrade() {
                publish_fullscreen(&lost_app, &session, grant, false);
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
fn publish_fullscreen(app: &AppHandle, session: &Session, grant: super::Grant, enabled: bool) {
    {
        let mut current = lock(&session.fullscreen_grant);
        if enabled {
            *current = Some(grant);
        } else if current.as_ref() == Some(&grant) {
            *current = None;
        } else {
            return;
        }
    }
    lock(&session.status).fullscreen = enabled;
    if let Some(view) = app.get_webview(&session.label) {
        let _ = view.eval(&format!("window.__atsumiYouTubeVod?.fullscreen({enabled})"));
    }
    let _ = app.emit(
        "youtube-original:fullscreen",
        json!({"channelId":session.video,"enabled":enabled}),
    );
}
fn escape_binding(session:&Session)->Option<super::escape_intent::Binding> {
    if session.closed.load(Ordering::Acquire)||session.identity_blocked.load(Ordering::Acquire){return None;}
    if !lock(&session.viewport).as_ref().is_some_and(|v|v.visible&&v.player.is_some()){return None;}
    if !lock(&session.status).fullscreen{return None;}
    let grant=lock(&session.fullscreen_grant).clone()?;
    let epoch=session.epoch.load(Ordering::Acquire);
    (epoch>0&&grant.document_epoch==epoch).then_some(super::escape_intent::Binding{epoch,grant})
}
fn accepts(source: &str, label: &str, session: &Session, message: &Envelope) -> bool {
    !session.closed.load(Ordering::Acquire)
        && !session.identity_blocked.load(Ordering::Acquire)
        && label == session.label
        && uuid::Uuid::parse_str(&message.id).is_ok()
        && message.epoch > 0
        && message.epoch == session.epoch.load(Ordering::Acquire)
        && matches!(message.kind.as_str(), "status" | "fullscreen")
        && tauri::Url::parse(source)
            .ok()
            .is_some_and(|url| policy::matches_document(&url, &session.video, false))
}
fn update_status(session: &Session, message: &Envelope) {
    if message.epoch != session.epoch.load(Ordering::Acquire)
        || session.closed.load(Ordering::Acquire)
    {
        return;
    }
    let mut status = lock(&session.status);
    if session.identity_blocked.load(Ordering::Acquire) { return; }
    if let Some(state) = message.fields["state"]
        .as_str()
        .filter(|v| matches!(*v, "loading" | "playing" | "ended" | "error"))
    {
        status.state = state.into();
    }
    status.authenticated = message.fields["authenticated"].as_bool();
    if message.fields["unavailable"].as_bool() == Some(true) {
        status.error = Some(SITE_ERROR.into());
    } else if status.error.as_deref() == Some(SITE_ERROR) {
        status.error = None;
    }
}
/// A committed SPA source change bypasses NavigationStarting. Keep its original
/// binding and error, never relabel or reuse the receiver for another video.
fn block_changed_source(session: &Session, source: &str) -> bool {
    if session.closed.load(Ordering::Acquire) || session.identity_blocked.load(Ordering::Acquire)
        || source == "about:blank" && session.epoch.load(Ordering::Acquire) == 0
        || tauri::Url::parse(source).ok().is_some_and(|url| policy::matches_document(&url, &session.video, false)) {
        return false;
    }
    if session.identity_blocked.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() { return false; }
    session.paint.fetch_add(1, Ordering::AcqRel);
    session.epoch.fetch_add(1, Ordering::AcqRel);
    let mut status = lock(&session.status);
    status.state = "error".into(); status.error = Some(IDENTITY_ERROR.into());
    true
}
#[cfg(windows)]
fn attach_bridge(
    view: &Webview,
    session: Arc<Session>,
    host: YouTubeOriginalHost,
) -> Result<(), String> {
    use webview2_com::{CoTaskMemPWSTR, SourceChangedEventHandler, WebMessageReceivedEventHandler,
        Microsoft::Web::WebView2::Win32::ICoreWebView2_8};
    use windows::core::{Interface, PWSTR, w};
    let app = view.app_handle().clone();
    let native = view.clone();
    let root = app
        .get_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as usize)
        .ok_or(UNAVAILABLE)?;
    view.with_webview(move |platform| unsafe {
        if let Ok(core) = platform.controller().CoreWebView2() {
            let weak = Arc::downgrade(&session);
            let source_app = app.clone();
            let controller = platform.controller();
            let mut source_token = 0;
            let source_handler = SourceChangedEventHandler::create(Box::new(move |sender, _| {
                let (Some(core), Some(session)) = (sender, weak.upgrade()) else { return Ok(()); };
                let mut source = PWSTR::null();
                let source = if core.Source(&mut source).is_ok() {
                    CoTaskMemPWSTR::from(source).to_string()
                } else { "invalid-source-observation".into() };
                if !block_changed_source(&session, &source) { return Ok(()); }
                // No session lock is held when releasing the exact UI grant.
                let _ = fullscreen(&source_app, &session, false);
                let _ = controller.SetIsVisible(false);
                if let Ok(audio) = core.cast::<ICoreWebView2_8>() { let _ = audio.SetIsMuted(true); }
                let _ = core.Stop();
                if core.Navigate(w!("about:blank")).is_err() { let _ = controller.Close(); }
                Ok(())
            }));
            if core.add_SourceChanged(&source_handler, &mut source_token).is_err() {
                // Without the source boundary, this document cannot safely play.
                block_changed_source(&session, "invalid-source-observer");
                lock(&session.status).error = Some("영상 전환 보호를 시작하지 못했습니다. 다시 열어 주세요.".into());
                let _ = core.Stop(); let _ = platform.controller().Close();
                return;
            }
            let mut pane = windows::Win32::Foundation::HWND::default();
            if platform.controller().ParentWindow(&mut pane).is_err() {
                return;
            }
            let pane = pane.0 as usize;
            let input_diagnostics = super::input_diagnostics::Budget::default();
            let escape_intent=Arc::new(super::escape_intent::Intent::default());
            let weak=Arc::downgrade(&session);
            if super::escape_intent::install(&platform.controller(),root,pane,escape_intent.clone(),Arc::new(move||weak.upgrade().and_then(|session|escape_binding(&session)))).is_err(){
                tracing::warn!(diag_stage="youtube_fullscreen_escape_unavailable");
            }
            let mut event_token = 0;
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
                    let Some(raw) = body.strip_prefix(PREFIX).filter(|raw| raw.len() <= 4096)
                    else {
                        return Ok(());
                    };
                    let Ok(message) = serde_json::from_str::<Envelope>(raw) else {
                        return Ok(());
                    };
                    if !accepts(&source, native.label(), &session, &message) {
                        return Ok(());
                    }
                    if message.kind == "status" {
                        update_status(&session, &message);
                        if let Some(value) = lock(&session.status).authenticated {
                            lock(&host.authenticated)
                                .observe(session.account_revision.load(Ordering::Acquire), value);
                        }
                    } else {
                        let key = message.fields["inputKey"].as_i64().unwrap_or(0) as i32;
                        if !matches!(key, 1 | 0x46 | 0x1B) {
                            return Ok(());
                        }
                        let gate=platform_native_geometry::trusted_input_gate(root,pane,key);
                        if key==0x1B {
                            if message.fields["enabled"].as_bool()!=Some(false){escape_intent.clear();return Ok(());}
                            let (gate,grant,receipt_used)=escape_intent.exit(escape_binding(&session),gate,||platform_native_geometry::trusted_escape_receipt_context(root,pane),std::time::Instant::now());
                            input_diagnostics.record(true,key,gate,receipt_used);
                            if let Some(grant)=grant {let _=super::window_fullscreen::release(&app,grant);}
                            return Ok(());
                        }
                        input_diagnostics.record(true,key,gate,false);
                        if !gate.allowed() {return Ok(());}
                        if let Some(enabled) = message.fields["enabled"].as_bool() {
                            let _ = fullscreen(&app, &session, enabled);
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
fn attach_bridge(_: &Webview, _: Arc<Session>, _: YouTubeOriginalHost) -> Result<(), String> {
    Err("YouTube 원본 플레이어는 Windows에서 사용할 수 있습니다.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        Session {
            token: "youtube-vod-test".into(),
            label: "youtube-original-vod-test".into(),
            video: "M7lc1UVf-VE".into(),
            closed: AtomicBool::new(false),
            identity_blocked: AtomicBool::new(false),
            epoch: AtomicU64::new(2),
            account_revision: AtomicU64::new(0),
            paint: Arc::new(AtomicU64::new(0)),
            status: Mutex::new(OriginalSnapshot {
                token: "youtube-vod-test".into(),
                channel_id: "M7lc1UVf-VE".into(),
                broadcast_id: "M7lc1UVf-VE".into(),
                canonical_channel_id: None,
                state: "loading".into(),
                authenticated: None,
                chat_visible: false,
                capture_ready: false,
                capture_unavailable_reason: None,
                recording_job: None,
                recording_stopping: false,
                fullscreen: false,
                error: None,
            }),
            fullscreen_grant: Mutex::new(None),
            viewport: Mutex::new(None),
            sequences: Mutex::new(HashMap::new()),
        }
    }
    fn message(kind: &str) -> Envelope {
        serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"epoch":2,"type":kind,"state":"ended","authenticated":true,"captureReady":true,"recordingJob":"forged"})).unwrap()
    }
    #[test]
    fn watch_only_bridge_rejects_live_capture_and_foreign_sources() {
        let s = session();
        let url = "https://www.youtube.com/watch?v=M7lc1UVf-VE";
        let mut m = message("status");
        assert!(accepts(url, &s.label, &s, &m));
        for kind in [
            "encoded_begin",
            "encoded_append",
            "encoded_finish",
            "screenshot",
            "chat_status",
        ] {
            assert!(!accepts(url, &s.label, &s, &message(kind)));
        }
        assert!(!accepts(
            "https://www.youtube.com/watch?v=abcdefghijk",
            &s.label,
            &s,
            &m
        ));
        assert!(!accepts(url, "youtube-original-another", &s, &m));
        m.epoch = 1;
        assert!(!accepts(url, &s.label, &s, &m));
    }
    #[test]
    fn ended_keeps_session_replayable_without_recording_authority() {
        let s = session();
        update_status(&s, &message("status"));
        assert_eq!(lock(&s.status).state, "ended");
        assert!(!s.closed.load(Ordering::Acquire));
        assert!(!lock(&s.status).capture_ready);
        assert!(lock(&s.status).recording_job.is_none());
        let mut m = message("status");
        m.fields["state"] = json!("playing");
        update_status(&s, &m);
        assert_eq!(lock(&s.status).state, "playing");
        m.fields["unavailable"] = json!(true);
        update_status(&s, &m);
        assert!(lock(&s.status).error.is_some());
        m.fields["unavailable"] = json!(false);
        update_status(&s, &m);
        assert!(lock(&s.status).error.is_none());
    }
    #[test]
    fn late_viewer_close_cannot_dispose_replacement_or_another_session() {
        let s = Arc::new(session());
        let mut registry = Registry::default();
        registry.sessions.insert(s.token.clone(), s.clone());
        for token in ["old", "replacement"] {
            registry.tokens.insert(token.into(), s.token.clone());
            registry
                .requests
                .insert(format!("request-{token}"), (s.video.clone(), token.into()));
        }
        assert!(release_viewer(&mut registry, "old").is_none());
        assert!(registry.tokens.contains_key("replacement"));
        assert!(registry.requests.contains_key("request-replacement"));
        assert!(!registry.requests.contains_key("request-old"));
        assert!(release_viewer(&mut registry, "old").is_none());
        assert_eq!(
            release_viewer(&mut registry, "replacement").unwrap().token,
            s.token
        );
    }
    #[test]
    fn same_video_source_changes_preserve_the_selected_document() {
        let s = session();
        s.epoch.store(0, Ordering::Release);
        assert!(!block_changed_source(&s, "about:blank"));
        for source in ["https://www.youtube.com/watch?v=M7lc1UVf-VE", "https://www.youtube.com/watch?v=M7lc1UVf-VE&autoplay=1#same", "https://www.youtube.com/watch?t=2&v=M7lc1UVf-VE"] {
            assert!(!block_changed_source(&s, source), "{source}");
        }
        assert!(reusable(&s)); assert_eq!(s.epoch.load(Ordering::Acquire), 0);
    }
    #[test]
    fn foreign_spa_source_permanently_blocks_only_its_receiver_and_old_messages() {
        for source in ["https://www.youtube.com/watch?v=abcdefghijk", "https://www.youtube.com/watch?v=M7lc1UVf-VE&v=M7lc1UVf-VE", "https://www.youtube.com/", "https://www.youtube.com.evil.test/watch?v=M7lc1UVf-VE", "http://www.youtube.com/watch?v=M7lc1UVf-VE", "about:blank", "not-a-url"] {
            let s = session(); let other = session(); let revision = s.paint.load(Ordering::Acquire);
            assert!(block_changed_source(&s, source), "{source}");
            assert!(!reusable(&s)); assert!(reusable(&other));
            assert_eq!(s.epoch.load(Ordering::Acquire), 3); assert!(s.paint.load(Ordering::Acquire) > revision);
            assert_eq!(lock(&s.status).error.as_deref(), Some(IDENTITY_ERROR));
            let mut m = message("status"); m.epoch = 3; m.fields["state"] = json!("playing"); m.fields["unavailable"] = json!(false);
            update_status(&s, &m);
            assert_eq!(lock(&s.status).state, "error"); assert_eq!(lock(&s.status).error.as_deref(), Some(IDENTITY_ERROR));
            assert!(!accepts("https://www.youtube.com/watch?v=M7lc1UVf-VE", &s.label, &s, &m));
            assert!(!block_changed_source(&s, "about:blank")); assert!(!block_changed_source(&s, "https://www.youtube.com/watch?v=M7lc1UVf-VE"));
            assert_eq!(s.epoch.load(Ordering::Acquire), 3);
        }
    }
    #[test]
    fn late_source_event_cannot_revive_or_mutate_closed_receiver() {
        let s = session(); s.closed.store(true, Ordering::Release);
        assert!(!block_changed_source(&s, "https://www.youtube.com/watch?v=abcdefghijk"));
        assert!(!reusable(&s)); assert!(!s.identity_blocked.load(Ordering::Acquire));
        assert_eq!(s.epoch.load(Ordering::Acquire), 2); assert!(lock(&s.status).error.is_none());
    }
    #[test]
    fn blocked_reuse_cannot_bypass_a_full_combined_document_capacity() {
        let host = WatchHost::default(); let s = Arc::new(session());
        lock(&host.registry).sessions.insert(s.token.clone(), s.clone());
        // Three LIVE documents plus this VOD leave no creation permit.
        let permit = 3 + host.count() < 4;
        assert!(!permit);
        assert_eq!(reuse_or_admit(&lock(&host.registry), &s.video, permit).unwrap().unwrap().token, s.token);
        // SourceChanged arrives after the parent count, before latest reuse.
        assert!(block_changed_source(&s, "https://www.youtube.com/watch?v=abcdefghijk"));
        assert!(host.contains_video(&s.video));
        assert!(reuse_or_admit(&lock(&host.registry), &s.video, permit).is_err());
        assert!(reuse_or_admit(&lock(&host.registry), "abcdefghijk", permit).is_err());
        assert!(reuse_or_admit(&lock(&host.registry), &s.video, true).unwrap().is_none());
        assert_eq!(host.count(), 1);
    }
}
