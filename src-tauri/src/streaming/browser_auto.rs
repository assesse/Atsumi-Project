//! Persistent opt-in scheduling. Metadata polling never downloads media. The
//! official page owns access checks; existing capture owns files/chat/merging.
use super::super::model::{LiveInfo, LiveStatus};
use super::*;
use std::collections::HashMap;
use tauri::Emitter;

const FILE: &str = "chzzk-auto-record.json";
const MAX_CHANNELS: usize = 32;
const POLL_MS: u64 = 30_000;

fn start_retry_delay(attempts: u8, normal: u64) -> u64 {
    // A temporary receiver/storage fault must not disable an opted-in broadcast
    // forever. After five failures retry slowly (5/10/20/30 min), not in bursts.
    if attempts < 5 {
        normal
    } else {
        (300_000u64 << (attempts - 5).min(3)).min(1_800_000)
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    channel_id: String,
    channel_name: String,
    enabled: bool,
    #[serde(default)]
    suppressed_live: Option<String>,
    #[serde(default)]
    hold_until_offline: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_failure: Option<StartFailure>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartFailure {
    occurred_at: u64,
    live_key: String,
    stage: String,
    message: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    version: u32,
    capture_chat: bool,
    channels: Vec<Channel>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            capture_chat: true,
            channels: Vec::new(),
        }
    }
}
#[derive(Clone, Default)]
struct Observation {
    info: Option<LiveInfo>,
    checked_at: u64,
    next_at: u64,
    offline_count: u8,
    error: Option<String>,
}
impl Observation {
    fn accept(&mut self, result: Result<LiveInfo, StreamError>, now: u64) {
        self.checked_at = now;
        self.next_at = now.saturating_add(POLL_MS);
        match result {
            Ok(info) => {
                self.offline_count = if info.status == LiveStatus::Offline {
                    self.offline_count.saturating_add(1)
                } else {
                    0
                };
                self.info = Some(info);
                self.error = None;
            }
            Err(cause) => {
                self.error = Some(cause.message);
                self.offline_count = 0;
            }
        }
    }
    fn live_key(&self) -> Option<String> {
        let info = self
            .info
            .as_ref()
            .filter(|info| info.status == LiveStatus::Live)?;
        info.live_id
            .as_ref()
            .filter(|id| !id.is_empty())
            .map(|id| format!("id:{id}"))
            .or_else(|| info.broadcast_started_at.map(|at| format!("start:{at}")))
    }
    fn can_start(&self, now: u64) -> bool {
        self.error.is_none()
            && now.saturating_sub(self.checked_at) <= 90_000
            && self.live_key().is_some()
    }
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    status: String,
    recording_id: Option<String>,
    message: Option<String>,
}
#[derive(Default)]
struct Core {
    config: Config,
    revision: u64,
    observations: HashMap<String, Observation>,
    progress: HashMap<String, Progress>,
    fault: Option<String>,
    history_error: Option<String>,
}
pub(super) struct AutoRecorder {
    directory: PathBuf,
    core: Mutex<Core>,
    started: AtomicBool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    channel_id: String,
    channel_name: String,
    enabled: bool,
    checked_at: u64,
    last_failure: Option<StartFailure>,
    #[serde(flatten)]
    progress: Progress,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    channels: Vec<Entry>,
    capture_chat: bool,
    error: Option<String>,
}

