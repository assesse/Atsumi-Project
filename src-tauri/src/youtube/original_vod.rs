//! Watch-only sessions share YouTube's original account profile, not live capture authority.
use super::{
    lock, policy, profile, Envelope, OriginalSnapshot, VodPlayability, YouTubeOriginalHost, STALE, UNAVAILABLE,
};
use crate::platform_native_geometry::{self, NativeViewport};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager, Webview, WebviewBuilder, WebviewUrl};
#[path = "original_vod_frames.rs"]
mod frames;
pub(crate) use frames::NativeFrameWindow;
#[path = "original_vod_owned.rs"]
mod owned;
pub(crate) use owned::OwnedVodLease;

const PREFIX: &str = "ATSUMI_YOUTUBE_VOD_V1:";
const TOKEN_PREFIX: &str = "youtube-vod-";
const SITE_ERROR: &str =
    "YouTube에서 이 영상을 재생할 수 없습니다. 공식 플레이어의 안내를 확인해 주세요.";
const LOGIN_ERROR: &str = "이 영상을 볼 수 있는 YouTube 계정으로 로그인해 주세요.";
const IDENTITY_ERROR: &str = "다른 영상으로의 자동 전환을 차단했습니다. 선택한 영상을 다시 열어 주세요.";
const CLOCK_MAX_AGE: Duration = Duration::from_millis(1500);
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub struct VodClockFields {
    pub video_id: String, pub source_id: String, pub source_generation: u64,
    pub sequence: u64, pub seek_generation: u64, pub current_time: Option<f64>,
    pub duration: Option<f64>, pub paused: bool, pub seeking: bool, pub playback_rate: f64,
    pub buffering: bool, pub ended: bool, pub ad_state: String, pub valid: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct VodClock {
    pub token: String, pub document_epoch: u64, pub age_ms: u64,
    #[serde(flatten)] pub fields: VodClockFields,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub struct VodControlRequest {
    pub request_id: String, pub document_epoch: u64, pub source_id: String,
    pub source_generation: u64, pub action: String, pub seconds: Option<f64>, pub rate: Option<f64>,
}
struct ClockState { epoch:u64, fields:VodClockFields, received:Instant }
struct ControlBusy<'a>(&'a AtomicBool);
impl Drop for ControlBusy<'_> {fn drop(&mut self){self.0.store(false,Ordering::Release);}}
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
    owned_by: Option<uuid::Uuid>,
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
    clock: Mutex<Option<ClockState>>,
    control_busy: AtomicBool,
    frame_busy: AtomicBool,
    presentation_owner: Mutex<Option<String>>,
    close_enabled: AtomicBool,
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
        self.open_inner(app,host,video,request,can_create_document,None)
    }
    fn open_inner(&self,app:&AppHandle,host:YouTubeOriginalHost,video:String,request:String,
        can_create_document:bool,owned_by:Option<uuid::Uuid>)->Result<OriginalSnapshot,String>{
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
            if let Some(session) = if owned_by.is_none(){reuse_or_admit(&registry,&video,can_create_document)?}
                else if can_create_document{None}else{return Err("YouTube 영상은 최대 4개까지 열 수 있습니다.".into());} {
                registry.tokens.insert(token.clone(), session.token.clone());
                registry.requests.insert(request, (video, token.clone()));
                let mut value = lock(&session.status).clone();
                value.token = token;
                return Ok(value);
            }
        }
        let session = Arc::new(Session {
            owned_by,
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
                playability: None,
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
            clock: Mutex::new(None), control_busy: AtomicBool::new(false), frame_busy: AtomicBool::new(false),
            presentation_owner: Mutex::new(None), close_enabled: AtomicBool::new(false),
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
    pub fn clock(&self,token:&str)->Result<Option<VodClock>,String>{
        let session=self.session(token)?;
        Ok(clock_snapshot(&session,token))
    }
    pub(crate) fn sample_frames(&self,app:&AppHandle,token:&str,video:&str,cancel:&AtomicBool)->Result<NativeFrameWindow,String>{
        frames::sample(self,app,token,video,cancel)
    }
    fn publish_clock(&self,app:&AppHandle,session:&Session){
        if session.owned_by.is_some(){return;}
        let tokens=lock(&self.registry).tokens.iter().filter(|(_,id)|*id==&session.token).map(|(token,_)|token.clone()).collect::<Vec<_>>();
        for token in tokens {if let Some(value)=clock_snapshot(session,&token){let _=app.emit_to("main","youtube-original:vod-clock",value);}}
    }
    pub fn control(&self,app:&AppHandle,token:&str,request:VodControlRequest)->Result<(),String>{
        let session=self.session(token)?;
        validate_control(&session,&request)?;
        if session.control_busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("이전 영상 조작을 확인 중입니다.".into());}
        let _busy=ControlBusy(&session.control_busy);
        let view=app.get_webview(&session.label).ok_or(UNAVAILABLE)?;
        let (tx,rx)=std::sync::mpsc::sync_channel(1);
        let mut payload=json!(request);
        payload["expiresAtMs"]=json!(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_|STALE)?.as_millis()+2500);
        view.eval_with_callback(format!("window.__atsumiYouTubeVod?.control({payload})"),move|value|{let _=tx.try_send(value);}).map_err(|_|UNAVAILABLE)?;
        let raw=rx.recv_timeout(Duration::from_secs(3)).map_err(|_|"영상 조작 응답을 확인하지 못했습니다.".to_string())?;
        if !Arc::ptr_eq(&session,&self.session(token)?) || session.closed.load(Ordering::Acquire) || session.identity_blocked.load(Ordering::Acquire) || session.epoch.load(Ordering::Acquire)!=request.document_epoch{return Err(STALE.into());}
        let value:serde_json::Value=serde_json::from_str(&raw).map_err(|_|UNAVAILABLE)?;
        if value["requestId"].as_str()!=Some(&request.request_id)||value["accepted"].as_bool()!=Some(true){return Err("현재 영상 상태에서는 이 조작을 실행할 수 없습니다.".into());}
        let current=clock_snapshot(&session,token).ok_or(STALE)?;
        if current.fields.source_id!=request.source_id||current.fields.source_generation!=request.source_generation{return Err(STALE.into());}
        Ok(())
    }
    pub fn controls(&self,app:&AppHandle,token:&str,can_close:bool)->Result<(),String>{
        let session=self.session(token)?;
        if session.identity_blocked.load(Ordering::Acquire)||lock(&session.presentation_owner).as_deref()!=Some(token){return Ok(());}
        session.close_enabled.store(can_close,Ordering::Release);
        app.get_webview(&session.label).ok_or(UNAVAILABLE)?.eval(format!("window.__atsumiYouTubeVod?.controls({})",json!({"epoch":session.epoch.load(Ordering::Acquire),"canClose":can_close}))).map_err(|_|UNAVAILABLE.into())
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
        *lock(&session.presentation_owner)=Some(token.into());
        session.close_enabled.store(false,Ordering::Release);
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
        *lock(&session.clock)=None;
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
            *lock(&session.clock)=None;
            let _ = fullscreen(app, &session, false);
            {
                let mut status = lock(&session.status);
                if session.identity_blocked.load(Ordering::Acquire) { continue; }
                status.authenticated = None;
                status.state = "loading".into();
                status.playability = None;
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
                if policy::account_url(next) { let mut status = lock(&navigation.status); status.error = Some(LOGIN_ERROR.into()); status.playability = Some(VodPlayability::LoginRequired); }
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
                    *lock(&page.clock)=None;page.close_enabled.store(false,Ordering::Release);
                    let mut status = lock(&page.status); status.state = "loading".into(); status.authenticated = None; status.error = None; status.playability = None;
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
        let mut owner=lock(&session.presentation_owner);if owner.as_deref()==Some(token){*owner=None;session.close_enabled.store(false,Ordering::Release);}
    }
    if registry.tokens.values().any(|value| value == &id) {
        None
    } else {
        registry.sessions.get(&id).cloned()
    }
}
fn reusable(session: &Session) -> bool {
    session.owned_by.is_none() && !session.closed.load(Ordering::Acquire) && !session.identity_blocked.load(Ordering::Acquire)
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
    let (event,payload)=fullscreen_notice(session,enabled);
    let _ = app.emit(event,payload);
}
fn fullscreen_notice(session:&Session,enabled:bool)->(&'static str,serde_json::Value){
    match session.owned_by {
        Some(owner)=>("youtube-owned-vod:fullscreen",json!({"owner":owner,"enabled":enabled})),
        None=>("youtube-original:fullscreen",json!({"channelId":session.video,"enabled":enabled})),
    }
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
        && matches!(message.kind.as_str(), "status" | "fullscreen" | "clock" | "viewer_close")
        && tauri::Url::parse(source)
            .ok()
            .is_some_and(|url| policy::matches_document(&url, &session.video, false))
}
fn valid_clock(fields:&VodClockFields,video:&str)->bool{
    let safe=|value:u64|value>0&&value<=9_007_199_254_740_991;
    fields.video_id==video&&uuid::Uuid::parse_str(&fields.source_id).is_ok()
        &&safe(fields.source_generation)&&safe(fields.sequence)&&safe(fields.seek_generation)
        &&fields.playback_rate.is_finite()&&(0.1..=16.).contains(&fields.playback_rate)
        &&if fields.valid {fields.ad_state=="content"&&matches!((fields.current_time,fields.duration),(Some(t),Some(d)) if t.is_finite()&&d.is_finite()&&t>=0.&&d>0.&&d<=604800.&&t<=d+2.)}
          else {matches!(fields.ad_state.as_str(),"ad"|"unknown")&&fields.current_time.is_none()&&fields.duration.is_none()&&!fields.ended}
}
fn update_clock(session:&Session,message:&Envelope)->bool{
    let Ok(fields)=serde_json::from_value::<VodClockFields>(message.fields.clone()) else{return false;};
    if !valid_clock(&fields,&session.video){return false;}
    let mut current=lock(&session.clock);
    if session.closed.load(Ordering::Acquire)||session.identity_blocked.load(Ordering::Acquire)||message.epoch!=session.epoch.load(Ordering::Acquire){return false;}
    if let Some(old)=current.as_ref().filter(|old|old.epoch==message.epoch){
        if fields.sequence<=old.fields.sequence||fields.source_generation<old.fields.source_generation||fields.seek_generation<old.fields.seek_generation
            ||fields.source_generation==old.fields.source_generation&&fields.source_id!=old.fields.source_id{return false;}
    }
    *current=Some(ClockState{epoch:message.epoch,fields,received:Instant::now()});true
}
fn clock_snapshot(session:&Session,token:&str)->Option<VodClock>{
    let current=lock(&session.clock);let value=current.as_ref()?;
    if session.closed.load(Ordering::Acquire)||session.identity_blocked.load(Ordering::Acquire)||value.epoch!=session.epoch.load(Ordering::Acquire)||value.received.elapsed()>CLOCK_MAX_AGE{return None;}
    Some(VodClock{token:token.into(),document_epoch:value.epoch,age_ms:value.received.elapsed().as_millis().min(u64::MAX as u128) as u64,fields:value.fields.clone()})
}
fn validate_control(session:&Session,request:&VodControlRequest)->Result<(),String>{
    let clock=clock_snapshot(session,&session.token).ok_or(STALE)?;
    if uuid::Uuid::parse_str(&request.request_id).is_err()||request.document_epoch!=clock.document_epoch||request.source_id!=clock.fields.source_id||request.source_generation!=clock.fields.source_generation{return Err(STALE.into());}
    match request.action.as_str(){
        "pause" if request.seconds.is_none()&&request.rate.is_none()=>Ok(()),
        "resume" if request.seconds.is_none()&&request.rate.is_none()&&clock.fields.valid&&lock(&session.viewport).as_ref().is_some_and(|v|v.visible&&!v.muted&&v.player.is_some())=>Ok(()),
        "seek" if request.rate.is_none()&&clock.fields.valid&&matches!((request.seconds,clock.fields.duration),(Some(t),Some(d)) if t.is_finite()&&t>=0.&&t<=d)=>Ok(()),
        "rate" if request.seconds.is_none()&&clock.fields.valid&&request.rate.is_some_and(|rate|rate.is_finite()&&(0.1..=16.).contains(&rate))=>Ok(()),
        _=>Err("현재 영상 상태에서는 이 조작을 실행할 수 없습니다.".into())
    }
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
        status.playability = Some(if message.fields["playability"] == "LOGIN_REQUIRED" { VodPlayability::LoginRequired } else { VodPlayability::Unavailable });
        status.error = Some(if status.playability == Some(VodPlayability::LoginRequired) { LOGIN_ERROR } else { SITE_ERROR }.into());
    } else if matches!(status.state.as_str(), "playing" | "ended") {
        status.playability = None;
        if matches!(status.error.as_deref(), Some(SITE_ERROR | LOGIN_ERROR)) { status.error = None; }
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
    *lock(&session.clock)=None;session.close_enabled.store(false,Ordering::Release);
    let mut status = lock(&session.status);
    status.state = "error".into(); status.error = Some(IDENTITY_ERROR.into()); status.playability = None;
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
                    } else if message.kind=="clock" {
                        if update_clock(&session,&message){host.watch_only.publish_clock(&app,&session);}
                    } else if message.kind=="viewer_close" {
                        if message.fields.as_object().is_some_and(|v|v.len()==1)&&message.fields["inputKey"].as_i64()==Some(1)
                            &&session.close_enabled.load(Ordering::Acquire)&&platform_native_geometry::trusted_input_gate(root,pane,1).allowed()
                            &&lock(&session.viewport).as_ref().is_some_and(|v|v.visible&&v.player.is_some()) {
                            if let Some(owner)=session.owned_by {let _=app.emit_to("main","youtube-owned-vod:close",json!({"owner":owner}));}
                            else if let Some(token)=lock(&session.presentation_owner).clone(){let _=app.emit_to("main","youtube-original:action",json!({"token":token,"action":"viewer_close"}));}
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
    pub(super) fn session() -> Session {
        Session {
            owned_by: None,
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
                playability: None,
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
            clock: Mutex::new(None),control_busy:AtomicBool::new(false),frame_busy:AtomicBool::new(false),presentation_owner:Mutex::new(None),close_enabled:AtomicBool::new(false),
        }
    }
    fn message(kind: &str) -> Envelope {
        serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"epoch":2,"type":kind,"state":"ended","authenticated":true,"captureReady":true,"recordingJob":"forged"})).unwrap()
    }
    fn clock_message()->Envelope{
        serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"epoch":2,"type":"clock","videoId":"M7lc1UVf-VE","sourceId":uuid::Uuid::new_v4(),"sourceGeneration":1,"sequence":1,"seekGeneration":1,"currentTime":15.25,"duration":100.5,"paused":false,"seeking":false,"playbackRate":1.,"buffering":false,"ended":false,"adState":"content","valid":true})).unwrap()
    }
    #[test]
    fn vod_clock_has_closed_schema_and_content_ad_separation(){
        let s=session();let m=clock_message();assert!(update_clock(&s,&m));let value=clock_snapshot(&s,"alias").unwrap();assert_eq!(value.token,"alias");assert_eq!(value.fields.current_time,Some(15.25));
        for (field,value) in [("extra",json!(true)),("videoId",json!("abcdefghijk")),("sourceId",json!("invalid")),("sequence",json!(0)),("duration",json!(-1)),("currentTime",json!(1000)),("adState",json!("ad")),("playbackRate",json!(0))]{let mut invalid=clock_message();invalid.fields[field]=value;assert!(!update_clock(&session(),&invalid),"{field}");}
        let mut ad=clock_message();ad.fields["sequence"]=json!(2);ad.fields["sourceGeneration"]=json!(2);ad.fields["valid"]=json!(false);ad.fields["adState"]=json!("ad");ad.fields["currentTime"]=json!(null);ad.fields["duration"]=json!(null);assert!(update_clock(&s,&ad));assert!(clock_snapshot(&s,"alias").unwrap().fields.current_time.is_none());
    }
    #[test]
    fn vod_clock_rejects_stale_epoch_source_sequence_and_expiry(){
        let s=session();let mut m=clock_message();assert!(update_clock(&s,&m));assert!(!update_clock(&s,&m));m.fields["sequence"]=json!(2);let id=m.fields["sourceId"].clone();m.fields["sourceId"]=json!(uuid::Uuid::new_v4());assert!(!update_clock(&s,&m));m.fields["sourceId"]=id;m.epoch=1;assert!(!update_clock(&s,&m));m.epoch=2;assert!(update_clock(&s,&m));
        lock(&s.clock).as_mut().unwrap().received=Instant::now()-Duration::from_secs(2);assert!(clock_snapshot(&s,"alias").is_none());
        m.fields["sequence"]=json!(3);assert!(update_clock(&s,&m));s.epoch.store(3,Ordering::Release);assert!(clock_snapshot(&s,"alias").is_none());m.epoch=3;m.fields["sequence"]=json!(1);assert!(update_clock(&s,&m));s.closed.store(true,Ordering::Release);assert!(clock_snapshot(&s,"alias").is_none());
    }
    #[test]
    fn vod_control_requires_exact_source_and_content_for_seek_or_resume(){
        let s=session();let m=clock_message();assert!(update_clock(&s,&m));
        let mut request=VodControlRequest{request_id:uuid::Uuid::new_v4().to_string(),document_epoch:2,source_id:m.fields["sourceId"].as_str().unwrap().into(),source_generation:1,action:"seek".into(),seconds:Some(60.),rate:None};
        assert!(validate_control(&s,&request).is_ok());for value in [-1.,101.,f64::NAN,f64::INFINITY]{request.seconds=Some(value);assert!(validate_control(&s,&request).is_err());}
        request.seconds=None;request.action="pause".into();assert!(validate_control(&s,&request).is_ok());request.action="resume".into();assert!(validate_control(&s,&request).is_err());
        let mut viewport=super::super::hidden_viewport();viewport.visible=true;viewport.muted=false;viewport.player=Some(crate::platform_native_geometry::Rect{x:0.,y:0.,width:640.,height:360.});*lock(&s.viewport)=Some(viewport);assert!(validate_control(&s,&request).is_ok());
        request.source_generation=2;assert!(validate_control(&s,&request).is_err());request.source_generation=1;lock(&s.clock).as_mut().unwrap().fields.valid=false;assert!(validate_control(&s,&request).is_err());request.action="pause".into();assert!(validate_control(&s,&request).is_ok());
        request.action="rate".into();request.rate=Some(1.5);assert!(validate_control(&s,&request).is_err());lock(&s.clock).as_mut().unwrap().fields.valid=true;assert!(validate_control(&s,&request).is_ok());
        for rate in [0.,-1.,17.,f64::NAN,f64::INFINITY]{request.rate=Some(rate);assert!(validate_control(&s,&request).is_err());}
        request.rate=Some(1.5);request.seconds=Some(0.);assert!(validate_control(&s,&request).is_err());request.action="seek".into();assert!(validate_control(&s,&request).is_err());request.seconds=None;request.action="pause".into();assert!(validate_control(&s,&request).is_err());
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
    fn login_required_is_typed_and_clears_without_certifying_the_upload_account() {
        let s = session();
        let mut m = message("status");
        m.fields["state"] = json!("error"); m.fields["unavailable"] = json!(true);
        m.fields["playability"] = json!("LOGIN_REQUIRED"); m.fields["authenticated"] = json!(false);
        update_status(&s, &m);
        let status = lock(&s.status).clone();
        assert_eq!(status.playability, Some(VodPlayability::LoginRequired));
        assert_eq!(status.error.as_deref(), Some(LOGIN_ERROR));
        assert_eq!(serde_json::to_value(status).unwrap()["playability"], "LOGIN_REQUIRED");
        m.fields["unavailable"] = json!(false); m.fields["state"] = json!("loading");
        update_status(&s, &m); assert_eq!(lock(&s.status).playability, Some(VodPlayability::LoginRequired));
        m.fields["unavailable"] = json!(true);
        m.fields["playability"] = json!("forged-other-reason"); m.fields["authenticated"] = json!(true);
        update_status(&s, &m);
        assert_eq!(lock(&s.status).playability, Some(VodPlayability::Unavailable));
        assert_eq!(lock(&s.status).error.as_deref(), Some(SITE_ERROR));
        m.fields["unavailable"] = json!(false); m.fields["state"] = json!("playing");
        update_status(&s, &m);
        assert!(lock(&s.status).playability.is_none()); assert!(lock(&s.status).error.is_none());
        // Stale account-page status must not reintroduce a recovery action.
        m.epoch = 1; m.fields["unavailable"] = json!(true); m.fields["playability"] = json!("LOGIN_REQUIRED");
        update_status(&s, &m); assert!(lock(&s.status).playability.is_none());
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
