//! SOOP-owned original document receiver. Viewer and recorder share this child.
#[path = "original_documents.rs"]
mod documents;
#[path = "original_policy.rs"]
mod policy;
#[path = "original_vod.rs"]
mod vod;
use crate::platform_media::{
    encoded::{EncodedAppend, EncodedArchive, EncodedTrack, OfficialReceiverLease},
    live::RecordingOwner,
    MediaRef, PlatformMediaService,
};
use crate::platform_native_geometry::{self, NativeViewport};
use crate::platform_native_player_intent as player_intent;
use crate::platform_window_fullscreen::{self as window_fullscreen, Grant};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, Emitter, Manager, Webview, WebviewBuilder, WebviewUrl, WebviewWindowBuilder,
};
pub use vod::Snapshot as VodSnapshot;

const UNAVAILABLE: &str = "SOOP 원본 플레이어를 준비하지 못했습니다.";
const STALE: &str = "SOOP 방송 상태가 변경되었습니다. 다시 확인해 주세요.";
fn reserve_ui_diagnostic(budget:&AtomicU64)->bool {
    budget.fetch_update(Ordering::AcqRel,Ordering::Acquire,|value|if value<48{Some(value+1)}else{None}).is_ok()
}
fn ui_diagnostic(session:&Session,stage:&'static str,operation:&'static str,success:bool){
    if reserve_ui_diagnostic(&session.ui_diagnostics){tracing::info!(diag_stage=stage,diag_operation=operation,success);}
}
fn ui_action_category(value:Option<&str>)->Option<&'static str>{match value{
    Some("record_start")=>Some("record_start"),Some("record_stop")=>Some("record_stop"),Some("screenshot")=>Some("screenshot"),Some("viewer_close")=>Some("viewer_close"),_=>None,
}}
fn pointer_category(fields:&Value)->Option<&'static str>{
    if !fields.as_object().is_some_and(|fields|fields.len()==1&&fields.contains_key("target")){return None;}
    match fields["target"].as_str(){Some("record")=>Some("record"),Some("camera")=>Some("camera"),Some("close")=>Some("close"),Some("player_other")=>Some("player_other"),_=>None}
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginalSnapshot {
    pub token: String,
    pub channel_id: String,
    pub broadcast_id: String,
    pub state: String,
    pub authenticated: Option<bool>,
    pub chat_visible: bool,
    pub capture_ready: bool,
    pub recording_job: Option<String>,
    pub recording_stopping: bool,
    pub fullscreen: bool,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    pub authenticated: Option<bool>,
    pub open: bool,
    pub error: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotSnapshot {
    pub item: crate::platform_media::screenshots::Item,
    pub preview: String,
}
#[derive(Default)]
struct Registry {
    sessions: HashMap<String, Arc<Session>>,
    tokens: HashMap<String, String>,
    requests: HashMap<String, String>,
}
struct Session {
    id: String,
    label: String,
    channel: String,
    broadcast: String,
    title: String,
    name: String,
    document: documents::Permit,
    epoch: AtomicU64,
    account_revision: AtomicU64,
    paint: Arc<AtomicU64>,
    closed: AtomicBool,
    opening: AtomicBool,
    viewers: Mutex<HashSet<String>>,
    ui_intent: Arc<player_intent::Intent>,
    ui_diagnostics: AtomicU64,
    close_enabled: AtomicBool,
    status: Mutex<OriginalSnapshot>,
    fullscreen_grant: Mutex<Option<Grant>>,
    arm: Mutex<Option<(policy::Arm, RecordingOwner)>>,
    archive: Mutex<Option<EncodedArchive>>,
    source: Mutex<Option<String>>,
    owner: Mutex<Option<RecordingOwner>>,
    lease: OfficialReceiverLease,
    viewport: Mutex<Option<NativeViewport>>,
    viewport_sequences: Mutex<HashMap<String, u64>>,
    capture_busy: AtomicBool,
    last_input: Mutex<Option<Instant>>,
    pending_begin: Mutex<Option<Instant>>,
    stop_requested: Mutex<Option<Instant>>,
    account_refresh_pending: AtomicBool,
    account_refreshing: AtomicBool,
}
#[derive(Clone, Default)]
pub struct SoopOriginalHost {
    registry: Arc<Mutex<Registry>>,
    documents: documents::Documents,
    vod: vod::WatchHost,
    closing: Arc<AtomicBool>,
    tray_muted: Arc<AtomicBool>,
    account_state: Arc<Mutex<Option<bool>>>,
    account_flow: Arc<Mutex<policy::AccountFlow>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    #[serde(skip)]
    intent: Option<player_intent::Binding>,
    id: String,
    epoch: u64,
    #[serde(rename = "type")]
    kind: String,
    #[serde(flatten)]
    fields: Value,
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}
fn recording_approval<'a, T>(
    slot: &'a Mutex<Option<T>>,
    closed: &AtomicBool,
    refreshing: &AtomicBool,
    ready: impl FnOnce() -> bool,
) -> Result<std::sync::MutexGuard<'a, Option<T>>, String> {
    let approval = lock(slot);
    // Account refresh clears readiness under this same approval mutex.
    if closed.load(Ordering::Acquire) || refreshing.load(Ordering::Acquire) || !ready() {
        return Err("원본 영상이 준비된 뒤 녹화를 시작해 주세요.".into());
    }
    Ok(approval)
}
fn current_status<T>(
    _approval: &std::sync::MutexGuard<'_, Option<T>>,
    closed: &AtomicBool,
    refreshing: &AtomicBool,
    current_epoch: &AtomicU64,
    message_epoch: u64,
) -> bool {
    !closed.load(Ordering::Acquire)
        && !refreshing.load(Ordering::Acquire)
        && current_epoch.load(Ordering::Acquire) == message_epoch
}
pub(super) fn profile(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|_| UNAVAILABLE)?
        .join("soop-original-profile");
    std::fs::create_dir_all(&path).map_err(|_| UNAVAILABLE)?;
    Ok(path)
}
impl SoopOriginalHost {
    pub fn open(
        &self,
        app: &AppHandle,
        media: PlatformMediaService,
        channel: String,
        request_id: String,
        download_root: PathBuf,
    ) -> Result<OriginalSnapshot, String> {
        if self.closing.load(Ordering::Acquire)
            || !policy::valid_channel(&channel)
            || uuid::Uuid::parse_str(&request_id).is_err()
        {
            return Err(STALE.into());
        }
        let channel = channel.to_ascii_lowercase();
        {
            let registry = lock(&self.registry);
            if let Some(token) = registry.requests.get(&request_id) {
                return self.snapshot_locked(&registry, token);
            }
        }
        let info = super::observe_channel(&channel)?;
        let broadcast = info
            .broadcast_key
            .filter(|id| policy::valid_broadcast(id))
            .ok_or("방송 중인 채널이 아닙니다.")?;
        // Official player, not the anonymous extractor, enforces account/age/paid restrictions.
        let mut registry = lock(&self.registry);
        if self.closing.load(Ordering::Acquire) {
            return Err(STALE.into());
        }
        if let Some(token) = registry.requests.get(&request_id) {
            return self.snapshot_locked(&registry, token);
        }
        let existing = registry
            .sessions
            .values()
            .find(|s| {
                s.channel == channel
                    && s.broadcast == broadcast
                    && !s.closed.load(Ordering::Acquire)
            })
            .cloned();
        let session = if let Some(session) = existing {
            session
        } else {
            let document = self.documents.acquire()?;
            let id = uuid::Uuid::new_v4().to_string();
            let lease = media.reserve_official_receiver(
                MediaRef {
                    provider: "soop".into(),
                    kind: "live".into(),
                    external_id: channel.clone(),
                },
                broadcast.clone(),
                id.clone(),
                download_root,
            )?;
            let session = Arc::new(Session {
                id: id.clone(),
                label: format!("soop-original-{}", id.replace('-', "")),
                channel: channel.clone(),
                broadcast: broadcast.clone(),
                title: info.title.unwrap_or_else(|| info.name.clone()),
                name: info.name,
                document,
                epoch: AtomicU64::new(0),
                account_revision: AtomicU64::new(0),
                paint: Arc::new(AtomicU64::new(0)),
                closed: AtomicBool::new(false),
                opening: AtomicBool::new(true),
                viewers: Mutex::new(HashSet::new()),
                ui_intent: Arc::new(player_intent::Intent::default()),
                ui_diagnostics: AtomicU64::new(0),
                close_enabled: AtomicBool::new(false),
                status: Mutex::new(OriginalSnapshot {
                    token: String::new(),
                    channel_id: channel.clone(),
                    broadcast_id: broadcast,
                    state: "loading".into(),
                    authenticated: None,
                    chat_visible: false,
                    capture_ready: false,
                    recording_job: None,
                    recording_stopping: false,
                    fullscreen: false,
                    error: None,
                }),
                fullscreen_grant: Mutex::new(None),
                arm: Mutex::new(None),
                archive: Mutex::new(None),
                source: Mutex::new(None),
                owner: Mutex::new(None),
                lease,
                viewport: Mutex::new(None),
                viewport_sequences: Mutex::new(HashMap::new()),
                capture_busy: AtomicBool::new(false),
                last_input: Mutex::new(None),
                pending_begin: Mutex::new(None),
                stop_requested: Mutex::new(None),
                account_refresh_pending: AtomicBool::new(false),
                account_refreshing: AtomicBool::new(false),
            });
            registry.sessions.insert(id, session.clone());
            if let Err(error) = self.create_view(app, &session) {
                registry.sessions.remove(&session.id);
                if let Some(view) = app.get_webview(&session.label) {
                    if view.close().is_err() {
                        session.document.retain_after_close_failure();
                    }
                }
                session.document.release();
                session.closed.store(true, Ordering::Release);
                session.lease.close();
                return Err(error);
            }
            session
        };
        let token = uuid::Uuid::new_v4().to_string();
        lock(&session.viewers).insert(token.clone());
        session.opening.store(false, Ordering::Release);
        registry.tokens.insert(token.clone(), session.id.clone());
        registry.requests.insert(request_id, token.clone());
        let mut result = lock(&session.status).clone();
        result.token = token;
        Ok(result)
    }
    fn snapshot_locked(
        &self,
        registry: &Registry,
        token: &str,
    ) -> Result<OriginalSnapshot, String> {
        let session = registry
            .tokens
            .get(token)
            .and_then(|id| registry.sessions.get(id))
            .ok_or(STALE)?;
        let mut result = lock(&session.status).clone();
        result.token = token.into();
        Ok(result)
    }
    fn session(&self, token: &str) -> Result<Arc<Session>, String> {
        let registry = lock(&self.registry);
        registry
            .tokens
            .get(token)
            .and_then(|id| registry.sessions.get(id))
            .cloned()
            .ok_or_else(|| STALE.into())
    }
    pub fn snapshot(&self, token: &str) -> Result<OriginalSnapshot, String> {
        self.snapshot_locked(&lock(&self.registry), token)
    }
    pub fn vod_open(
        &self,
        app: &AppHandle,
        video: String,
        request: String,
    ) -> Result<VodSnapshot, String> {
        self.vod.open(
            app,
            &self.documents,
            video,
            request,
            self.closing.clone(),
            self.tray_muted.clone(),
        )
    }
    pub fn vod_snapshot(&self, token: &str) -> Result<VodSnapshot, String> {
        self.vod.snapshot(token)
    }
    pub fn vod_viewport(
        &self,
        app: &AppHandle,
        token: &str,
        viewport: NativeViewport,
    ) -> Result<(), String> {
        self.vod.viewport(
            app,
            token,
            viewport,
            self.tray_muted.load(Ordering::Acquire),
        )
    }
    pub fn vod_close(&self, app: &AppHandle, token: &str) -> Result<(), String> {
        self.vod.close(app, token)
    }
    pub fn vod_fullscreen(
        &self,
        app: &AppHandle,
        token: &str,
        enabled: bool,
    ) -> Result<(), String> {
        self.vod.fullscreen(app, token, enabled)
    }
    /// Called only by trusted main IPC on a blocking worker. No new media request.
    pub fn screenshot(
        &self,
        app: &AppHandle,
        token: &str,
        request_id: &str,
    ) -> Result<ScreenshotSnapshot, String> {
        if uuid::Uuid::parse_str(request_id).is_err() {
            return Err(STALE.into());
        }
        let session = self.session(token)?;
        ui_diagnostic(&session,"soop_live_capture_begin","screenshot",true);
        let result=(|| {
        if session.capture_busy.swap(true, Ordering::AcqRel) {
            return Err("스크린샷을 저장 중입니다.".into());
        }
        struct Guard<'a>(&'a AtomicBool);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _guard = Guard(&session.capture_busy);
        let epoch = session.epoch.load(Ordering::Acquire);
        let valid = || {
            !session.closed.load(Ordering::Acquire)
                && session.epoch.load(Ordering::Acquire) == epoch
                && !self.tray_muted.load(Ordering::Acquire)
                && lock(&session.viewers).contains(token)
                && lock(&session.viewport)
                    .as_ref()
                    .is_some_and(|v| v.visible && v.player.is_some())
        };
        if !valid() {
            return Err("화면에 표시된 방송에서만 스크린샷을 저장할 수 있습니다.".into());
        }
        let view = app.get_webview(&session.label).ok_or(STALE)?;
        let url = policy::player_url(&session.channel, &session.broadcast)?;
        let png = crate::platform_native_capture::video_png(
            &view,
            url.as_str(),
            crate::platform_native_capture::VideoSurface::Soop,
        )?;
        if !valid() {
            return Err(STALE.into());
        }
        let item =
            session
                .lease
                .save_screenshot(request_id, &png, &session.title, &session.name)?;
        Ok(ScreenshotSnapshot {
            item,
            preview: format!("data:image/png;base64,{}", STANDARD.encode(&png)),
        })
        })();
        ui_diagnostic(&session,"soop_live_capture_result","screenshot",result.is_ok());
        result
    }
    pub fn undo_screenshot(&self, token: &str, id: &str) -> Result<(), String> {
        self.session(token)?.lease.undo_screenshot(id)
    }
    pub fn fullscreen(&self, app: &AppHandle, token: &str, enabled: bool) -> Result<(), String> {
        set_fullscreen(app, &self.session(token)?, enabled)
    }
    /// Preparation waits outside the scheduler's admission/core lock.
    pub fn prepare_scheduled(
        &self,
        app: &AppHandle,
        media: PlatformMediaService,
        channel: String,
        broadcast: &str,
        request: String,
        root: PathBuf,
    ) -> Result<String, String> {
        let first = self.open(app, media, channel, request, root)?;
        if first.broadcast_id != broadcast {
            let _ = self.close(app, &first.token);
            return Err("방송이 변경되어 예약 시작을 취소했습니다.".into());
        }
        let until = Instant::now() + Duration::from_secs(45);
        while Instant::now() < until && !self.closing.load(Ordering::Acquire) {
            match self.snapshot(&first.token) {
                Ok(current) if current.capture_ready => return Ok(first.token),
                Ok(current) if current.state == "ended" || current.error.is_some() => {
                    let _ = self.close(app, &first.token);
                    return Err(current
                        .error
                        .unwrap_or_else(|| "방송이 종료되었습니다.".into()));
                }
                Err(error) => return Err(error),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.close(app, &first.token);
        Err("SOOP 원본 영상 준비 시간이 초과되었습니다.".into())
    }
    pub fn start_scheduled(
        &self,
        app: &AppHandle,
        token: &str,
        owner: RecordingOwner,
    ) -> Result<crate::platform_media::live::ScheduledCapture, String> {
        let session = self.session(token)?;
        let RecordingOwner::Scheduled {
            schedule_id,
            broadcast_key,
            ..
        } = &owner
        else {
            return Err(STALE.into());
        };
        if schedule_id != &session.channel || broadcast_key != &session.broadcast {
            return Err(STALE.into());
        }
        self.arm(app, &session, owner.clone())?;
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until && !self.closing.load(Ordering::Acquire) {
            let current = lock(&session.status).clone();
            if let Some(job_id) = current.recording_job {
                return Ok(crate::platform_media::live::ScheduledCapture {
                    session_id: session.id.clone(),
                    job_id,
                    owner,
                });
            }
            if let Some(error) = current.error {
                return Err(error);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        lock(&session.arm).take();
        // A begin racing the timeout still belongs to this exact reservation.
        if lock(&session.owner).as_ref() == Some(&owner) {
            let _ = self.stop_recording(app, &session);
        }
        Err("예약 녹화 시작 응답이 지연되었습니다.".into())
    }
    pub fn stop_scheduled(
        &self,
        app: &AppHandle,
        capture: &crate::platform_media::live::ScheduledCapture,
    ) -> Result<(), String> {
        let session = lock(&self.registry)
            .sessions
            .get(&capture.session_id)
            .cloned();
        if let Some(session) = session {
            if lock(&session.owner).as_ref() == Some(&capture.owner)
                && lock(&session.status).recording_job.as_ref() == Some(&capture.job_id)
            {
                self.stop_recording(app, &session)?;
            }
        }
        Ok(())
    }
    pub fn viewport(
        &self,
        app: &AppHandle,
        token: &str,
        mut viewport: NativeViewport,
    ) -> Result<(), String> {
        viewport.validate()?;
        let session = self.session(token)?;
        // Keep admission and paint submission serialized. A new React effect can
        // overtake the preceding invocation when privacy or tab visibility changes.
        let mut sequences = lock(&session.viewport_sequences);
        if session.closed.load(Ordering::Acquire) || !lock(&session.viewers).contains(token) {
            return Err(STALE.into());
        }
        if !accept_viewport_sequence(&mut sequences, token, viewport.sequence) {
            return Ok(());
        }
        let revision = session.paint.fetch_add(1, Ordering::AcqRel) + 1;
        session.ui_intent.present(token,session.epoch.load(Ordering::Acquire),&viewport);
        *lock(&session.viewport) = Some(viewport.clone());
        viewport.muted |= self.tray_muted.load(Ordering::Acquire);
        if let Some(view) = app.get_webview(&session.label) {
            let payload = serde_json::to_string(&viewport).map_err(|_| UNAVAILABLE)?;
            view.eval(&format!("window.__atsumiSoopOriginal?.layout({payload})"))
                .map_err(|_| UNAVAILABLE)?;
            platform_native_geometry::paint(&view, &viewport, session.paint.clone(), revision)?;
        }
        Ok(())
    }
    pub fn record(
        &self,
        app: &AppHandle,
        token: &str,
        enabled: bool,
    ) -> Result<OriginalSnapshot, String> {
        let session = self.session(token)?;
        if enabled {
            self.arm(app, &session, RecordingOwner::Manual)?;
        } else {
            self.stop_recording(app, &session)?;
        }
        self.snapshot(token)
    }
    pub fn controls(&self, app:&AppHandle, token:&str, can_close:bool, capture_busy:bool)->Result<(),String>{
        let session=self.session(token)?;let Some(binding)=ui_binding(&session) else{return Ok(());};
        if binding.token!=token{return Ok(());}
        session.close_enabled.store(can_close,Ordering::Release);
        let status=lock(&session.status).clone();let pending=lock(&session.arm).is_some();
        app.get_webview(&session.label).ok_or(UNAVAILABLE)?.eval(&format!("window.__atsumiSoopOriginal?.controls({})",json!({"epoch":binding.epoch,"canClose":can_close,"captureBusy":capture_busy,"canRecord":status.capture_ready,"recording":status.recording_job.is_some(),"recordBusy":pending||status.recording_stopping}))).map_err(|_|UNAVAILABLE.into())
    }
    pub fn resume(&self,app:&AppHandle,token:&str)->Result<(),String>{
        let session=self.session(token)?;let binding=ui_binding(&session).filter(|b|b.token==token).ok_or(STALE)?;
        let weak=Arc::downgrade(&session);let tray=self.tray_muted.clone();let app2=app.clone();
        app.run_on_main_thread(move||{if let Some(session)=weak.upgrade(){if !tray.load(Ordering::Acquire)&&ui_binding(&session).as_ref()==Some(&binding){if let Some(view)=app2.get_webview(&session.label){let _=view.eval(&format!("window.__atsumiSoopOriginal?.resume({})",binding.epoch));}}}}).map_err(|_|UNAVAILABLE.into())
    }
    fn arm(
        &self,
        app: &AppHandle,
        session: &Arc<Session>,
        owner: RecordingOwner,
    ) -> Result<(), String> {
        let mut approval = recording_approval(
            &session.arm,
            &session.closed,
            &session.account_refreshing,
            || lock(&session.status).capture_ready,
        )?;
        if approval.is_some() || lock(&session.archive).is_some() {
            return Err("이미 녹화 중이거나 준비 중입니다.".into());
        }
        let arm = policy::Arm {
            id: uuid::Uuid::new_v4().to_string(),
            epoch: session.epoch.load(Ordering::Acquire),
            created: Instant::now(),
        };
        let id = arm.id.clone();
        *approval = Some((arm, owner));
        drop(approval);
        lock(&session.status).error = None;
        app.get_webview(&session.label)
            .ok_or(UNAVAILABLE)?
            .eval(&format!(
                "window.__atsumiSoopOriginal?.start({})",
                json!(id)
            ))
            .map_err(|_| UNAVAILABLE)?;
        Ok(())
    }
    fn stop_recording(&self, app: &AppHandle, session: &Arc<Session>) -> Result<(), String> {
        lock(&session.arm).take();
        if lock(&session.archive).is_none() {
            return Ok(());
        }
        lock(&session.status).recording_stopping = true;
        *lock(&session.stop_requested) = Some(Instant::now());
        app.get_webview(&session.label)
            .ok_or(UNAVAILABLE)?
            .eval("window.__atsumiSoopOriginal?.stop()")
            .map_err(|_| UNAVAILABLE)?;
        Ok(())
    }
    pub fn close(&self, app: &AppHandle, token: &str) -> Result<(), String> {
        let session = {
            let mut registry = lock(&self.registry);
            let Some(id) = registry.tokens.remove(token) else {
                return Ok(());
            };
            registry.requests.retain(|_, value| value != token);
            registry.sessions.get(&id).cloned()
        };
        if let Some(session) = session {
            session.ui_intent.hide(token);
            let mut sequences = lock(&session.viewport_sequences);
            sequences.remove(token);
            drop(sequences);
            lock(&session.viewers).remove(token);
            if lock(&session.viewers).is_empty() {
                *lock(&session.viewport) = Some(hidden_viewport());
                if lock(&session.status).fullscreen {
                    let _ = set_fullscreen(app, &session, false);
                }
                if let Some(view) = app.get_webview(&session.label) {
                    let revision = session.paint.fetch_add(1, Ordering::AcqRel) + 1;
                    let _ = platform_native_geometry::paint(
                        &view,
                        &hidden_viewport(),
                        session.paint.clone(),
                        revision,
                    );
                }
                if lock(&session.archive).is_none() && lock(&session.arm).is_none() {
                    self.dispose(app, &session);
                }
            }
        }
        Ok(())
    }
    fn dispose(&self, app: &AppHandle, session: &Arc<Session>) {
        if session.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if lock(&session.status).fullscreen {
            let _ = set_fullscreen(app, session, false);
        }
        session.paint.fetch_add(1, Ordering::AcqRel);
        if let Some(view) = app.get_webview(&session.label) {
            if view.close().is_err() {
                session.document.retain_after_close_failure();
            }
        }
        session.document.release();
        session.lease.close();
        let mut registry = lock(&self.registry);
        registry.sessions.remove(&session.id);
        registry.tokens.retain(|_, id| id != &session.id);
        let remaining = registry.tokens.keys().cloned().collect::<HashSet<_>>();
        registry
            .requests
            .retain(|_, token| remaining.contains(token));
    }
    pub fn account(&self, app: &AppHandle) -> AccountSnapshot {
        let authenticated = *lock(&self.account_state);
        AccountSnapshot {
            // A normal viewer close does not revoke an accepted account result.
            // Old documents cannot override the callback through registry priority.
            authenticated,
            open: app.get_webview_window("soop-original-account").is_some(),
            error: None,
        }
    }
    pub fn login(&self, app: &AppHandle) -> Result<AccountSnapshot, String> {
        self.open_account(app, false)
    }
    pub fn logout(&self, app: &AppHandle) -> Result<AccountSnapshot, String> {
        self.open_account(app, true)
    }
    fn open_account(&self, app: &AppHandle, logout: bool) -> Result<AccountSnapshot, String> {
        let url = policy::account_entry_url(logout);
        if let Some(window) = app.get_webview_window("soop-original-account") {
            window.show().map_err(|_| UNAVAILABLE)?;
            window.set_focus().map_err(|_| UNAVAILABLE)?;
            let changed_intent = {
                let mut flow = lock(&self.account_flow);
                flow.nonce()
                    .and_then(|nonce| flow.reopen(&nonce, !logout))
                    .unwrap_or(false)
            };
            if changed_intent {
                // Opposite intent resets proof; same-intent reopen keeps its security page.
                window.navigate(url).map_err(|_| UNAVAILABLE)?;
            }
            return Ok(self.account(app));
        }
        let nonce = uuid::Uuid::new_v4().to_string();
        lock(&self.account_flow).begin(nonce.clone(), !logout);
        let account_host = self.clone();
        let navigation_host = self.clone();
        let navigation_app = app.clone();
        let navigation_nonce = nonce.clone();
        let page_nonce = nonce.clone();
        let popup_app = app.clone();
        let window = WebviewWindowBuilder::new(
            app,
            "soop-original-account",
            WebviewUrl::External("about:blank".parse().unwrap()),
        )
        .title("SOOP 로그인")
        .data_directory(profile(app)?)
        .browser_extensions_enabled(false)
        .initialization_script(
            include_str!("original_account.js")
                .replace("__ATSUMI_SOOP_ACCOUNT_NONCE__", &json!(nonce).to_string()),
        )
        .inner_size(720.0, 780.0)
        .min_inner_size(400.0, 450.0)
        .on_navigation(move |url| {
            if url.as_str() == "about:blank" {
                return true;
            }
            if policy::account_url(url) {
                return lock(&navigation_host.account_flow)
                    .navigate(&navigation_nonce)
                    .is_some();
            }
            if let Some(secure) = policy::account_https_return(url) {
                let epoch = lock(&navigation_host.account_flow).prepare_resume(&navigation_nonce);
                if let Some(epoch) = epoch {
                    navigation_host.resume_account_home(
                        &navigation_app,
                        &navigation_nonce,
                        epoch,
                        secure,
                    );
                }
            }
            false
        })
        .on_new_window(move |url, features| open_popup(&popup_app, url, features, 0))
        .on_download(|_, _| false)
        .on_page_load(move |view, payload| {
            if !policy::account_url(payload.url()) {
                return;
            }
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                lock(&account_host.account_flow).navigate(&page_nonce);
            } else if view.url().ok().as_ref() == Some(payload.url()) {
                let epoch = lock(&account_host.account_flow).finish(&page_nonce, payload.url());
                if let Some(epoch) = epoch {
                    if policy::account_observation_url(payload.url()) {
                        let _ =
                            view.eval(&format!("window.__atsumiSoopAccount?.configure({epoch})"));
                    }
                    account_host.finish_account_window(view.app_handle(), &page_nonce, epoch);
                }
            }
        })
        .build()
        .map_err(|_| UNAVAILABLE)?;
        let destroyed_host = self.clone();
        let destroyed_nonce = nonce.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                lock(&destroyed_host.account_flow).invalidate(&destroyed_nonce);
            }
        });
        attach_account_bridge(
            &app.get_webview("soop-original-account")
                .ok_or(UNAVAILABLE)?,
            self.clone(),
            nonce,
        )?;
        window.navigate(url).map_err(|_| UNAVAILABLE)?;
        Ok(self.account(app))
    }
    fn resume_account_home(&self, app: &AppHandle, nonce: &str, epoch: u64, secure: tauri::Url) {
        let Some(window) = app.get_webview_window("soop-original-account") else {
            return;
        };
        let host = self.clone();
        let app = app.clone();
        let nonce = nonce.to_owned();
        // Leave NavigationStarting before entering another browser operation.
        std::thread::spawn(move || {
            let _ = app.run_on_main_thread(move || {
                if host.closing.load(Ordering::Acquire)
                    || !lock(&host.account_flow).can_resume(&nonce, epoch)
                {
                    return;
                }
                // Only a current official account document can resume its legacy home GET.
                if !window.url().ok().as_ref().is_some_and(policy::account_url) {
                    return;
                }
                let _ = window.navigate(secure);
            });
        });
    }
    fn finish_account_window(&self, app: &AppHandle, nonce: &str, epoch: u64) {
        if !lock(&self.account_flow).can_close(nonce, epoch) {
            return;
        }
        let Some(window) = app.get_webview_window("soop-original-account") else {
            return;
        };
        let host = self.clone();
        let app = app.clone();
        let nonce = nonce.to_owned();
        std::thread::spawn(move || {
            let _ = app.run_on_main_thread(move || {
                if !lock(&host.account_flow).can_close(&nonce, epoch) {
                    return;
                }
                if !window.url().ok().as_ref().is_some_and(policy::account_home) {
                    return;
                }
                // Normal native close also releases this window's associated DevTools.
                let _ = window.close();
            });
        });
    }
    fn sync_account(&self, app: &AppHandle, nonce: &str, revision: u64) {
        if !lock(&self.account_flow).accepts_refresh(nonce, revision) {
            return;
        }
        // Existing account flow remains the sole authority; VOD only refreshes its document.
        self.vod.refresh_account(app);
        let sessions = lock(&self.registry)
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            let current = lock(&self.account_flow);
            if !current.accepts_refresh(nonce, revision) {
                return;
            }
            let arm = lock(&session.arm);
            let archive = lock(&session.archive);
            if arm.is_some()
                || archive.is_some()
                || session.capture_busy.load(Ordering::Acquire)
                || session.closed.load(Ordering::Acquire)
            {
                session
                    .account_refresh_pending
                    .store(true, Ordering::Release);
                continue;
            }
            {
                session.account_refreshing.store(true, Ordering::Release);
                let mut status = lock(&session.status);
                status.capture_ready = false;
                status.authenticated = None;
                status.state = "loading".into();
            }
            drop(archive);
            drop(arm);
            drop(current);
            // Once committed under the flow/arm gate, finish this reload. Do not
            // strand the latch if a newer account intent arrives before dispatch.
            if !app
                .get_webview(&session.label)
                .is_some_and(|view| view.reload().is_ok())
            {
                let _approval = lock(&session.arm);
                session.account_refreshing.store(false, Ordering::Release);
                lock(&session.status).error = Some("계정 변경 후 방송을 다시 열어 주세요.".into());
            }
        }
    }
    pub fn shutdown(&self, app: &AppHandle) {
        self.closing.store(true, Ordering::Release);
        self.vod.detach(app);
        let sessions = lock(&self.registry)
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in &sessions {
            lock(&session.arm).take();
            if let Some(view) = app.get_webview(&session.label) {
                let _ = view.eval("window.__atsumiSoopOriginal?.stop()");
            }
        }
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until
            && sessions
                .iter()
                .any(|session| lock(&session.status).recording_job.is_some())
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        for session in sessions {
            lock(&session.arm).take();
            if let Some(archive) = lock(&session.archive).take() {
                let _ = archive.finish(Some("앱 종료로 원본 저장을 마무리했습니다.".into()));
            }
            self.dispose(app, &session);
        }
        if let Some(account) = app.get_webview_window("soop-original-account") {
            let _ = account.close();
        }
    }
    pub fn detach_views(&self, app: &AppHandle) {
        self.vod.detach(app);
        let sessions = {
            let mut registry = lock(&self.registry);
            registry.tokens.clear();
            registry.requests.clear();
            registry.sessions.values().cloned().collect::<Vec<_>>()
        };
        for session in sessions {
            let mut sequences = lock(&session.viewport_sequences);
            sequences.clear();
            lock(&session.viewers).clear();
            *lock(&session.viewport) = Some(hidden_viewport());
            if lock(&session.status).fullscreen {
                let _ = set_fullscreen(app, &session, false);
            }
            let revision = session.paint.fetch_add(1, Ordering::AcqRel) + 1;
            if let Some(view) = app.get_webview(&session.label) {
                let _ = platform_native_geometry::paint(
                    &view,
                    &hidden_viewport(),
                    session.paint.clone(),
                    revision,
                );
            }
            if lock(&session.archive).is_none() && lock(&session.arm).is_none() {
                self.dispose(app, &session);
            }
        }
    }
    pub fn set_tray_muted(&self, app: &AppHandle, muted: bool) {
        self.tray_muted.store(muted, Ordering::Release);
        self.vod.mute(app, muted);
        let sessions = lock(&self.registry)
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            if let Some(view) = app.get_webview(&session.label) {
                if let Some(mut viewport) = lock(&session.viewport).clone() {
                    viewport.muted |= muted;
                    if lock(&session.viewers).is_empty() {
                        viewport = hidden_viewport();
                    }
                    let revision = session.paint.fetch_add(1, Ordering::AcqRel) + 1;
                    let _ = platform_native_geometry::paint(
                        &view,
                        &viewport,
                        session.paint.clone(),
                        revision,
                    );
                } else {
                    let revision = session.paint.fetch_add(1, Ordering::AcqRel) + 1;
                    let _ = platform_native_geometry::paint(
                        &view,
                        &hidden_viewport(),
                        session.paint.clone(),
                        revision,
                    );
                }
            }
        }
    }
    fn create_view(&self, app: &AppHandle, session: &Arc<Session>) -> Result<(), String> {
        let parent = app.get_window("main").ok_or(UNAVAILABLE)?;
        let url = policy::player_url(&session.channel, &session.broadcast)?;
        let navigation = session.clone();
        let page = session.clone();
        let host = self.clone();
        let popup_app = app.clone();
        let page_app = app.clone();
        let focus_nonce = uuid::Uuid::new_v4().to_string();
        let builder=WebviewBuilder::new(&session.label,WebviewUrl::External("about:blank".parse().unwrap())).data_directory(profile(app)?).focused(false).browser_extensions_enabled(false)
            .initialization_script(platform_native_geometry::pointer_script(url.as_str(),&focus_nonce))
            .initialization_script(include_str!("original_capture.js")).initialization_script(include_str!("original_page.js"))
            .additional_browser_args("--autoplay-policy=no-user-gesture-required --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows")
            .on_navigation(move|next|next.as_str()=="about:blank"&&navigation.epoch.load(Ordering::Acquire)==0||policy::matches_player(next,&navigation.channel,&navigation.broadcast))
            .on_new_window(move|url,features|open_popup(&popup_app,url,features,0))
            .on_download(|_,_|false)
            .on_page_load(move|view,payload|{
                if !policy::matches_player(payload.url(),&page.channel,&page.broadcast){return;}
                if matches!(payload.event(),tauri::webview::PageLoadEvent::Started){
                    let _=set_fullscreen(&page_app,&page,false);
                    // Same lock order as account refresh and status retention.
                    let account_flow=lock(&host.account_flow);
                    let mut approval=lock(&page.arm);
                    let epoch=page.epoch.fetch_add(1,Ordering::AcqRel)+1;page.ui_intent.document(epoch);
                    page.account_revision.store(account_flow.observation_revision(),Ordering::Release);
                    // Detach old ownership before the new document can be configured.
                    // Only finalization is asynchronous; it cannot clear a later owner.
                    approval.take();
                    let old_archive=lock(&page.archive).take();
                    lock(&page.pending_begin).take();lock(&page.stop_requested).take();
                    {let mut status=lock(&page.status);status.state="loading".into();status.capture_ready=false;status.recording_job=None;status.recording_stopping=false;}
                    page.account_refreshing.store(false,Ordering::Release);
                    drop(approval);
                    drop(account_flow);
                    if let Some(archive)=old_archive {std::thread::spawn(move||{let _=archive.finish(Some("SOOP 원본 페이지가 변경되어 저장을 마무리했습니다.".into()));});}
                } else {
                    let config=json!({"epoch":page.epoch.load(Ordering::Acquire),"channel":page.channel});
                    let _=view.eval(&format!("window.__atsumiSoopOriginal?.configure({config})"));
                    let mut viewport=lock(&page.viewport).clone().unwrap_or_else(hidden_viewport);
                    if lock(&page.viewers).is_empty(){viewport=hidden_viewport();}
                    viewport.muted |= host.tray_muted.load(Ordering::Acquire);
                    let revision=page.paint.fetch_add(1,Ordering::AcqRel)+1;
                    let _=view.eval(&format!("window.__atsumiSoopOriginal?.layout({})",json!(viewport)));
                    let _=platform_native_geometry::paint(&view,&viewport,page.paint.clone(),revision);
                    let _=&page_app;
                }
            });
        let view = parent
            .add_child(
                builder,
                tauri::LogicalPosition::new(0.0, 0.0),
                tauri::LogicalSize::new(1.0, 1.0),
            )
            .map_err(|_| UNAVAILABLE)?;
        view.hide().map_err(|_| UNAVAILABLE)?;
        let initial_revision = session.paint.fetch_add(1, Ordering::AcqRel) + 1;
        platform_native_geometry::paint(
            &view,
            &hidden_viewport(),
            session.paint.clone(),
            initial_revision,
        )?;
        platform_native_geometry::install_pointer_focus(&view, url.to_string(), focus_nonce)
            .map_err(|_| UNAVAILABLE)?;
        attach_bridge(&view, session.clone(), self.clone())?;
        view.navigate(url).map_err(|_| UNAVAILABLE)?;
        Ok(())
    }
}