fn config_error() -> StreamError {
    error(
        "AUTO_RECORD_CONFIG",
        "자동 녹화 설정을 저장하지 못했습니다. 기존 설정은 유지됩니다.",
    )
}
fn validate(config: &Config) -> Result<(), StreamError> {
    let mut ids = std::collections::HashSet::new();
    if config.version != 1 || config.channels.len() > MAX_CHANNELS {
        return Err(config_error());
    }
    for channel in &config.channels {
        if normalize_channel_input(&channel.channel_id).ok().as_deref() != Some(&channel.channel_id)
            || !ids.insert(&channel.channel_id)
            || channel.channel_name.chars().count() > 200
            || channel
                .suppressed_live
                .as_ref()
                .is_some_and(|key| key.len() > 160)
            || channel.last_failure.as_ref().is_some_and(|failure| {
                failure.live_key.chars().count() > 160
                    || failure.stage.chars().count() > 80
                    || failure.message.chars().count() > 500
            })
        {
            return Err(config_error());
        }
    }
    Ok(())
}
fn suppressed(channel: &Channel, key: &str) -> bool {
    channel.hold_until_offline || channel.suppressed_live.as_deref() == Some(key)
}
impl AutoRecorder {
    pub fn load(directory: &Path) -> Self {
        let mut core = Core::default();
        let path = directory.join(FILE);
        let read = (|| {
            if std::fs::metadata(&path).map_err(|_| config_error())?.len() > 256 * 1024 {
                return Err(config_error());
            }
            let bytes = std::fs::read(&path).map_err(|_| config_error())?;
            let config: Config = serde_json::from_slice(&bytes).map_err(|_| config_error())?;
            validate(&config)?;
            Ok(config)
        })();
        match read {
            Ok(config) => core.config = config,
            Err(_) if !path.exists() => {}
            Err(_) => {
                core.fault = Some(
                    "자동 녹화 설정을 읽지 못했습니다. 파일을 보존하고 자동 녹화를 중지했습니다."
                        .into(),
                )
            }
        }
        Self {
            directory: directory.into(),
            core: Mutex::new(core),
            started: AtomicBool::new(false),
        }
    }
    fn edit(
        &self,
        change: impl FnOnce(&mut Config, &HashMap<String, Observation>),
    ) -> Result<(), StreamError> {
        let mut core = self.core.lock().map_err(|_| config_error())?;
        if core.fault.is_some() {
            return Err(config_error());
        }
        let mut config = core.config.clone();
        change(&mut config, &core.observations);
        validate(&config)?;
        super::super::browser_store::atomic_write(
            &self.directory,
            FILE,
            &serde_json::to_vec(&config).map_err(|_| config_error())?,
        )?;
        core.config = config;
        core.history_error = None;
        core.revision = core.revision.wrapping_add(1);
        let ids: std::collections::HashSet<_> = core
            .config
            .channels
            .iter()
            .map(|c| c.channel_id.clone())
            .collect();
        core.observations.retain(|id, _| ids.contains(id));
        core.progress.retain(|id, _| ids.contains(id));
        Ok(())
    }
    pub fn capture_chat(&self) -> bool {
        self.core.lock().map_or(true, |c| c.config.capture_chat)
    }
    pub(super) fn watch_channel_name(&self, id: &str, recording_id: &str) -> Option<String> {
        let core = self.core.lock().ok()?;
        let channel = core
            .config
            .channels
            .iter()
            .find(|channel| channel.channel_id == id)?;
        let progress = core.progress.get(id)?;
        (progress.status == "recording" && progress.recording_id.as_deref() == Some(recording_id))
            .then(|| channel.channel_name.clone())
    }
    pub fn set_capture_chat(&self, enabled: bool) -> Result<(), StreamError> {
        self.edit(|config, _| config.capture_chat = enabled)
    }
    fn add(&self, id: String, name: String) -> Result<(), StreamError> {
        self.edit(|config, _| {
            if let Some(channel) = config
                .channels
                .iter_mut()
                .find(|channel| channel.channel_id == id)
            {
                channel.enabled = true;
                channel.channel_name = name;
                channel.suppressed_live = None;
                channel.hold_until_offline = false;
            } else {
                config.channels.push(Channel {
                    channel_id: id,
                    channel_name: name,
                    enabled: true,
                    suppressed_live: None,
                    hold_until_offline: false,
                    last_failure: None,
                });
            }
        })
    }
    pub fn suppress(&self, channel_id: &str) -> Result<(), StreamError> {
        if !self
            .core
            .lock()
            .map_err(|_| config_error())?
            .config
            .channels
            .iter()
            .any(|c| c.channel_id == channel_id)
        {
            return Ok(());
        }
        let result = self.edit(|config, observations| {
            if let Some(channel) = config
                .channels
                .iter_mut()
                .find(|c| c.channel_id == channel_id)
            {
                let key = observations.get(channel_id).and_then(Observation::live_key);
                channel.hold_until_offline = key.is_none();
                channel.suppressed_live = key;
            }
        });
        if result.is_err() {
            // Disk-full/permissions must never trap a running recorder. Keep
            // the stop fence in memory even when persistence is unavailable.
            let mut core = self.core.lock().map_err(|_| config_error())?;
            let key = core
                .observations
                .get(channel_id)
                .and_then(Observation::live_key);
            if let Some(channel) = core
                .config
                .channels
                .iter_mut()
                .find(|c| c.channel_id == channel_id)
            {
                channel.hold_until_offline = key.is_none();
                channel.suppressed_live = key;
            }
        }
        result
    }
    fn update(&self, id: &str, action: Change) -> Result<(), StreamError> {
        if matches!(action, Change::Stop) {
            return self.suppress(id);
        }
        self.edit(|config, _| match action {
            Change::Remove => config.channels.retain(|c| c.channel_id != id),
            Change::Enabled { enabled } => {
                if let Some(c) = config.channels.iter_mut().find(|c| c.channel_id == id) {
                    c.enabled = enabled;
                    if enabled {
                        c.suppressed_live = None;
                        c.hold_until_offline = false;
                    }
                }
            }
            Change::Stop => {}
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        let core = self.core.lock().unwrap_or_else(|p| p.into_inner());
        Snapshot {
            capture_chat: core.config.capture_chat,
            error: core.fault.clone().or_else(|| core.history_error.clone()),
            channels: core
                .config
                .channels
                .iter()
                .map(|c| {
                    let observed = core.observations.get(&c.channel_id);
                    Entry {
                        channel_id: c.channel_id.clone(),
                        channel_name: c.channel_name.clone(),
                        enabled: c.enabled,
                        checked_at: observed.map_or(0, |o| o.checked_at),
                        last_failure: c.last_failure.clone(),
                        progress: core
                            .progress
                            .get(&c.channel_id)
                            .cloned()
                            .unwrap_or_else(|| Progress {
                                status: if c.enabled { "waiting" } else { "disabled" }.into(),
                                ..Default::default()
                            }),
                    }
                })
                .collect(),
        }
    }
    fn publish(
        &self,
        id: &str,
        status: &str,
        recording_id: Option<String>,
        message: Option<String>,
    ) {
        let mut core = self.core.lock().unwrap_or_else(|p| p.into_inner());
        // Preparation/retries are not a successful start. Only an accepted
        // recording (including an existing receiver) retires the old failure.
        if status == "recording" && recording_id.as_ref().is_some_and(|id| !id.is_empty()) {
            if let Some(channel) = core.config.channels.iter_mut().find(|c| c.channel_id == id) {
                if channel.last_failure.take().is_some() {
                    let saved = core.fault.is_none()
                        && validate(&core.config).is_ok()
                        && serde_json::to_vec(&core.config).ok().is_some_and(|bytes| {
                            super::super::browser_store::atomic_write(&self.directory, FILE, &bytes)
                                .is_ok()
                        });
                    // Do not show an obsolete start failure as a current error,
                    // or stop recording if persisting its removal fails.
                    core.history_error = (!saved).then(|| "이전 녹화 시작 실패 기록의 정리를 저장하지 못했습니다. 현재 녹화는 계속됩니다.".into());
                }
            }
        }
        core.progress.insert(
            id.into(),
            Progress {
                status: status.into(),
                recording_id,
                message,
            },
        );
    }
    fn allowed(&self, id: &str, key: &str) -> bool {
        self.core.lock().is_ok_and(|c| {
            c.fault.is_none()
                && c.config
                    .channels
                    .iter()
                    .any(|c| c.channel_id == id && c.enabled && !suppressed(c, key))
        })
    }

    pub(super) fn observed_live_key(&self, channel: &str) -> Option<String> {
        self.core.lock().ok()?.observations.get(channel)?.live_key()
    }

    /// Retain the first failure through stop/offline/restart until recording starts.
    /// Retries must not write the same message or show a toast every 90 seconds.
    fn remember_failure(&self, id: &str, live_key: &str, stage: &str, message: &str) -> bool {
        let mut core = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let Some(index) = core.config.channels.iter().position(|c| c.channel_id == id) else {
            return false;
        };
        if core.config.channels[index]
            .last_failure
            .as_ref()
            .is_some_and(|failure| failure.live_key == live_key)
        {
            return false;
        }
        let failure = StartFailure {
            occurred_at: now_ms(),
            live_key: live_key.chars().take(160).collect(),
            stage: stage.chars().take(80).collect(),
            message: message.chars().take(500).collect(),
        };
        let mut config = core.config.clone();
        config.channels[index].last_failure = Some(failure.clone());
        let saved = core.fault.is_none()
            && validate(&config).is_ok()
            && serde_json::to_vec(&config).ok().is_some_and(|bytes| {
                super::super::browser_store::atomic_write(&self.directory, FILE, &bytes).is_ok()
            });
        // A diagnostic write failure must not erase the stop fence, block stop,
        // or silently overwrite a damaged configuration on a later retry.
        core.config.channels[index].last_failure = Some(failure);
        core.history_error = (!saved).then(|| "자동 녹화 실패 기록을 디스크에 보관하지 못했습니다. 현재 화면에서 먼저 확인해 주세요.".into());
        true
    }
}

fn preparation_problem(diagnostics: Option<&Value>) -> &'static str {
    match diagnostics.and_then(|value| value["reason"].as_str()) {
        Some("waiting_video") => "재생 가능한 영상을 받지 못해 저장을 시작하지 못했습니다. 라이브에서 재생·로그인·연결 안내를 확인해 주세요.",
        Some("encrypted") => "보호된 영상이므로 원본 녹화를 시작하지 않았습니다.",
        Some("source_not_observed" | "source_object_unsupported" | "mse_unavailable") => "이 플레이어의 원본 영상 수신 경로를 확인하지 못해 저장을 시작하지 못했습니다.",
        Some("waiting_init" | "waiting_tracks" | "buffering") => "영상·음성 데이터 준비가 끝나지 않아 저장을 시작하지 못했습니다.",
        Some("ready") => "녹화 시작 요청이 완료되지 않아 저장을 시작하지 못했습니다.",
        _ => "플레이어 또는 원본 영상 준비에 실패해 저장을 시작하지 못했습니다. 라이브에서 재생 상태를 확인해 주세요.",
    }
}

struct Session {
    host: OfficialBrowser,
    owns_view: bool,
    owns_recording: bool,
    live_key: String,
    started: Instant,
    stop_at: Option<Instant>,
    attempted: bool,
    recording_id: Option<String>,
    request_id: Option<String>,
}
impl OfficialBrowser {
    fn report_auto_start_failure(&self, id: &str, live_key: &str, stage: &str, message: &str) {
        if self
            .inner
            .auto_record
            .remember_failure(id, live_key, stage, message)
        {
            if stage == "cancelled_before_start" {
                tracing::info!(
                    channel_id = id,
                    live_key,
                    stage,
                    "automatic recording cancelled before start"
                );
                return;
            }
            tracing::warn!(
                channel_id = id,
                live_key,
                stage,
                "automatic recording did not start"
            );
            self.inner.contexts.notice("자동 녹화 시작에 실패했습니다. 이번 시도의 영상은 저장되지 않았으며, 자동 녹화 탭에서 이유를 확인할 수 있습니다.");
        }
    }
    /// Called once after AppState is managed. Neither worker depends on the UI
    /// tab, renderer refresh, or main-window visibility.
    pub fn start_auto_recording(&self, app: &AppHandle) -> Result<(), StreamError> {
        if self.inner.auto_record.started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let polling = self.clone();
        thread::Builder::new()
            .name("chzzk-auto-metadata".into())
            .spawn(move || polling.poll_auto_metadata())
            .map_err(|_| unavailable())?;
        let controller = self.clone();
        let app = app.clone();
        thread::Builder::new()
            .name("chzzk-auto-record".into())
            .spawn(move || controller.run_auto_recording(&app))
            .map_err(|_| unavailable())?;
        Ok(())
    }
    fn poll_auto_metadata(&self) {
        let provider = match ChzzkProvider::new() {
            Ok(p) => p,
            Err(cause) => {
                self.inner
                    .auto_record
                    .core
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .fault = Some(cause.message);
                return;
            }
        };
        while !self.inner.closing.load(Ordering::Acquire) {
            let task = {
                let mut core = self
                    .inner
                    .auto_record
                    .core
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                let now = now_ms();
                let id = core
                    .config
                    .channels
                    .iter()
                    .filter(|c| c.enabled)
                    .filter(|c| {
                        core.observations
                            .get(&c.channel_id)
                            .is_none_or(|o| o.next_at <= now)
                    })
                    .min_by_key(|c| {
                        core.observations
                            .get(&c.channel_id)
                            .map_or(0, |o| o.next_at)
                    })
                    .map(|c| c.channel_id.clone());
                id.map(|id| {
                    core.observations.entry(id.clone()).or_default().next_at = now + POLL_MS;
                    (id, core.revision)
                })
            };
            if let Some((id, revision)) = task {
                let response = provider.inspect_for_browser(&id);
                if self.inner.closing.load(Ordering::Acquire) {
                    break;
                }
                let clear_hold = {
                    let mut core = self
                        .inner
                        .auto_record
                        .core
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    if core.revision != revision {
                        continue;
                    }
                    let observation = core.observations.entry(id.clone()).or_default();
                    observation.accept(response, now_ms());
                    observation.offline_count >= 2
                        && core.config.channels.iter().any(|c| {
                            c.channel_id == id
                                && (c.hold_until_offline || c.suppressed_live.is_some())
                        })
                };
                if clear_hold {
                    let _ = self.inner.auto_record.edit(|config, _| {
                        if let Some(c) = config.channels.iter_mut().find(|c| c.channel_id == id) {
                            c.hold_until_offline = false;
                            c.suppressed_live = None;
                        }
                    });
                }
            } else {
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
    fn auto_target(
        &self,
        app: &AppHandle,
        channel: &str,
    ) -> Result<(Self, bool, bool), StreamError> {
        // Reuse an existing receiver, including manually started recordings.
        for host in self.inner.contexts.hosts() {
            let state = host.inner.view.lock().map_err(|_| unavailable())?;
            if !host.inner.detached.load(Ordering::Acquire)
                && !host.multiview_active()
                && state.open
                && state.channel.as_deref() == Some(channel)
            {
                let active = state.recording.is_some() || state.arm.is_some();
                drop(state);
                let automatic = host.label().starts_with("chzzk-auto-");
                if automatic {
                    self.claim_auto_receiver(host.label());
                }
                return Ok((host, automatic, !active));
            }
        }
        if self.active_ids().len() >= 4
            || self.inner.contexts.reconfiguring.load(Ordering::Acquire) != 0
            || self.primary_profile_busy()
        {
            return Err(unavailable());
        }
        // Do not change the selected channel, reveal a window or play sound.
        // Existing visible receivers above are reused; otherwise use a silent
        // dedicated official receiver independent of the current tab.
        self.open_auto_view(app, channel)
            .map(|host| (host, true, true))
    }
    #[cfg(windows)]
    pub(super) fn restore_auto_extensions(&self, view: &Webview) -> Result<(), StreamError> {
        let root = self.capture_context(Some(WINDOW_LABEL))?;
        {
            let _gate = self.inner.contexts.gate.lock().map_err(|_| unavailable())?;
            let active = !root.active_ids().is_empty();
            let mut state = root.inner.view.lock().map_err(|_| unavailable())?;
            if !state.extension_reconnect_enabled || !state.loaded_extensions.is_empty() {
                return Ok(());
            }
            if active
                || state.account_busy
                || state.extension_connecting
                || root.account_window_open(view.app_handle())
            {
                return Err(unavailable());
            }
            state.extension_connecting = true;
        }
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let completed = root.clone();
        let result = super::super::browser_extension::reconnect_background(
            view,
            &self.inner.data_dir,
            move |result| {
                let mut state = completed
                    .inner
                    .view
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                if let Ok(report) = &result {
                    state.loaded_extensions = report.loaded_ids.clone();
                }
                state.extension_connecting = false;
                drop(state);
                let _ = send.send(result);
            },
        );
        if result.is_err() {
            root.inner
                .view
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .extension_connecting = false;
        }
        result?;
        // A timeout does not release the profile reservation; the actual COM
        // callback does. Late profile mutations cannot race a login/recording.
        receive
            .recv_timeout(Duration::from_secs(20))
            .map_err(|_| unavailable())??;
        Ok(())
    }
    fn run_auto_recording(&self, app: &AppHandle) {
        let auto = &self.inner.auto_record;
        let mut sessions: HashMap<String, Session> = HashMap::new();
        let mut retry: HashMap<String, u64> = HashMap::new();
        let mut attempts: HashMap<(String, String), u8> = HashMap::new();
        let mut configuration = 0;
        while !self.inner.closing.load(Ordering::Acquire) {
            for notice in self.inner.contexts.take_notices() {
                if !self.show_recording_notice(app, &notice) {
                    let _ = app.emit_to("main", "chzzk-recording:notice", notice);
                }
            }
            let (channels, observations, revision) = {
                let core = auto.core.lock().unwrap_or_else(|p| p.into_inner());
                (
                    core.config.channels.clone(),
                    core.observations.clone(),
                    core.revision,
                )
            };
            if configuration != revision {
                configuration = revision;
                attempts.clear();
            }
            attempts.retain(|(id, key), _| {
                let same = observations
                    .get(id)
                    .and_then(Observation::live_key)
                    .as_ref()
                    == Some(key);
                if !same {
                    retry.remove(id);
                }
                same
            });
            retry.retain(|id, _| channels.iter().any(|c| &c.channel_id == id));
            let mut completed = Vec::new();
            for (id, session) in &mut sessions {
                let observed = observations.get(id).cloned().unwrap_or_default();
                let ended = observed.offline_count >= 2
                    || observed.live_key().is_some_and(|key| {
                        key.split(':').next() == session.live_key.split(':').next()
                            && key != session.live_key
                    });
                let should_stop = !auto.allowed(id, &session.live_key) || ended;
                let (ready, active, current_id, status, problem, request_id, accepted_id) = {
                    let mut s = session
                        .host
                        .inner
                        .view
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    if s.arm
                        .as_ref()
                        .is_some_and(|a| a.created.elapsed() > Duration::from_secs(20))
                    {
                        s.arm = None;
                        s.status = "error".into();
                    }
                    (
                        s.ready,
                        s.recording.is_some() || s.arm.is_some(),
                        s.recording.clone(),
                        s.status.clone(),
                        s.error.clone(),
                        s.arm
                            .as_ref()
                            .map(|a| a.id.clone())
                            .or_else(|| s.accepted_arm.as_ref().map(|a| a.0.clone())),
                        accepted_session_recording(&s, session.request_id.as_deref()),
                    )
                };
                let same_channel = session
                    .host
                    .inner
                    .view
                    .lock()
                    .is_ok_and(|s| s.channel.as_deref() == Some(id));
                if !same_channel {
                    completed.push(id.clone());
                    retry.insert(id.clone(), now_ms() + 30_000);
                    continue;
                }
                // A user can stop then immediately start a NEW recording in a
                // borrowed player. Never stop/adopt that replacement session.
                if session_replaced(
                    session.recording_id.as_deref(),
                    current_id.as_deref(),
                    session.request_id.as_deref(),
                    request_id.as_deref(),
                    active,
                ) {
                    completed.push(id.clone());
                    continue;
                }
                if let Some(current) = current_id.clone().or(accepted_id) {
                    session.recording_id = Some(current);
                }
                if active && !session.attempted {
                    session.owns_recording = false;
                }
                if should_stop || session.stop_at.is_some() {
                    if session.owns_recording && active && session.stop_at.is_none() {
                        if let Some(view) = app.get_webview(session.host.label()) {
                            let reason = if ended {
                                if observed.offline_count >= 2 {
                                    "broadcast_ended"
                                } else {
                                    "broadcast_changed"
                                }
                            } else {
                                "user_stop"
                            };
                            let _ = send_command(
                                &view,
                                json!({"kind":"stop","channelId":id,"reason":reason}),
                            );
                        }
                        session.stop_at = Some(Instant::now());
                    }
                    if !session.owns_recording
                        || !active
                        || session
                            .stop_at
                            .is_some_and(|at| at.elapsed() > Duration::from_secs(12))
                    {
                        if session.owns_recording && active {
                            session.host.interrupt_matching(
                                "window_closed",
                                session.recording_id.as_deref(),
                            );
                        }
                        // Cancel/flush first. A begin ACK can race the snapshot
                        // above; never claim "no file" for an accepted recording.
                        let accepted = session.host.inner.view.lock().ok().is_some_and(|state| {
                            state.recording.is_some()
                                || accepted_session_recording(&state, session.request_id.as_deref())
                                    .is_some()
                        });
                        if session.owns_recording && session.recording_id.is_none() && !accepted {
                            self.report_auto_start_failure(id, &session.live_key,
                                if ended { "ended_before_start" } else { "cancelled_before_start" },
                                if ended { "영상 저장을 시작하기 전에 방송이 종료되거나 회차가 바뀌었습니다. 이 시도에서는 영상이 저장되지 않았습니다." }
                                else { "영상 저장을 시작하기 전에 이번 방송의 녹화 시도를 중지했습니다. 이 시도에서는 영상이 저장되지 않았습니다." });
                        }
                        completed.push(id.clone());
                    } else {
                        auto.publish(id, "stopping", current_id, None);
                    }
                    continue;
                }
                if active {
                    if current_id.is_some() && session.started.elapsed() > Duration::from_secs(120)
                    {
                        attempts.remove(&(id.clone(), session.live_key.clone()));
                    }
                    auto.publish(
                        id,
                        if status == "stopping" {
                            "stopping"
                        } else if current_id.is_some() {
                            "recording"
                        } else {
                            "starting"
                        },
                        current_id,
                        None,
                    );
                    continue;
                }
                if session.attempted || session.recording_id.is_some() || !session.owns_recording {
                    let failed = status == "error"
                        || problem.is_some()
                        || (session.attempted && session.recording_id.is_none());
                    if failed && session.recording_id.is_none() {
                        self.report_auto_start_failure(id, &session.live_key, "recording_start", "녹화 시작 요청이 완료되지 않았습니다. 저장된 영상이 없으므로 라이브에서 재생 상태를 확인해 주세요.");
                    }
                    auto.publish(
                        id,
                        if failed { "retry" } else { "waiting" },
                        None,
                        if failed {
                            problem.or(Some("연결을 확인한 뒤 다시 시도합니다.".into()))
                        } else {
                            None
                        },
                    );
                    let count = attempts
                        .get(&(id.clone(), session.live_key.clone()))
                        .copied()
                        .unwrap_or(0);
                    retry.insert(
                        id.clone(),
                        now_ms()
                            + if failed {
                                start_retry_delay(count, 60_000)
                            } else {
                                POLL_MS
                            },
                    );
                    completed.push(id.clone());
                    continue;
                }
                if ready
                    && auto.allowed(id, &session.live_key)
                    && !self.inner.reserved.load(Ordering::Acquire)
                {
                    session.attempted = true;
                    let result = app.state::<AppState>().start_browser_managed_checked(
                        app,
                        Some(session.host.label()),
                        true,
                        auto.capture_chat(),
                        Some(id),
                    );
                    if result.is_ok() {
                        session.request_id = session.host.inner.view.lock().ok().and_then(|s| {
                            s.arm
                                .as_ref()
                                .map(|a| a.id.clone())
                                .or_else(|| s.accepted_arm.as_ref().map(|a| a.0.clone()))
                        });
                    }
                    if let Err(cause) = result {
                        self.report_auto_start_failure(id, &session.live_key, &cause.code, "녹화 시작 요청이 거부되어 영상을 저장하지 못했습니다. 라이브의 연결 상태와 저장 공간을 확인해 주세요.");
                        auto.publish(id, "retry", None, Some(cause.message));
                        let count = attempts
                            .get(&(id.clone(), session.live_key.clone()))
                            .copied()
                            .unwrap_or(0);
                        retry.insert(id.clone(), now_ms() + start_retry_delay(count, 60_000));
                        completed.push(id.clone());
                    }
                } else if session.started.elapsed() > Duration::from_secs(90) {
                    let state = session
                        .host
                        .inner
                        .view
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    let message = preparation_problem(state.capture_diagnostics.as_ref());
                    tracing::warn!(channel_id = id, live_key = %session.live_key, diagnostics = ?state.capture_diagnostics, "automatic receiver prepare timeout");
                    drop(state);
                    self.report_auto_start_failure(
                        id,
                        &session.live_key,
                        "player_prepare",
                        message,
                    );
                    auto.publish(id, "retry", None, Some(message.into()));
                    let count = attempts
                        .get(&(id.clone(), session.live_key.clone()))
                        .copied()
                        .unwrap_or(0);
                    retry.insert(id.clone(), now_ms() + start_retry_delay(count, 120_000));
                    completed.push(id.clone());
                } else {
                    auto.publish(id, "starting", None, None);
                }
            }
            for id in completed {
                if let Some(session) = sessions.remove(&id) {
                    if session.owns_view {
                        self.retire_auto_receiver(session.host.label());
                    }
                }
            }
            self.reap_retired_auto_receivers(app);
            for channel in channels {
                let id = &channel.channel_id;
                if sessions.contains_key(id) {
                    continue;
                }
                let observed = observations.get(id).cloned().unwrap_or_default();
                let key = observed.live_key();
                // A user may explicitly start another recording in the shared
                // live player after suppressing automatic restart. Keep that
                // existing capture discoverable/watchable without re-enabling
                // the rule or taking ownership of the manual recording.
                let already_recording = self.inner.contexts.hosts().into_iter().find_map(|host| {
                    let s = host.inner.view.lock().ok()?;
                    (!host.inner.detached.load(Ordering::Acquire)
                        && s.channel.as_deref() == Some(id)
                        && (s.recording.is_some() || s.arm.is_some()))
                    .then(|| (s.recording.clone(), s.status.clone()))
                });
                if let Some((recording_id, status)) = already_recording {
                    auto.publish(
                        id,
                        if status == "stopping" {
                            "stopping"
                        } else if recording_id.is_some() {
                            "recording"
                        } else {
                            "starting"
                        },
                        recording_id,
                        None,
                    );
                    continue;
                }
                if !channel.enabled {
                    auto.publish(id, "disabled", None, None);
                    continue;
                }
                if key.as_deref().is_some_and(|key| suppressed(&channel, key))
                    || channel.hold_until_offline
                {
                    auto.publish(id, "skipped", None, None);
                    continue;
                }
                if retry.get(id).is_some_and(|at| *at > now_ms()) {
                    continue;
                }
                if !observed.can_start(now_ms()) {
                    auto.publish(
                        id,
                        if observed.error.is_some() {
                            "retry"
                        } else {
                            "waiting"
                        },
                        None,
                        observed.error.clone(),
                    );
                    continue;
                }
                if self.inner.closing.load(Ordering::Acquire)
                    || self.inner.reserved.load(Ordering::Acquire)
                {
                    continue;
                }
                if sessions.len() >= 4 || self.active_ids().len() >= 4 {
                    auto.publish(id, "queued", None, None);
                    continue;
                }
                let attempt_key = (id.clone(), key.clone().unwrap());
                let checking_end = self
                    .inner
                    .store
                    .lock()
                    .is_ok_and(|store| store.checking_end(id, key.as_deref(), now_ms()));
                if checking_end {
                    auto.publish(id, "ending", None, None);
                    continue;
                }
                let count = attempts.entry(attempt_key).or_default();
                *count = count.saturating_add(1);
                let attempt_count = *count;
                match self.auto_target(app, id) {
                    Ok((host, owns_view, owns_recording)) => {
                        sessions.insert(
                            id.clone(),
                            Session {
                                host,
                                owns_view,
                                owns_recording,
                                live_key: key.unwrap(),
                                started: Instant::now(),
                                stop_at: None,
                                attempted: false,
                                recording_id: None,
                                request_id: None,
                            },
                        );
                    }
                    Err(cause) => {
                        self.report_auto_start_failure(id, key.as_deref().unwrap_or("unknown"), "receiver_create", "자동 녹화용 재생 창을 만들지 못해 저장을 시작하지 못했습니다. 자동 재시도 후에도 계속 실패하면 앱 오류 정보를 확인해 주세요.");
                        auto.publish(id, "retry", None, Some(cause.message));
                        retry.insert(
                            id.clone(),
                            now_ms() + start_retry_delay(attempt_count, 30_000),
                        );
                    }
                }
            }
            thread::sleep(Duration::from_millis(500));
        }
        // Shutdown owns stop/flush/join across ALL contexts. Do not detach here:
        // it would discard the renderer's final original-data/chat messages.
    }
}

/// A short recording may finish between scheduler ticks. Its accepted start
/// token is durable in ViewState until a new start; do not call it a failed start.
fn accepted_session_recording(state: &ViewState, request_id: Option<&str>) -> Option<String> {
    let (nonce, id, generation) = state.accepted_arm.as_ref()?;
    (request_id == Some(nonce.as_str()) && *generation == state.page_generation).then(|| id.clone())
}

fn session_replaced(
    old_id: Option<&str>,
    current_id: Option<&str>,
    old_request: Option<&str>,
    current_request: Option<&str>,
    active: bool,
) -> bool {
    active
        && ((old_id.is_some() && current_id.is_some() && old_id != current_id)
            || (old_request.is_some()
                && current_request.is_some()
                && old_request != current_request))
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    Enabled { enabled: bool },
    Remove,
    Stop,
}
#[tauri::command]
pub async fn chzzk_auto_record_snapshot(app: AppHandle, window: Webview) -> ApiResult<Snapshot> {
    (|| {
        require_main(&window)?;
        Ok::<_, StreamError>(host(&app)?.inner.auto_record.snapshot())
    })()
    .into()
}
#[tauri::command]
pub async fn chzzk_auto_record_add(
    app: AppHandle,
    window: Webview,
    input: String,
) -> ApiResult<Snapshot> {
    let prepared = (|| {
        require_main(&window)?;
        Ok::<_, StreamError>((host(&app)?, normalize_channel_input(&input)?))
    })();
    let (host, id) = match prepared {
        Ok(value) => value,
        Err(cause) => return Err::<Snapshot, _>(cause).into(),
    };
    match tauri::async_runtime::spawn_blocking(move || {
        let (name, _) = ChzzkProvider::new()?.channel_profile(&id)?;
        host.inner.auto_record.add(id, name)?;
        Ok::<_, StreamError>(host.inner.auto_record.snapshot())
    })
    .await
    {
        Ok(result) => result.into(),
        Err(_) => Err::<Snapshot, _>(unavailable()).into(),
    }
}
#[tauri::command]
pub async fn chzzk_auto_record_update(
    app: AppHandle,
    window: Webview,
    channel_id: String,
    change: Change,
) -> ApiResult<Snapshot> {
    (|| {
        require_main(&window)?;
        let host = host(&app)?;
        let stop = matches!(change, Change::Stop);
        let channel_id = normalize_channel_input(&channel_id)?;
        if stop {
            host.remember_manual_stop(&channel_id);
        } else {
            host.inner.auto_record.update(&channel_id, change)?;
        }
        if stop {
            for target in host.inner.contexts.hosts() {
                let matching = target.inner.view.lock().is_ok_and(|s| {
                    s.channel.as_deref() == Some(&channel_id)
                        && (s.recording.is_some() || s.arm.is_some())
                });
                if matching {
                    let pending = target
                        .request_control(ControlAction::RecordStop)?
                        .pending_control
                        .ok_or_else(control_stale)?;
                    target.confirm_control(&app, &pending.id, true, true, true)?;
                }
            }
        }
        Ok::<_, StreamError>(host.inner.auto_record.snapshot())
    })()
    .into()
}
#[tauri::command]
pub async fn chzzk_browser_capture_chat(
    app: AppHandle,
    window: Webview,
    enabled: bool,
) -> ApiResult<BrowserSnapshot> {
    (|| {
        require_main(&window)?;
        let host = host(&app)?;
        host.inner.auto_record.set_capture_chat(enabled)?;
        host.snapshot()
    })()
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_start_failures_back_off_without_permanently_disabling_the_broadcast() {
        assert_eq!(start_retry_delay(1, 30_000), 30_000);
        assert_eq!(start_retry_delay(4, 120_000), 120_000);
        assert_eq!(start_retry_delay(5, 30_000), 300_000);
        assert_eq!(start_retry_delay(6, 30_000), 600_000);
        assert_eq!(start_retry_delay(7, 30_000), 1_200_000);
        assert_eq!(start_retry_delay(u8::MAX, 30_000), 1_800_000);
    }
    const CHANNEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fn live(id: &str, status: LiveStatus) -> LiveInfo {
        LiveInfo {
            channel_id: CHANNEL.into(),
            channel_name: "방송".into(),
            title: String::new(),
            live_id: Some(id.into()),
            broadcast_started_at: None,
            status,
            chat_available: true,
            chat_channel_id: None,
            notice: None,
        }
    }
    #[test]
    fn registrations_chat_preference_and_stop_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let auto = AutoRecorder::load(dir.path());
        auto.add(CHANNEL.into(), "방송".into()).unwrap();
        auto.set_capture_chat(false).unwrap();
        let mut observed = Observation::default();
        observed.accept(Ok(live("42", LiveStatus::Live)), 100);
        auto.core
            .lock()
            .unwrap()
            .observations
            .insert(CHANNEL.into(), observed);
        auto.suppress(CHANNEL).unwrap();
        let loaded = AutoRecorder::load(dir.path());
        assert!(!loaded.capture_chat());
        assert!(!loaded.allowed(CHANNEL, "id:42"));
        assert!(loaded.allowed(CHANNEL, "id:43"));
        assert_eq!(loaded.snapshot().channels.len(), 1);
    }
    #[test]
    fn watch_lookup_requires_the_exact_registered_recording_without_changing_config() {
        let dir = tempfile::tempdir().unwrap();
        let auto = AutoRecorder::load(dir.path());
        auto.add(CHANNEL.into(), "방송".into()).unwrap();
        let before = std::fs::read(dir.path().join(FILE)).unwrap();
        for status in ["waiting", "starting", "stopping", "retry"] {
            auto.publish(CHANNEL, status, Some("recording".into()), None);
            assert!(auto.watch_channel_name(CHANNEL, "recording").is_none());
        }
        auto.publish(CHANNEL, "recording", Some("recording".into()), None);
        assert_eq!(
            auto.watch_channel_name(CHANNEL, "recording").as_deref(),
            Some("방송")
        );
        assert!(auto.watch_channel_name(CHANNEL, "old-recording").is_none());
        assert!(auto
            .watch_channel_name("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "recording")
            .is_none());
        assert_eq!(std::fs::read(dir.path().join(FILE)).unwrap(), before);
    }
    #[test]
    fn preparation_failure_survives_stop_offline_and_restart_without_rearming() {
        let dir = tempfile::tempdir().unwrap();
        let auto = AutoRecorder::load(dir.path());
        auto.add(CHANNEL.into(), "test".into()).unwrap();
        auto.set_capture_chat(false).unwrap();
        let mut observed = Observation::default();
        observed.accept(Ok(live("42", LiveStatus::Live)), 100);
        auto.core
            .lock()
            .unwrap()
            .observations
            .insert(CHANNEL.into(), observed);
        let message = preparation_problem(Some(&json!({"reason":"waiting_video"})));
        assert!(auto.remember_failure(CHANNEL, "id:42", "player_prepare", message));
        auto.suppress(CHANNEL).unwrap();
        auto.core
            .lock()
            .unwrap()
            .observations
            .get_mut(CHANNEL)
            .unwrap()
            .accept(Ok(live("42", LiveStatus::Offline)), 200);
        auto.publish(CHANNEL, "waiting", None, None);
        let loaded = AutoRecorder::load(dir.path());
        assert!(!loaded.allowed(CHANNEL, "id:42"));
        assert!(loaded.allowed(CHANNEL, "id:43"));
        assert!(!loaded.capture_chat());
        let snapshot = loaded.snapshot();
        let failure = snapshot.channels[0].last_failure.as_ref().unwrap();
        assert_eq!(failure.live_key, "id:42");
        assert_eq!(failure.stage, "player_prepare");
        assert_eq!(failure.message, message);
        assert!(failure.occurred_at > 0);
        assert!(!dir.path().join("streaming").exists());
    }
    #[test]
    fn repeated_failure_keeps_first_cause_and_new_broadcast_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let auto = AutoRecorder::load(dir.path());
        auto.add(CHANNEL.into(), "test".into()).unwrap();
        assert!(auto.remember_failure(CHANNEL, "id:42", "player_prepare", "first cause"));
        let bytes = std::fs::read(dir.path().join(FILE)).unwrap();
        assert!(!auto.remember_failure(CHANNEL, "id:42", "cancelled_before_start", "cancelled"));
        assert_eq!(std::fs::read(dir.path().join(FILE)).unwrap(), bytes);
        assert!(auto.remember_failure(CHANNEL, "id:43", "receiver_create", "new cause"));
        let loaded = AutoRecorder::load(dir.path());
        let snapshot = loaded.snapshot();
        let failure = snapshot.channels[0].last_failure.as_ref().unwrap();
        assert_eq!(failure.live_key, "id:43");
        assert_eq!(failure.message, "new cause");
    }

    #[test]
    fn successful_recording_clears_failure_durably_without_changing_registration() {
        let dir = tempfile::tempdir().unwrap();
        let auto = AutoRecorder::load(dir.path());
        auto.add(CHANNEL.into(), "test".into()).unwrap();
        auto.set_capture_chat(false).unwrap();
        assert!(auto.remember_failure(CHANNEL, "id:42", "receiver_create", "old failure"));
        let before = std::fs::read(dir.path().join(FILE)).unwrap();
        for (status, recording) in [("starting", None), ("retry", None), ("recording", None)] {
            auto.publish(CHANNEL, status, recording, None);
            assert!(auto.snapshot().channels[0].last_failure.is_some());
            assert_eq!(std::fs::read(dir.path().join(FILE)).unwrap(), before);
        }
        auto.publish(
            CHANNEL,
            "recording",
            Some("accepted-recording".into()),
            None,
        );
        assert!(auto.snapshot().channels[0].last_failure.is_none());
        let loaded = AutoRecorder::load(dir.path());
        assert!(loaded.snapshot().channels[0].last_failure.is_none());
        assert!(loaded.snapshot().channels[0].enabled);
        assert!(!loaded.capture_chat());
        // A subsequent failure, even in the same broadcast, is a new attempt.
        assert!(auto.remember_failure(CHANNEL, "id:42", "receiver_create", "new failure"));
    }

    #[test]
    fn successful_start_keeps_recording_when_history_cleanup_cannot_be_saved() {
        let dir = tempfile::tempdir().unwrap();
        let auto = AutoRecorder::load(dir.path());
        auto.add(CHANNEL.into(), "test".into()).unwrap();
        auto.remember_failure(CHANNEL, "id:42", "receiver_create", "old failure");
        // A corrupt-config fence must not be overwritten by a status update.
        auto.core.lock().unwrap().fault = Some("configuration unavailable".into());
        let before = std::fs::read(dir.path().join(FILE)).unwrap();
        auto.publish(CHANNEL, "recording", Some("accepted".into()), None);
        assert!(auto.snapshot().channels[0].last_failure.is_none());
        assert_eq!(auto.snapshot().channels[0].progress.status, "recording");
        assert!(auto.snapshot().error.is_some());
        assert_eq!(std::fs::read(dir.path().join(FILE)).unwrap(), before);
    }
    #[test]
    fn legacy_registration_loads_without_failure_and_unknown_channels_cannot_create_history() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = serde_json::to_vec(&json!({"version":1,"captureChat":true,"channels":[{
            "channelId":CHANNEL,"channelName":"test","enabled":true,
            "suppressedLive":"id:42","holdUntilOffline":false
        }]}))
        .unwrap();
        std::fs::write(dir.path().join(FILE), &bytes).unwrap();
        let auto = AutoRecorder::load(dir.path());
        assert!(auto.snapshot().error.is_none());
        assert!(auto.snapshot().channels[0].last_failure.is_none());
        assert!(!auto.allowed(CHANNEL, "id:42"));
        assert!(!auto.remember_failure(&"b".repeat(32), "id:42", "player_prepare", "no"));
        assert_eq!(std::fs::read(dir.path().join(FILE)).unwrap(), bytes);
    }
    #[test]
    fn failure_write_error_retains_memory_and_stop_fence_then_recovers_on_next_edit() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("not-created");
        let auto = AutoRecorder::load(&missing);
        auto.core.lock().unwrap().config.channels.push(Channel {
            channel_id: CHANNEL.into(),
            channel_name: "test".into(),
            enabled: true,
            suppressed_live: Some("id:42".into()),
            hold_until_offline: false,
            last_failure: None,
        });
        assert!(auto.remember_failure(CHANNEL, "id:42", "player_prepare", "not started"));
        assert!(auto.snapshot().error.is_some());
        assert!(auto.snapshot().channels[0].last_failure.is_some());
        assert!(!auto.allowed(CHANNEL, "id:42"));
        assert!(!missing.exists());
        assert!(auto.suppress(CHANNEL).is_err());
        assert!(!auto.allowed(CHANNEL, "id:any"));
        std::fs::create_dir(&missing).unwrap();
        auto.set_capture_chat(false).unwrap();
        assert!(auto.snapshot().error.is_none());
        let loaded = AutoRecorder::load(&missing);
        assert!(loaded.snapshot().channels[0].last_failure.is_some());
        assert!(!loaded.allowed(CHANNEL, "id:any"));
    }
    #[test]
    fn persisted_failure_is_bounded_and_never_copies_untrusted_page_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let auto = AutoRecorder::load(dir.path());
        auto.add(CHANNEL.into(), "test".into()).unwrap();
        assert!(auto.remember_failure(
            CHANNEL,
            &"한".repeat(200),
            &"글".repeat(100),
            &"자".repeat(600)
        ));
        let loaded = AutoRecorder::load(dir.path());
        assert!(loaded.snapshot().error.is_none());
        let snapshot = loaded.snapshot();
        let failure = snapshot.channels[0].last_failure.as_ref().unwrap();
        assert_eq!(failure.live_key.chars().count(), 160);
        assert_eq!(failure.stage.chars().count(), 80);
        assert_eq!(failure.message.chars().count(), 500);
        let message = preparation_problem(Some(
            &json!({"reason":"unexpected-private-url","cookie":"secret"}),
        ));
        assert!(!message.contains("private") && !message.contains("secret"));
        assert!(preparation_problem(Some(&json!({"reason":"encrypted"}))).contains("보호된"));
    }
    #[test]
    fn a_recording_completed_between_ticks_is_not_reported_as_never_started() {
        let mut state = ViewState::default();
        state.accepted_arm = Some((
            "request-1".into(),
            "recording-1".into(),
            state.page_generation,
        ));
        assert!(state.recording.is_none());
        assert_eq!(
            accepted_session_recording(&state, Some("request-1")).as_deref(),
            Some("recording-1")
        );
        assert!(accepted_session_recording(&state, Some("request-2")).is_none());
        assert!(accepted_session_recording(&state, None).is_none());
        state.page_generation += 1;
        assert!(accepted_session_recording(&state, Some("request-1")).is_none());
    }
    #[test]
    fn unknown_broadcast_stop_waits_for_offline_and_explicit_enable_rearms() {
        let dir = tempfile::tempdir().unwrap();
        let auto = AutoRecorder::load(dir.path());
        auto.add(CHANNEL.into(), "방송".into()).unwrap();
        auto.suppress(CHANNEL).unwrap();
        assert!(!auto.allowed(CHANNEL, "id:new"));
        auto.update(CHANNEL, Change::Enabled { enabled: true })
            .unwrap();
        assert!(auto.allowed(CHANNEL, "id:new"));
        auto.update(CHANNEL, Change::Remove).unwrap();
        assert!(auto.snapshot().channels.is_empty());
    }
    #[test]
    fn duplicate_registration_is_idempotent_and_invalid_input_cannot_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let auto = AutoRecorder::load(dir.path());
        auto.add(CHANNEL.into(), "first".into()).unwrap();
        auto.add(CHANNEL.into(), "second".into()).unwrap();
        assert_eq!(auto.snapshot().channels.len(), 1);
        assert!(auto.add("../outside".into(), "bad".into()).is_err());
        assert_eq!(AutoRecorder::load(dir.path()).snapshot().channels.len(), 1);
    }
    #[test]
    fn network_failure_is_not_broadcast_end_and_stale_metadata_cannot_start() {
        let mut observed = Observation::default();
        observed.accept(Ok(live("1", LiveStatus::Live)), 100);
        assert!(observed.can_start(101));
        assert!(!observed.can_start(100_000));
        observed.accept(Ok(live("1", LiveStatus::Offline)), 200);
        assert_eq!(observed.offline_count, 1);
        observed.accept(Err(unavailable()), 300);
        assert_eq!(observed.offline_count, 0);
        assert!(!observed.can_start(301));
        observed.accept(Ok(live("1", LiveStatus::Offline)), 400);
        observed.accept(Ok(live("1", LiveStatus::Offline)), 500);
        assert_eq!(observed.offline_count, 2);
    }
    #[test]
    fn corrupt_configuration_is_preserved_and_never_automatically_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        std::fs::write(&path, b"corrupt").unwrap();
        let auto = AutoRecorder::load(dir.path());
        assert!(auto.snapshot().error.is_some());
        assert!(auto.snapshot().channels.is_empty());
        assert!(auto.add(CHANNEL.into(), "test".into()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"corrupt");
    }
    #[test]
    fn failed_stop_persistence_still_prevents_restarting_in_the_current_process() {
        let dir = tempfile::tempdir().unwrap();
        let auto = AutoRecorder::load(&dir.path().join("missing"));
        auto.core.lock().unwrap().config.channels.push(Channel {
            channel_id: CHANNEL.into(),
            channel_name: "test".into(),
            enabled: true,
            suppressed_live: None,
            hold_until_offline: false,
            last_failure: None,
        });
        assert!(auto.suppress(CHANNEL).is_err());
        assert!(!auto.allowed(CHANNEL, "id:any"));
    }
    #[test]
    fn registrations_are_bounded_and_an_overflow_preserves_the_saved_list() {
        let dir = tempfile::tempdir().unwrap();
        let auto = AutoRecorder::load(dir.path());
        for id in 0..MAX_CHANNELS {
            auto.add(format!("{id:032x}"), "test".into()).unwrap();
        }
        assert!(auto
            .add(format!("{:032x}", MAX_CHANNELS), "overflow".into())
            .is_err());
        assert_eq!(
            AutoRecorder::load(dir.path()).snapshot().channels.len(),
            MAX_CHANNELS
        );
    }
    #[test]
    fn a_new_manual_recording_is_never_owned_or_stopped_as_the_previous_auto_session() {
        assert!(session_replaced(
            Some("old"),
            Some("new"),
            Some("arm1"),
            Some("arm2"),
            true
        ));
        assert!(session_replaced(
            None,
            Some("new"),
            Some("arm1"),
            Some("arm2"),
            true
        ));
        assert!(!session_replaced(
            Some("old"),
            None,
            Some("arm1"),
            Some("arm1"),
            false
        ));
        assert!(!session_replaced(
            None,
            Some("new"),
            None,
            Some("arm2"),
            true
        ));
    }
}