fn set_fullscreen(app: &AppHandle, session: &Arc<Session>, enabled: bool) -> Result<(), String> {
    if !enabled {
        let grant = lock(&session.fullscreen_grant).clone();
        if let Some(grant) = grant {
            window_fullscreen::release(app, grant)?;
        }
        return Ok(());
    }
    let epoch = session.epoch.load(Ordering::Acquire);
    let valid = Arc::downgrade(session);
    let acquired = Arc::downgrade(session);
    let lost = Arc::downgrade(session);
    let acquired_app = app.clone();
    let lost_app = app.clone();
    window_fullscreen::acquire(
        app,
        session.id.clone(),
        epoch,
        Arc::new(move || {
            valid.upgrade().is_some_and(|session| {
                !session.closed.load(Ordering::Acquire)
                    && session.epoch.load(Ordering::Acquire) == epoch
                    && !lock(&session.viewers).is_empty()
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
            let _ = window_fullscreen::release(app, grant);
        }
        error.message
    })?;
    Ok(())
}
fn publish_fullscreen(app: &AppHandle, session: &Session, grant: Grant, enabled: bool) {
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
        let _ = view.eval(&format!(
            "window.__atsumiSoopOriginal?.fullscreen({enabled})"
        ));
    }
    let _ = app.emit_to(
        "main",
        "soop-original:fullscreen",
        json!({"channelId": session.channel, "broadcastId": session.broadcast, "enabled": enabled}),
    );
}

fn open_popup(
    app: &AppHandle,
    url: tauri::Url,
    features: tauri::webview::NewWindowFeatures,
    depth: u8,
) -> tauri::webview::NewWindowResponse<tauri::Wry> {
    use tauri::webview::NewWindowResponse;
    if depth >= 3
        || !(policy::popup_url(&url) || url.as_str() == "about:blank")
        || app
            .webview_windows()
            .keys()
            .filter(|key| key.starts_with("soop-original-popup-"))
            .count()
            >= 6
    {
        return NewWindowResponse::Deny;
    }
    let Ok(profile) = profile(app) else {
        return NewWindowResponse::Deny;
    };
    let child = app.clone();
    let initial = Arc::new(AtomicBool::new(true));
    let result = WebviewWindowBuilder::new(
        app,
        format!("soop-original-popup-{}", uuid::Uuid::new_v4().simple()),
        WebviewUrl::External("about:blank".parse().unwrap()),
    )
    .data_directory(profile)
    .browser_extensions_enabled(false)
    .window_features(features)
    .focused(false)
    .title("SOOP")
    .inner_size(720.0, 780.0)
    .min_inner_size(360.0, 420.0)
    .on_navigation(move |next| {
        if next.as_str() == "about:blank" {
            return initial.load(Ordering::Acquire);
        }
        let allowed = policy::popup_url(next);
        if allowed {
            initial.store(false, Ordering::Release);
        }
        allowed
    })
    .on_new_window(move |url, features| open_popup(&child, url, features, depth + 1))
    .on_download(|_, _| false)
    .build();
    match result {
        Ok(window) => NewWindowResponse::Create { window },
        Err(_) => NewWindowResponse::Deny,
    }
}
#[cfg(windows)]
fn attach_account_bridge(
    view: &Webview,
    host: SoopOriginalHost,
    nonce: String,
) -> Result<(), String> {
    use webview2_com::{CoTaskMemPWSTR, WebMessageReceivedEventHandler};
    use windows::core::PWSTR;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AccountMessage {
        nonce: String,
        epoch: u64,
        authenticated: bool,
    }
    let app = view.app_handle().clone();
    view.with_webview(move |platform| unsafe {
        let Ok(core) = platform.controller().CoreWebView2() else {
            return;
        };
        let mut subscription = 0;
        let _ = core.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                let mut source = PWSTR::null();
                args.Source(&mut source)?;
                let source = CoTaskMemPWSTR::from(source).to_string();
                let Some(window) = app.get_webview_window("soop-original-account") else {
                    return Ok(());
                };
                if !window.url().ok().as_ref().is_some_and(|url| {
                    policy::account_observation_matches(&source, url)
                }) {
                    return Ok(());
                }
                let mut body = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut body).is_err() {
                    return Ok(());
                }
                let body = CoTaskMemPWSTR::from(body).to_string();
                let Some(raw) = body
                    .strip_prefix("ATSUMI_SOOP_ACCOUNT_V1:")
                    .filter(|value| value.len() <= 512)
                else {
                    return Ok(());
                };
                let Ok(message) = serde_json::from_str::<AccountMessage>(raw) else {
                    return Ok(());
                };
                if message.nonce != nonce {
                    return Ok(());
                }
                let revision = {
                    let mut flow = lock(&host.account_flow);
                    let previous = flow.observation_revision();
                    if !flow.observe(&nonce, message.epoch, message.authenticated) {
                        return Ok(());
                    }
                    // Commit in validated arrival order; a delayed refresh worker never writes this.
                    *lock(&host.account_state) = Some(message.authenticated);
                    (flow.observation_revision() != previous).then_some(flow.observation_revision())
                };
                // The bounded home observation may arrive after PageLoad Finished.
                host.finish_account_window(&app, &nonce, message.epoch);
                let Some(revision) = revision else { return Ok(()) };
                let host = host.clone();
                let app = app.clone();
                let observed_nonce = nonce.clone();
                std::thread::spawn(move || host.sync_account(&app, &observed_nonce, revision));
                Ok(())
            })),
            &mut subscription,
        );
    })
    .map_err(|_| UNAVAILABLE)?;
    Ok(())
}
#[cfg(not(windows))]
fn attach_account_bridge(_: &Webview, _: SoopOriginalHost, _: String) -> Result<(), String> {
    Err(UNAVAILABLE.into())
}
fn response(view: &Webview, id: &str, result: Result<Value, String>) {
    let body = match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => {
            json!({"ok":false,"code":if error=="SOOP 저장 요청이 많습니다."{"BRIDGE_BUSY"}else{"NATIVE_REJECTED"},"error":error})
        }
    };
    let _ = view.eval(&format!(
        "window.__atsumiSoopOriginal?.reply({},{})",
        json!(id),
        body
    ));
}
fn hidden_viewport() -> NativeViewport {
    NativeViewport {
        sequence: 1,
        player: None,
        chat: None,
        visible: false,
        muted: true,
        occlusions: Vec::new(),
    }
}
fn process(host: &SoopOriginalHost, session: &Arc<Session>, message: Envelope) -> Result<Value, String> {
    if session.closed.load(Ordering::Acquire)
        || message.epoch != session.epoch.load(Ordering::Acquire)
    {
        return Err(STALE.into());
    }
    let fields = &message.fields;
    match message.kind.as_str() {
        "status" => {
            // Serialize retained observations with explicit login/logout results.
            // Never take this lock after the arm/status locks (refresh uses flow first).
            let account_flow = lock(&host.account_flow);
            let approval = lock(&session.arm);
            if !current_status(
                &approval,
                &session.closed,
                &session.account_refreshing,
                &session.epoch,
                message.epoch,
            ) {
                return Ok(Value::Null);
            }
            let mut status = lock(&session.status);
            if let Some(state) = fields["state"]
                .as_str()
                .filter(|s| matches!(*s, "preparing" | "loading" | "playing" | "ended" | "error"))
            {
                status.state = state.into();
            }
            status.authenticated = fields["authenticated"].as_bool();
            account_flow.retain_player(
                session.account_revision.load(Ordering::Acquire),
                session.account_refresh_pending.load(Ordering::Acquire),
                status.authenticated,
                &mut lock(&host.account_state),
            );
            status.chat_visible = fields["chatVisible"].as_bool().unwrap_or(false);
            status.capture_ready = fields["captureReady"].as_bool().unwrap_or(false)
                && fields["actualChannel"].as_str() == Some(&session.channel)
                && fields["actualBroadcast"].as_str() == Some(&session.broadcast);
            if fields["error"].is_string() {
                status.error = Some("SOOP 원본 플레이어 또는 저장 상태를 확인해 주세요.".into());
            }
            Ok(Value::Null)
        }
        "encoded_begin" => {
            if fields["actualChannel"].as_str() != Some(&session.channel)
                || fields["actualBroadcast"].as_str() != Some(&session.broadcast)
            {
                return Err("재생 중인 방송이 변경되었습니다.".into());
            }
            let request = fields["requestId"].as_str().ok_or(STALE)?;
            let source = fields["sourceId"]
                .as_str()
                .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                .ok_or(STALE)?;
            let mut arm = lock(&session.arm);
            let (approval, owner) = arm.as_ref().ok_or("녹화 시작 승인이 없습니다.")?;
            if !approval.accepts(request, message.epoch, Instant::now())
                || fields["channelId"].as_str() != Some(&session.channel)
            {
                return Err(STALE.into());
            }
            let tracks = fields["tracks"]
                .as_array()
                .filter(|v| !v.is_empty() && v.len() <= 2)
                .ok_or(STALE)?
                .iter()
                .map(|track| {
                    let init = track["init"]
                        .as_str()
                        .filter(|v| v.len() <= 88_000)
                        .ok_or(STALE)?;
                    Ok(EncodedTrack {
                        track_index: track["trackIndex"]
                            .as_u64()
                            .filter(|i| *i < 2)
                            .ok_or(STALE)? as u32,
                        mime_type: track["mimeType"]
                            .as_str()
                            .filter(|s| s.len() <= 120)
                            .ok_or(STALE)?
                            .into(),
                        init: STANDARD.decode(init).map_err(|_| STALE)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut slot = lock(&session.archive);
            if slot.is_some() {
                return Err("이미 녹화 중입니다.".into());
            }
            let archive = session.lease.begin_encoded(
                source.into(),
                message.epoch,
                request.into(),
                tracks,
                session.title.clone(),
                session.name.clone(),
                owner.clone(),
            )?;
            let id = archive.snapshot()?.job_id;
            *slot = Some(archive);
            *lock(&session.pending_begin) = Some(Instant::now());
            lock(&session.stop_requested).take();
            *lock(&session.source) = Some(source.into());
            *lock(&session.owner) = Some(owner.clone());
            arm.take();
            lock(&session.status).recording_job = Some(id.clone());
            Ok(json!({"id":id,"mode":"encoded","nativeApproved":true,"captureChat":true}))
        }
        "encoded_append" => {
            if fields["actualChannel"].as_str() != Some(&session.channel)
                || fields["actualBroadcast"].as_str() != Some(&session.broadcast)
            {
                return Err("재생 중인 방송이 변경되었습니다.".into());
            }
            let mut slot = lock(&session.archive);
            let archive = slot.as_mut().ok_or(STALE)?;
            let recording = fields["recordingId"].as_str().ok_or(STALE)?;
            if lock(&session.status).recording_job.as_deref() != Some(recording) {
                return Err(STALE.into());
            }
            let payload = fields["data"]
                .as_str()
                .filter(|s| s.len() <= 174_764)
                .ok_or(STALE)?;
            // Source is pinned by begin; the current archive validates exact generation and source.
            let source = lock(&session.source).clone().ok_or(STALE)?;
            archive.append(EncodedAppend {
                source_id: source,
                generation: message.epoch,
                track_index: fields["trackIndex"]
                    .as_u64()
                    .filter(|v| *v < 2)
                    .ok_or(STALE)? as u32,
                index: fields["appendIndex"].as_u64().ok_or(STALE)?,
                chunk: fields["chunkIndex"]
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or(STALE)?,
                final_chunk: fields["finalChunk"].as_bool().ok_or(STALE)?,
                bytes: STANDARD.decode(payload).map_err(|_| STALE)?,
            })?;
            // The first accepted byte proves the browser received begin's ACK.
            // Never time out a normally running recording using this deadline.
            lock(&session.pending_begin).take();
            Ok(json!({"accepted":true}))
        }
        "encoded_finish" => {
            if fields["recordingId"].as_str() != lock(&session.status).recording_job.as_deref() {
                return Err(STALE.into());
            }
            let archive = lock(&session.archive).take().ok_or(STALE)?;
            lock(&session.pending_begin).take();
            lock(&session.stop_requested).take();
            let mut interrupted = fields["interrupted"].as_bool().unwrap_or(true);
            if interrupted
                && matches!(
                    fields["reason"].as_str(),
                    Some("video_ended" | "source_changed")
                )
            {
                if let Ok(info) = super::observe_channel(&session.channel) {
                    if info.status == "offline"
                        || info
                            .broadcast_key
                            .as_ref()
                            .is_some_and(|id| id != &session.broadcast)
                    {
                        interrupted = false;
                    }
                }
            }
            let result = archive
                .finish(interrupted.then(|| "원본 영상 수신 또는 저장이 중단되었습니다.".into()));
            let mut status = lock(&session.status);
            status.recording_job = None;
            status.recording_stopping = false;
            result?;
            Ok(json!({"stopped":true,"interrupted":interrupted}))
        }
        _ => Err("지원하지 않는 SOOP 요청입니다.".into()),
    }
}
#[cfg(windows)]
fn attach_bridge(
    view: &Webview,
    session: Arc<Session>,
    host: SoopOriginalHost,
) -> Result<(), String> {
    use webview2_com::{CoTaskMemPWSTR, WebMessageReceivedEventHandler};
    use windows::core::PWSTR;
    let (tx, rx) = std::sync::mpsc::sync_channel::<Envelope>(4);
    let worker_view = view.clone();
    let worker_session = session.clone();
    let app = view.app_handle().clone();
    std::thread::Builder::new()
        .name("soop-original-writer".into())
        .spawn(move || loop {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(message) => {
                    let id = message.id.clone();
                    let silent = matches!(message.kind.as_str(), "status" | "presentation" | "fullscreen" | "screenshot_shortcut");
                    if message.kind=="ui_intent" {let result=dispatch_ui(&host,&app,&worker_session,&message);if let Some(action)=ui_action_category(message.fields["action"].as_str()){ui_diagnostic(&worker_session,"soop_live_dispatch",action,result.is_ok());}if let Err(error)=&result{lock(&worker_session.status).error=Some(error.clone());}response(&worker_view,&id,result);continue;}
                    if matches!(message.kind.as_str(), "fullscreen" | "screenshot_shortcut") {
                        if message.epoch == worker_session.epoch.load(Ordering::Acquire) && !worker_session.closed.load(Ordering::Acquire)
                            && !host.tray_muted.load(Ordering::Acquire) && !lock(&worker_session.viewers).is_empty()
                            && lock(&worker_session.viewport).as_ref().is_some_and(|v|v.visible && v.player.is_some()) {
                            if message.kind == "fullscreen" {
                                if let Some(enabled) = message.fields["enabled"].as_bool() { let _ = set_fullscreen(&app, &worker_session, enabled); }
                            } else { let _ = app.emit_to("main", "soop-original:screenshot", json!({"channelId":worker_session.channel,"broadcastId":worker_session.broadcast})); }
                        }
                        continue;
                    }
                    if message.kind=="presentation" {
                        if message.epoch==worker_session.epoch.load(Ordering::Acquire) {
                            if let Some(opacity)=message.fields["opacity"].as_f64().filter(|v|v.is_finite()&&(0.0..=1.0).contains(v)) {
                                let _=app.emit_to("main","soop-original:presentation",json!({"channelId":worker_session.channel,"broadcastId":worker_session.broadcast,"opacity":opacity}));
                            }
                        }
                        continue;
                    }
                    let result = process(&host, &worker_session, message);
                    if let Err(error)=&result{if !silent{lock(&worker_session.status).error=Some(error.clone());}}
                    if !silent {
                        response(&worker_view, &id, result);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            {
                let mut slot = lock(&worker_session.archive);
                let timeout = capture_timeout(*lock(&worker_session.pending_begin), *lock(&worker_session.stop_requested), Instant::now());
                if slot.is_some() && timeout.is_some() {
                    // A lost begin ACK leaves no browser active capture to stop.
                    // Drain only this pending owner, retain fragments, never mark success.
                    let _ = worker_view.eval("window.__atsumiSoopOriginal?.stop()");
                    if let Some(archive) = slot.take() { let _ = archive.finish(Some(timeout.unwrap().into())); }
                    lock(&worker_session.pending_begin).take();lock(&worker_session.stop_requested).take();
                    let mut status = lock(&worker_session.status);status.recording_job=None;status.recording_stopping=false;status.error=timeout.map(str::to_string);
                }
                if slot.as_ref().is_some_and(|archive| archive.is_cancelled()) {
                    if let Some(archive) = slot.take() {
                        let _ = archive.finish(Some("녹화 취소 요청으로 마무리했습니다.".into()));
                    }
                    let mut status = lock(&worker_session.status);
                    status.recording_job = None;
                    status.recording_stopping = false;
                    lock(&worker_session.pending_begin).take();lock(&worker_session.stop_requested).take();
                }
            }
            {
                let mut arm = lock(&worker_session.arm);
                if arm
                    .as_ref()
                    .is_some_and(|(arm, _)| arm.created.elapsed() > Duration::from_secs(25))
                {
                    arm.take();
                    lock(&worker_session.status).error =
                        Some("원본 영상 준비 시간이 초과되었습니다.".into());
                }
            }
            if worker_session.account_refresh_pending.load(Ordering::Acquire)
                && !lock(&worker_session.viewers).is_empty() && !worker_session.capture_busy.load(Ordering::Acquire) {
                let arm=lock(&worker_session.arm);let archive=lock(&worker_session.archive);
                if arm.is_none()&&archive.is_none()&&worker_session.account_refresh_pending.swap(false,Ordering::AcqRel){
                    worker_session.account_refreshing.store(true,Ordering::Release);
                    {let mut status=lock(&worker_session.status);status.capture_ready=false;status.authenticated=None;status.state="loading".into();}
                    drop(archive);drop(arm);
                    if worker_view.reload().is_err(){
                        let _approval=lock(&worker_session.arm);
                        worker_session.account_refreshing.store(false,Ordering::Release);
                        lock(&worker_session.status).error=Some("계정 변경 후 방송을 다시 열어 주세요.".into());
                    }
                }
            }
            if !worker_session.opening.load(Ordering::Acquire) && lock(&worker_session.viewers).is_empty()
                && lock(&worker_session.archive).is_none()
                && lock(&worker_session.arm).is_none()
            {
                host.dispose(&app, &worker_session);
                break;
            }
            if worker_session.closed.load(Ordering::Acquire) {
                break;
            }
        })
        .map_err(|_| UNAVAILABLE)?;
    let native_view = view.clone();
    let root_hwnd = view.window().hwnd().map_err(|_| UNAVAILABLE)?.0 as usize;
    view.with_webview(move |platform| unsafe {
        if let Ok(core) = platform.controller().CoreWebView2() {
            let mut pane_hwnd = windows::Win32::Foundation::HWND::default();
            if platform.controller().ParentWindow(&mut pane_hwnd).is_err() {
                return;
            }
            let pane_hwnd = pane_hwnd.0 as usize;
            let weak=Arc::downgrade(&session);
            let _=player_intent::install(&platform.controller(),root_hwnd,pane_hwnd,session.ui_intent.clone(),Arc::new(move||weak.upgrade().and_then(|s|ui_binding(&s))));
            let mut token = 0;
            let _ = core.add_WebMessageReceived(
                &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut source = PWSTR::null();
                    args.Source(&mut source)?;
                    let source = CoTaskMemPWSTR::from(source).to_string();
                    let mut body = PWSTR::null();
                    if args.TryGetWebMessageAsString(&mut body).is_err() {
                        return Ok(());
                    }
                    let body = CoTaskMemPWSTR::from(body).to_string();
                    let Some(raw) = body
                        .strip_prefix(policy::PREFIX)
                        .filter(|v| v.len() <= policy::MAX_MESSAGE)
                    else {
                        return Ok(());
                    };
                    let Ok(mut message) = serde_json::from_str::<Envelope>(raw) else {
                        return Ok(());
                    };
                    if !tauri::Url::parse(&source).ok().as_ref().is_some_and(|url| {
                        policy::matches_player(url, &session.channel, &session.broadcast)
                    }) || uuid::Uuid::parse_str(&message.id).is_err()
                        || !policy::validate_envelope(
                            &source,
                            native_view.label(),
                            &session.label,
                            &session.channel,
                            message.epoch,
                            session.epoch.load(Ordering::Acquire),
                            raw,
                        )
                    {
                        return Ok(());
                    }
                    // Diagnostic-only category: cannot enter the action/recording queue.
                    if message.kind=="ui_pointer" {if let Some(category)=pointer_category(&message.fields){ui_diagnostic(&session,"soop_live_pointer",category,true);}return Ok(());}
                    if message.kind=="ui_intent" {
                        if message.fields["input"].as_str()!=Some("pointer")||message.fields["actualChannel"].as_str()!=Some(session.channel.as_str())||message.fields["actualBroadcast"].as_str()!=Some(session.broadcast.as_str())||!message.fields.as_object().is_some_and(|v|v.keys().all(|k|["action","input","actualChannel","actualBroadcast"].contains(&k.as_str())))||!matches!(message.fields["action"].as_str(),Some("record_start"|"record_stop"|"screenshot"|"viewer_close")){return Ok(());}
                        let observed=player_intent::consume_diagnosed(&session.ui_intent,ui_binding(&session).as_ref(),root_hwnd,pane_hwnd);
                        ui_diagnostic(&session,"soop_live_receipt",observed.as_ref().err().map(|reason|reason.diagnostic()).unwrap_or("accepted"),observed.is_ok());
                        message.intent=observed.ok();
                        if message.intent.is_none(){response(&native_view,&message.id,Err("버튼을 다시 눌러 주세요.".into()));return Ok(());}
                    }
                    if matches!(message.kind.as_str(), "fullscreen" | "screenshot_shortcut") {
                        let key = match (message.kind.as_str(), message.fields["input"].as_str()) {
                            ("fullscreen", Some("pointer")) => 1,
                            ("fullscreen", Some("f")) => 70,
                            ("fullscreen", Some("escape")) => 27,
                            ("screenshot_shortcut", Some("s")) => 83,
                            _ => return Ok(()),
                        };
                        if !platform_native_geometry::trusted_input(root_hwnd, pane_hwnd, key) {
                            return Ok(());
                        }
                        let mut last = lock(&session.last_input);
                        if last.is_some_and(|time| time.elapsed() < Duration::from_millis(300)) {
                            return Ok(());
                        }
                        *last = Some(Instant::now());
                    }
                    if let Err(std::sync::mpsc::TrySendError::Full(message)) = tx.try_send(message)
                    {
                        if !matches!(message.kind.as_str(), "status" | "presentation") {
                            response(
                                &native_view,
                                &message.id,
                                Err("SOOP 저장 요청이 많습니다.".into()),
                            );
                        }
                    }
                    Ok(())
                })),
                &mut token,
            );
        }
    })
    .map_err(|_| UNAVAILABLE)?;
    Ok(())
}
fn ui_binding(session:&Session)->Option<player_intent::Binding>{
    if session.closed.load(Ordering::Acquire)||session.account_refreshing.load(Ordering::Acquire){return None;}
    let binding=session.ui_intent.current(session.epoch.load(Ordering::Acquire))?;
    session.viewers.try_lock().ok()?.contains(&binding.token).then_some(binding)
}
fn dispatch_ui(host:&SoopOriginalHost,app:&AppHandle,session:&Arc<Session>,message:&Envelope)->Result<Value,String>{
    let binding=message.intent.as_ref().ok_or(STALE)?;
    if host.tray_muted.load(Ordering::Acquire)||ui_binding(session).as_ref()!=Some(binding){return Err(STALE.into());}
    let action=message.fields["action"].as_str().ok_or(STALE)?;
    match action{
        "record_start"|"record_stop"=>{
            let _sequence=lock(&session.viewport_sequences);
            if ui_binding(session).as_ref()!=Some(binding){return Err(STALE.into());}
            let snapshot=app.state::<crate::interface::commands::AppState>().begin_platform_work(||host.record(app,&binding.token,action=="record_start"))?;
            Ok(json!(snapshot))
        }
        "screenshot"=>{app.emit_to("main","soop-original:action",json!({"token":binding.token,"action":"screenshot"})).map_err(|_|UNAVAILABLE)?;Ok(Value::Null)}
        "viewer_close" if session.close_enabled.load(Ordering::Acquire)=>{
            host.close(app,&binding.token)?;
            app.emit_to("main","soop-original:action",json!({"token":binding.token,"action":"viewer_close"})).map_err(|_|UNAVAILABLE)?;
            Ok(Value::Null)
        }
        _=>Err(STALE.into())
    }
}
fn capture_timeout(
    pending_begin: Option<Instant>,
    stop: Option<Instant>,
    now: Instant,
) -> Option<&'static str> {
    if stop.is_some_and(|time| now.saturating_duration_since(time) >= Duration::from_secs(8)) {
        Some("원본 플레이어의 녹화 종료 응답이 지연되어 저장된 조각을 보존했습니다.")
    } else if pending_begin
        .is_some_and(|time| now.saturating_duration_since(time) >= Duration::from_secs(25))
    {
        Some("원본 플레이어의 녹화 시작 응답을 확인하지 못했습니다.")
    } else {
        None
    }
}
fn accept_viewport_sequence(
    sequences: &mut HashMap<String, u64>,
    token: &str,
    sequence: u64,
) -> bool {
    if sequence == 0
        || sequences
            .get(token)
            .is_some_and(|previous| sequence <= *previous)
    {
        return false;
    }
    sequences.insert(token.into(), sequence);
    true
}
#[cfg(not(windows))]
fn attach_bridge(_: &Webview, _: Arc<Session>, _: SoopOriginalHost) -> Result<(), String> {
    Err("SOOP 원본 플레이어는 Windows에서 사용할 수 있습니다.".into())
}
#[cfg(test)]
mod tests {
    #[test]
    fn live_ui_diagnostics_are_closed_and_share_one_saturating_budget(){
        use super::*;
        let budget=AtomicU64::new(0);for _ in 0..48{assert!(reserve_ui_diagnostic(&budget));}for _ in 0..10{assert!(!reserve_ui_diagnostic(&budget));}assert_eq!(budget.load(Ordering::Acquire),48);
        for value in ["record","camera","close","player_other"]{assert_eq!(pointer_category(&json!({"target":value})),Some(value));}
        for value in [json!({"target":"secret"}),json!({"target":"camera","token":"secret"}),json!({}),json!({"target":null})]{assert!(pointer_category(&value).is_none());}
        assert_eq!(ui_action_category(Some("screenshot")),Some("screenshot"));assert_eq!(ui_action_category(Some("secret")),None);
    }
    #[test]
    fn retained_account_survives_viewer_close_and_requires_fresh_document_on_reopen() {
        use std::sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Mutex};
        let gate = Mutex::new(None::<()>);
        let epoch = AtomicU64::new(1);
        let closed = AtomicBool::new(false);
        let refreshing = AtomicBool::new(false);
        let flow = super::policy::AccountFlow::default();
        let mut retained = None;
        let approval = super::lock(&gate);
        assert!(super::current_status(&approval, &closed, &refreshing, &epoch, 1));
        assert!(flow.retain_player(0, false, Some(true), &mut retained));
        closed.store(true, Ordering::Release);
        assert!(!super::current_status(&approval, &closed, &refreshing, &epoch, 1));
        assert_eq!(retained, Some(true));
        // Reopening has a new document epoch. A queued prior status remains invalid.
        closed.store(false, Ordering::Release);
        epoch.store(2, Ordering::Release);
        assert!(!super::current_status(&approval, &closed, &refreshing, &epoch, 1));
        refreshing.store(true, Ordering::Release);
        assert!(!super::current_status(&approval, &closed, &refreshing, &epoch, 2));
        assert_eq!(retained, Some(true));
        refreshing.store(false, Ordering::Release);
        assert!(super::current_status(&approval, &closed, &refreshing, &epoch, 2));
        assert!(flow.retain_player(0, false, Some(false), &mut retained));
        assert_eq!(retained, Some(false));
    }
    #[test]
    fn queued_player_account_status_cannot_pass_a_newer_logout_flow_gate() {
        use std::sync::{atomic::{AtomicBool, AtomicU64}, mpsc, Arc, Mutex};
        let flow = Arc::new(Mutex::new(super::policy::AccountFlow::default()));
        let arm = Arc::new(Mutex::new(None::<()>));
        let retained = Arc::new(Mutex::new(None));
        let mut account = super::lock(&flow);
        let old_revision = account.observation_revision();
        assert!(account.retain_player(old_revision, false, Some(true), &mut super::lock(&retained)));
        let (queued, receiving) = mpsc::channel();
        let (worker_flow, worker_arm, worker_retained) = (flow.clone(), arm.clone(), retained.clone());
        let worker = std::thread::spawn(move || {
            queued.send(()).unwrap();
            let account = super::lock(&worker_flow);
            let approval = super::lock(&worker_arm);
            assert!(super::current_status(&approval, &AtomicBool::new(false), &AtomicBool::new(false), &AtomicU64::new(1), 1));
            // Document is still loaded, and async refresh has not marked it yet.
            let accepted = account.retain_player(old_revision, false, Some(true), &mut super::lock(&worker_retained));
            accepted
        });
        receiving.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        account.begin("logout".into(), false);
        let epoch = account.current("logout").unwrap();
        assert!(account.observe("logout", epoch, false));
        *super::lock(&retained) = Some(false);
        drop(account);
        assert!(!worker.join().unwrap());
        assert_eq!(*super::lock(&retained), Some(false));
    }
    #[test]
    fn recording_admission_rechecks_readiness_after_waiting_for_account_refresh() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc, Mutex,
        };
        let approval = Arc::new(Mutex::new(None::<()>));
        let ready = Arc::new(AtomicBool::new(true));
        let closed = Arc::new(AtomicBool::new(false));
        let refreshing = Arc::new(AtomicBool::new(false));
        let refresh = super::lock(&approval);
        let (observed, receiver) = mpsc::channel();
        let worker_approval = approval.clone();
        let worker_ready = ready.clone();
        let worker_closed = closed.clone();
        let worker_refreshing = refreshing.clone();
        let worker = std::thread::spawn(move || {
            assert!(worker_ready.load(Ordering::Acquire));
            observed.send(()).unwrap();
            super::recording_approval(&worker_approval, &worker_closed, &worker_refreshing, || {
                worker_ready.load(Ordering::Acquire)
            })
            .is_err()
        });
        receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        refreshing.store(true, Ordering::Release);
        ready.store(false, Ordering::Release);
        drop(refresh);
        assert!(
            worker.join().unwrap(),
            "old readiness must not approve a recording during reload"
        );
        ready.store(true, Ordering::Release);
        assert!(
            super::recording_approval(&approval, &closed, &refreshing, || ready
                .load(Ordering::Acquire))
            .is_err(),
            "old status true cannot bypass refresh latch"
        );
        refreshing.store(false, Ordering::Release);
        ready.store(false, Ordering::Release);
        assert!(
            super::recording_approval(&approval, &closed, &refreshing, || ready
                .load(Ordering::Acquire))
            .is_err()
        );
        ready.store(true, Ordering::Release);
        assert!(
            super::recording_approval(&approval, &closed, &refreshing, || ready
                .load(Ordering::Acquire))
            .is_ok()
        );
        closed.store(true, Ordering::Release);
        assert!(
            super::recording_approval(&approval, &closed, &refreshing, || ready
                .load(Ordering::Acquire))
            .is_err()
        );
    }
    #[test]
    fn old_status_waiting_for_new_document_gate_cannot_restore_readiness() {
        use std::sync::{
            atomic::{AtomicBool, AtomicU64, Ordering},
            mpsc, Arc, Mutex,
        };
        let gate = Arc::new(Mutex::new(None::<()>));
        let epoch = Arc::new(AtomicU64::new(1));
        let closed = Arc::new(AtomicBool::new(false));
        let refreshing = Arc::new(AtomicBool::new(true));
        let page_start = super::lock(&gate);
        let (tx, rx) = mpsc::channel();
        let (g, e, c, r) = (
            gate.clone(),
            epoch.clone(),
            closed.clone(),
            refreshing.clone(),
        );
        let worker = std::thread::spawn(move || {
            assert_eq!(e.load(Ordering::Acquire), 1);
            tx.send(()).unwrap();
            let approval = super::lock(&g);
            super::current_status(&approval, &c, &r, &e, 1)
        });
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        epoch.store(2, Ordering::Release);
        refreshing.store(false, Ordering::Release);
        drop(page_start);
        assert!(!worker.join().unwrap());
        let approval = super::lock(&gate);
        assert!(super::current_status(
            &approval,
            &closed,
            &refreshing,
            &epoch,
            2
        ));
        refreshing.store(true, Ordering::Release);
        assert!(!super::current_status(
            &approval,
            &closed,
            &refreshing,
            &epoch,
            2
        ));
    }
    #[test]
    fn stale_visible_cannot_overwrite_new_hidden_and_viewers_have_independent_sequences() {
        let mut sequences = std::collections::HashMap::new();
        let mut hidden = true;
        assert!(super::accept_viewport_sequence(
            &mut sequences,
            "viewer-a",
            2
        ));
        if super::accept_viewport_sequence(&mut sequences, "viewer-a", 1) {
            hidden = false;
        }
        assert!(hidden);
        assert!(!super::accept_viewport_sequence(
            &mut sequences,
            "viewer-a",
            2
        ));
        assert!(super::accept_viewport_sequence(
            &mut sequences,
            "viewer-b",
            1
        ));
        assert!(super::accept_viewport_sequence(
            &mut sequences,
            "viewer-a",
            3
        ));
        assert!(!super::accept_viewport_sequence(
            &mut sequences,
            "viewer-b",
            0
        ));
    }
    #[test]
    fn missing_begin_ack_is_bounded_but_an_acknowledged_recording_is_not() {
        let start = std::time::Instant::now();
        assert!(super::capture_timeout(
            Some(start),
            None,
            start + std::time::Duration::from_secs(24)
        )
        .is_none());
        assert!(super::capture_timeout(
            Some(start),
            None,
            start + std::time::Duration::from_secs(25)
        )
        .is_some());
        // First accepted append clears pending_begin, irrespective of elapsed time.
        assert!(
            super::capture_timeout(None, None, start + std::time::Duration::from_secs(3600))
                .is_none()
        );
        assert!(super::capture_timeout(
            None,
            Some(start),
            start + std::time::Duration::from_secs(8)
        )
        .is_some());
    }
    #[test]
    fn hidden_receiver_viewport_is_valid_and_muted() {
        let viewport = super::hidden_viewport();
        assert!(viewport.validate().is_ok());
        assert!(!viewport.visible);
        assert!(viewport.muted);
        assert!(viewport.player.is_none() && viewport.chat.is_none());
    }
}
