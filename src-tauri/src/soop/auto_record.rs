//! Explicit SOOP reservations; no UI timer, account, hidden viewer or implicit opt-in.
use crate::platform_media::{
    live::{RecordingOwner, ScheduledCapture, ScheduledState},
    storage,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::Manager;

const FILE: &str = "soop-auto-record.json";
const MAX_ENABLED: usize = 32;
const MAX_CONFIG: usize = 128;
const POLL: u64 = 30_000;
fn clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn fault() -> String {
    "자동 녹화 설정을 읽지 못했습니다. 기존 파일을 보존하고 예약을 중지했습니다.".into()
}
fn volatile_stop_error(error: &str) -> String {
    if error.contains("예약 해제는 이번 실행에만 적용됩니다.") {
        return error.into();
    }
    format!("{error} 예약 해제는 이번 실행에만 적용됩니다. 다시 실행하면 예약이 켜질 수 있습니다.")
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Channel {
    channel_id: String,
    name: String,
    enabled: bool,
    generation: u64,
    last_broadcast: Option<String>,
    last_run: Option<String>,
    #[serde(default)]
    last_failure: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    version: u8,
    revision: u64,
    channels: Vec<Channel>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 1,
            channels: Vec::new(),
        }
    }
}
#[derive(Clone, Default)]
struct Observation {
    live: String,
    checked_at: u64,
    next_at: u64,
    broadcast: Option<String>,
    offline: u8,
    errors: u8,
    error: Option<String>,
    restricted: bool,
}
#[derive(Clone)]
struct Running {
    capture: ScheduledCapture,
    stopping: bool,
    state: String,
    error: Option<String>,
}
#[derive(Default)]
struct Core {
    config: Config,
    loaded: bool,
    fault: Option<String>,
    observations: HashMap<String, Observation>,
    running: HashMap<String, Running>,
    admissions: HashSet<String>,
    cursor: usize,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoChannel {
    pub channel_id: String,
    pub name: String,
    pub enabled: bool,
    pub live_status: String,
    pub status: String,
    pub job_id: Option<String>,
    pub checked_at: Option<u64>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoSnapshot {
    pub revision: u64,
    pub channels: Vec<AutoChannel>,
    pub error: Option<String>,
    pub ready: bool,
}
#[derive(Clone)]
struct Attempt {
    id: String,
    generation: u64,
    broadcast: String,
    run_id: String,
}
#[derive(Clone)]
struct Observed {
    live: String,
    broadcast: Option<String>,
    restricted: bool,
}
trait Host: Send + Sync {
    fn observe(&self, id: &str) -> Result<Observed, String>;
    fn start(
        &self,
        scheduler: &SoopAutoRecorder,
        attempt: &Attempt,
    ) -> Result<ScheduledCapture, String>;
    fn state(&self, capture: &ScheduledCapture) -> Result<ScheduledState, String>;
    fn stop(&self, capture: &ScheduledCapture) -> Result<(), String>;
}
struct NativeHost(tauri::AppHandle);
impl Host for NativeHost {
    fn observe(&self, id: &str) -> Result<Observed, String> {
        let channel = super::observe_channel(id)?;
        Ok(Observed {
            live: channel.status.into(),
            broadcast: channel.broadcast_key,
            restricted: channel.restricted,
        })
    }
    fn start(
        &self,
        scheduler: &SoopAutoRecorder,
        attempt: &Attempt,
    ) -> Result<ScheduledCapture, String> {
        let state = self
            .0
            .try_state::<crate::interface::commands::AppState>()
            .ok_or("앱을 준비 중입니다.")?;
        let settings = state
            .settings_snapshot()
            .map_err(|_| "저장 폴더 설정을 읽지 못했습니다.")?;
        if settings.download_root.trim().is_empty() {
            return Err("설정에서 다운로드 폴더를 먼저 선택하세요.".into());
        }
        // Tool/journal initialization happens outside the admission lock; the
        // second guard below rejects quit/update/off that occurred during it.
        let media = state.platform_media(&self.0)?;
        let native = self.0.state::<super::original::SoopOriginalHost>();
        let token = native.prepare_scheduled(&self.0, media, attempt.id.clone(), &attempt.broadcast, attempt.run_id.clone(), settings.download_root.into())?;
        let result = state.begin_platform_work(|| {
            scheduler.start_if_current(attempt, |owner| {
                native.start_scheduled(&self.0, &token, owner)
            })
        });
        // Preparation is a temporary viewer lease. An admitted archive keeps
        // the exact receiver alive; no native extractor is started here.
        let _ = native.close(&self.0, &token);
        result
    }
    fn state(&self, capture: &ScheduledCapture) -> Result<ScheduledState, String> {
        self.0.try_state::<crate::interface::commands::AppState>()
            .and_then(|state|state.existing_platform_media())
            .ok_or("녹화 작업을 찾을 수 없습니다.")?.encoded_scheduled_state(capture)
    }
    fn stop(&self, capture: &ScheduledCapture) -> Result<(), String> {
        self.0.state::<super::original::SoopOriginalHost>().stop_scheduled(&self.0,capture)
    }
}
pub struct SoopAutoRecorder {
    directory: PathBuf,
    core: Mutex<Core>,
    wake: Condvar,
    started: AtomicBool,
    stopped: AtomicBool,
    workers: AtomicUsize,
}
struct WorkerDone(Arc<SoopAutoRecorder>);
impl Drop for WorkerDone {
    fn drop(&mut self) {
        self.0.workers.fetch_sub(1, Ordering::AcqRel);
        self.0.wake.notify_all();
    }
}
struct Admission<'a> {
    scheduler: &'a SoopAutoRecorder,
    run_id: String,
}
impl Drop for Admission<'_> {
    fn drop(&mut self) {
        self.scheduler
            .core
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .admissions
            .remove(&self.run_id);
    }
}
impl SoopAutoRecorder {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            core: Mutex::new(Core::default()),
            wake: Condvar::new(),
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            workers: AtomicUsize::new(0),
        }
    }
    pub fn start(self: &Arc<Self>, app: tauri::AppHandle) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        // Two bounded metadata workers; neither performs a request under the
        // configuration/quit lock. next_at reserves each channel before request.
        for _ in 0..2 {
            let Some(worker_guard) = self.register_worker() else {
                break;
            };
            let scheduler = self.clone();
            let host = NativeHost(app.clone());
            if std::thread::Builder::new()
                .name("soop-auto-record".into())
                .spawn(move || {
                    let _worker_guard = worker_guard;
                    scheduler.load();
                    while !scheduler.stopped.load(Ordering::Acquire) {
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            scheduler.cycle(&host, clock())
                        }))
                        .is_err()
                        {
                            let mut core = scheduler.core.lock().unwrap_or_else(|e| e.into_inner());
                            core.fault = Some(
                                "자동 녹화 작업이 중단됐습니다. 앱을 다시 실행해 주세요.".into(),
                            );
                            break;
                        }
                        let guard = scheduler.core.lock().unwrap_or_else(|e| e.into_inner());
                        let _ = scheduler
                            .wake
                            .wait_timeout(guard, Duration::from_millis(500));
                    }
                })
                .is_err()
            {
                self.core.lock().unwrap_or_else(|e| e.into_inner()).fault =
                    Some("자동 녹화 확인 작업을 시작하지 못했습니다.".into());
            }
        }
    }
    fn register_worker(self: &Arc<Self>) -> Option<WorkerDone> {
        let _core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        if self.stopped.load(Ordering::Acquire) {
            return None;
        }
        self.workers.fetch_add(1, Ordering::AcqRel);
        Some(WorkerDone(self.clone()))
    }
    fn load(&self) {
        let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        if core.loaded {
            return;
        }
        match read_config(&self.directory) {
            Ok(config) => core.config = config,
            Err(error) => core.fault = Some(error),
        }
        core.loaded = true;
    }
    pub fn shutdown(&self) {
        let guard = self.core.lock().unwrap_or_else(|e| e.into_inner());
        self.stopped.store(true, Ordering::Release);
        drop(guard);
        self.wake.notify_all();
    }
    /// Call outside quit/configuration locks. A timeout is not a drained result.
    pub fn shutdown_and_wait(&self, timeout: Duration) -> Result<(), String> {
        self.shutdown();
        let started = std::time::Instant::now();
        let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        while self.workers.load(Ordering::Acquire) > 0 {
            let Some(remaining) = timeout
                .checked_sub(started.elapsed())
                .filter(|duration| !duration.is_zero())
            else {
                return Err("자동 녹화 확인 작업이 종료될 때까지 기다려 주세요.".into());
            };
            core = self
                .wake
                .wait_timeout(core, remaining.min(Duration::from_millis(100)))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        Ok(())
    }
    pub fn active_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .core
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .admissions
            .iter()
            .map(|id| format!("soop-auto-start:{id}"))
            .collect();
        if self.stopped.load(Ordering::Acquire) {
            ids.extend(
                (0..self.workers.load(Ordering::Acquire))
                    .map(|index| format!("soop-auto-drain:{index}")),
            );
        }
        ids
    }
    pub fn snapshot(&self) -> AutoSnapshot {
        let core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        let channels = core
            .config
            .channels
            .iter()
            .map(|channel| {
                let observation = core.observations.get(&channel.channel_id);
                let running = core.running.get(&channel.channel_id);
                let status = if let Some(running) = running {
                    if running.stopping
                        || matches!(running.state.as_str(), "finalizing" | "verifying")
                    {
                        "stopping"
                    } else if running.state == "recording" {
                        "recording"
                    } else {
                        "starting"
                    }
                } else if observation.is_some_and(|o| o.error.as_deref() == Some("자리 대기")) {
                    "waiting"
                } else if channel.last_failure.is_some()
                    || observation.is_some_and(|o| o.error.is_some())
                {
                    "failed"
                } else if channel.enabled
                    && channel.last_broadcast.is_some()
                    && observation.is_some_and(|o| o.broadcast == channel.last_broadcast)
                {
                    "skipped"
                } else {
                    "idle"
                };
                AutoChannel {
                    channel_id: channel.channel_id.clone(),
                    name: channel.name.clone(),
                    enabled: channel.enabled,
                    live_status: observation.map_or_else(
                        || "unknown".into(),
                        |o| {
                            if matches!(o.live.as_str(), "live" | "offline") {
                                o.live.clone()
                            } else {
                                "unknown".into()
                            }
                        },
                    ),
                    status: status.into(),
                    job_id: running.map(|r| r.capture.job_id.clone()),
                    checked_at: observation.map(|o| o.checked_at).filter(|n| *n > 0),
                    error: running
                        .and_then(|r| r.error.clone())
                        .or_else(|| channel.last_failure.clone())
                        .or_else(|| observation.and_then(|o| o.error.clone())),
                }
            })
            .collect();
        AutoSnapshot {
            revision: core.config.revision,
            channels,
            error: core.fault.clone(),
            ready: core.loaded,
        }
    }
    pub fn set(
        &self,
        channel_id: String,
        name: String,
        enabled: bool,
        expected_revision: u64,
    ) -> Result<AutoSnapshot, String> {
        let id = super::normalize_channel(&channel_id)?;
        let name = super::clean_text(Some(&name), 120).unwrap_or_else(|| id.clone());
        let mut core = self.core.lock().map_err(|_| fault())?;
        if self.stopped.load(Ordering::Acquire) {
            return Err("앱을 종료하는 중입니다.".into());
        }
        if !core.loaded {
            return Err("자동 녹화 설정을 불러오는 중입니다.".into());
        }
        if core.config.revision != expected_revision {
            return Err("예약 상태가 변경됐습니다. 다시 시도해 주세요.".into());
        }
        if let Some(error) = core.fault.clone() {
            // Never overwrite a damaged/failed durable file. An explicit off
            // still fences this process's receiver; report non-persistence.
            if !enabled {
                if let Some(channel) = core
                    .config
                    .channels
                    .iter_mut()
                    .find(|c| c.channel_id == id && c.enabled)
                {
                    channel.enabled = false;
                    channel.generation = channel.generation.saturating_add(1);
                    core.config.revision = core.config.revision.saturating_add(1);
                    self.wake.notify_all();
                    let error = volatile_stop_error(&error);
                    core.fault = Some(error.clone());
                    return Err(error);
                }
            }
            return Err(error);
        }
        if core
            .config
            .channels
            .iter()
            .find(|c| c.channel_id == id)
            .is_some_and(|c| c.enabled == enabled)
            || (!enabled && !core.config.channels.iter().any(|c| c.channel_id == id))
        {
            drop(core);
            return Ok(self.snapshot());
        }
        let mut next = core.config.clone();
        if let Some(channel) = next.channels.iter_mut().find(|c| c.channel_id == id) {
            channel.enabled = enabled;
            channel.generation = channel
                .generation
                .checked_add(1)
                .ok_or("예약 변경 횟수를 초과했습니다.")?;
            channel.name = name;
            if enabled {
                channel.last_broadcast = None;
                channel.last_run = None;
            }
        } else {
            next.channels.push(Channel {
                channel_id: id.clone(),
                name,
                enabled,
                generation: 1,
                last_broadcast: None,
                last_run: None,
                last_failure: None,
            });
        }
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or("예약 변경 횟수를 초과했습니다.")?;
        validate(&next)?;
        if let Err(error) = save_config(&self.directory, &next) {
            // An explicit stop must fence off a pending start even if the disk
            // becomes read-only. The old durable file is kept; advertise fault.
            if !enabled {
                core.config = next;
                let error = volatile_stop_error(&error);
                core.fault = Some(error.clone());
                self.wake.notify_all();
                return Err(error);
            }
            return Err(error);
        }
        core.config = next;
        if let Some(observation) = core.observations.get_mut(&id) {
            observation.next_at = 0;
            observation.error = None;
        }
        drop(core);
        self.wake.notify_all();
        Ok(self.snapshot())
    }
    fn start_if_current(
        &self,
        attempt: &Attempt,
        start: impl FnOnce(RecordingOwner) -> Result<ScheduledCapture, String>,
    ) -> Result<ScheduledCapture, String> {
        let mut core = self.core.lock().map_err(|_| fault())?;
        if self.stopped.load(Ordering::Acquire) || core.fault.is_some() {
            return Err("예약 시작이 취소됐습니다.".into());
        }
        let channel = core
            .config
            .channels
            .iter()
            .find(|c| {
                c.channel_id == attempt.id
                    && c.enabled
                    && c.generation == attempt.generation
                    && c.last_broadcast.as_deref() != Some(&attempt.broadcast)
            })
            .ok_or("예약 상태가 변경됐습니다.")?;
        let _ = channel;
        let mut next = core.config.clone();
        let channel = next
            .channels
            .iter_mut()
            .find(|c| c.channel_id == attempt.id)
            .unwrap();
        channel.last_broadcast = Some(attempt.broadcast.clone());
        channel.last_run = Some(attempt.run_id.clone());
        next.revision += 1;
        save_config(&self.directory, &next)?;
        core.config = next;
        let owner = RecordingOwner::Scheduled {
            schedule_id: attempt.id.clone(),
            run_id: attempt.run_id.clone(),
            broadcast_key: attempt.broadcast.clone(),
            generation: attempt.generation,
        };
        let result = start(owner);
        // Capacity is the only retryable pre-admission result. If a manual
        // recorder already owns this broadcast, keep suppression so stopping it
        // never starts an automatic replacement behind the user's back.
        if result.as_ref().err().is_some_and(|e| e == "자리 대기") {
            let mut next = core.config.clone();
            let channel = next
                .channels
                .iter_mut()
                .find(|c| c.channel_id == attempt.id)
                .unwrap();
            channel.last_broadcast = None;
            channel.last_run = None;
            next.revision += 1;
            if let Err(error) = save_config(&self.directory, &next) {
                core.fault = Some(error);
            } else {
                core.config = next;
            }
        }
        if let Ok(capture) = &result {
            core.running.insert(
                attempt.id.clone(),
                Running {
                    capture: capture.clone(),
                    stopping: false,
                    state: "resolving".into(),
                    error: None,
                },
            );
        } else if let Err(error) = &result {
            if !matches!(error.as_str(), "자리 대기" | "이미 녹화 중입니다.") {
                self.persist_failure(&mut core, &attempt.id, Some(error.clone()));
            }
        }
        result
    }
    fn persist_failure(&self, core: &mut Core, id: &str, failure: Option<String>) {
        let mut next = core.config.clone();
        let Some(channel) = next.channels.iter_mut().find(|c| c.channel_id == id) else {
            return;
        };
        let failure = failure.map(|message| {
            message
                .chars()
                .filter(|c| !c.is_control())
                .take(400)
                .collect::<String>()
        });
        if channel.last_failure == failure {
            return;
        }
        channel.last_failure = failure;
        next.revision = next.revision.saturating_add(1);
        if let Err(error) = save_config(&self.directory, &next) {
            core.fault = Some(error);
        }
        // Retain the failure in memory even if its durable write failed.
        core.config = next;
    }
    fn cycle(&self, host: &dyn Host, time: u64) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let running: Vec<_> = self
            .core
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .running
            .iter()
            .map(|(id, r)| (id.clone(), r.clone()))
            .collect();
        for (id, run) in running {
            let should_stop = {
                let core = self.core.lock().unwrap_or_else(|e| e.into_inner());
                let (generation, broadcast) = match &run.capture.owner {
                    RecordingOwner::Scheduled {
                        generation,
                        broadcast_key,
                        ..
                    } => (*generation, Some(broadcast_key)),
                    _ => (0, None),
                };
                !core
                    .config
                    .channels
                    .iter()
                    .any(|c| c.channel_id == id && c.enabled && c.generation == generation)
                    || core.observations.get(&id).is_some_and(|o| o.offline >= 2)
                    || core.observations.get(&id).is_some_and(|o| {
                        o.live == "live"
                            && o.error.is_none()
                            && o.broadcast.as_ref().is_some_and(|b| Some(b) != broadcast)
                    })
            };
            let stop_error = if should_stop && !run.stopping {
                host.stop(&run.capture).err()
            } else {
                None
            };
            let state = host.state(&run.capture);
            let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
            if core
                .running
                .get(&id)
                .is_none_or(|current| current.capture.job_id != run.capture.job_id)
            {
                continue;
            }
            match state {
                Ok(state) if !state.active => {
                    core.running.remove(&id);
                    if let Some(error) = state.error {
                        self.persist_failure(&mut core, &id, Some(error));
                    }
                }
                Ok(state) => {
                    if state.state == "recording" {
                        self.persist_failure(&mut core, &id, None);
                    }
                    let current = core.running.get_mut(&id).unwrap();
                    current.state = state.state;
                    current.error = stop_error.clone().or(state.error);
                    current.stopping |= should_stop && stop_error.is_none();
                }
                Err(error) => {
                    core.running.get_mut(&id).unwrap().error = Some(error);
                }
            }
        }
        let selected = {
            let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
            if !core.loaded || core.fault.is_some() || self.stopped.load(Ordering::Acquire) {
                return;
            }
            let channels: Vec<_> = core
                .config
                .channels
                .iter()
                .filter(|c| c.enabled)
                .cloned()
                .collect();
            let mut selected = None;
            for step in 0..channels.len() {
                let index = (core.cursor + step) % channels.len();
                let channel = &channels[index];
                let observation = core
                    .observations
                    .entry(channel.channel_id.clone())
                    .or_default();
                if observation.next_at <= time {
                    observation.next_at = time.saturating_add(POLL);
                    selected = Some(channel.clone());
                    core.cursor = index + 1;
                    break;
                }
            }
            selected
        };
        let Some(channel) = selected else {
            return;
        };
        let response = host.observe(&channel.channel_id);
        let attempt = {
            let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
            if self.stopped.load(Ordering::Acquire)
                || !core.config.channels.iter().any(|c| {
                    c.channel_id == channel.channel_id
                        && c.enabled
                        && c.generation == channel.generation
                })
            {
                return;
            }
            let observation = core
                .observations
                .entry(channel.channel_id.clone())
                .or_default();
            observation.checked_at = time;
            match response {
                Err(error) => {
                    observation.live = "unknown".into();
                    observation.offline = 0;
                    observation.errors = observation.errors.saturating_add(1);
                    observation.error = Some(error);
                    observation.next_at = time
                        .saturating_add(POLL.saturating_mul(1_u64 << observation.errors.min(4)));
                    return;
                }
                Ok(observed) => {
                    observation.live = observed.live;
                    observation.broadcast = observed.broadcast;
                    observation.restricted = observed.restricted;
                    observation.errors = 0;
                    observation.error = None;
                    observation.offline = if observation.live == "offline" {
                        observation.offline.saturating_add(1)
                    } else {
                        0
                    };
                }
            }
            if observation.restricted {
                observation.error = Some("공개 방송만 자동 녹화할 수 있습니다.".into());
                return;
            }
            if observation.live != "live" {
                return;
            }
            let Some(broadcast) = observation.broadcast.clone() else {
                return;
            };
            if core
                .config
                .channels
                .iter()
                .find(|current| current.channel_id == channel.channel_id)
                .is_some_and(|current| current.last_broadcast.as_ref() == Some(&broadcast))
                || core.running.contains_key(&channel.channel_id)
            {
                return;
            }
            let attempt = Attempt {
                id: channel.channel_id,
                generation: channel.generation,
                broadcast,
                run_id: uuid::Uuid::new_v4().to_string(),
            };
            core.admissions.insert(attempt.run_id.clone());
            attempt
        };
        let _admission = Admission {
            scheduler: self,
            run_id: attempt.run_id.clone(),
        };
        let result = host.start(self, &attempt);
        let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = result {
            // Settings/tool preparation can fail before start_if_current. Keep
            // that first failure too, but never let a late old attempt alter a
            // newly toggled reservation or shutdown state.
            if self.stopped.load(Ordering::Acquire)
                || !core.config.channels.iter().any(|c| {
                    c.channel_id == attempt.id && c.enabled && c.generation == attempt.generation
                })
            {
                return;
            }
            if !matches!(error.as_str(), "자리 대기" | "이미 녹화 중입니다.") {
                let mut next = core.config.clone();
                let channel = next
                    .channels
                    .iter_mut()
                    .find(|c| c.channel_id == attempt.id)
                    .unwrap();
                if channel.last_broadcast.as_deref() != Some(attempt.broadcast.as_str()) {
                    channel.last_broadcast = Some(attempt.broadcast.clone());
                    channel.last_run = Some(attempt.run_id.clone());
                    next.revision = next.revision.saturating_add(1);
                    if let Err(write_error) = save_config(&self.directory, &next) {
                        core.fault = Some(write_error);
                    }
                    core.config = next;
                }
                self.persist_failure(&mut core, &attempt.id, Some(error.clone()));
            }
            core.observations.entry(attempt.id).or_default().error =
                (error != "이미 녹화 중입니다.").then_some(error);
        }
    }
}
fn validate(config: &Config) -> Result<(), String> {
    if config.version != 1
        || config.revision == 0
        || config.revision > 9_007_199_254_740_000
        || config.channels.len() > MAX_CONFIG
        || config.channels.iter().filter(|c| c.enabled).count() > MAX_ENABLED
    {
        return Err("자동 녹화는 32개 채널까지 등록할 수 있습니다.".into());
    }
    let mut ids = HashSet::new();
    for c in &config.channels {
        if super::normalize_channel(&c.channel_id).ok().as_deref() != Some(&c.channel_id)
            || c.generation == 0
            || c.name.chars().count() > 120
            || c.name.chars().any(char::is_control)
            || c.last_failure.as_ref().is_some_and(|message| {
                message.chars().count() > 400 || message.chars().any(char::is_control)
            })
            || !ids.insert(&c.channel_id)
            || c.last_broadcast.as_ref().is_some_and(|b| {
                b.is_empty() || b.len() > 20 || !b.bytes().all(|b| b.is_ascii_digit())
            })
            || c.last_run
                .as_ref()
                .is_some_and(|r| uuid::Uuid::parse_str(r).is_err())
        {
            return Err(fault());
        }
    }
    Ok(())
}
fn read_config(directory: &Path) -> Result<Config, String> {
    storage::safe_path(directory)?;
    let path = directory.join(FILE);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(_) => return Err(fault()),
        Ok(_) => {}
    }
    if storage::regular(&path)?.len() > 65536 {
        return Err(fault());
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|_| fault())?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| fault())?;
    if bytes.len() > 65536 {
        return Err(fault());
    }
    let config = serde_json::from_slice(&bytes).map_err(|_| fault())?;
    validate(&config)?;
    Ok(config)
}
fn save_config(directory: &Path, config: &Config) -> Result<(), String> {
    validate(config)?;
    storage::safe_path(directory)?;
    fs::create_dir_all(directory).map_err(|_| "예약 저장 폴더를 만들지 못했습니다.")?;
    let target = directory.join(FILE);
    if target.exists() {
        storage::regular(&target)?;
    }
    let temporary = directory.join(format!("{}.soop-auto.tmp", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec(config).map_err(|_| fault())?;
    if bytes.len() > 65536 {
        return Err(fault());
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| "녹화 예약을 저장하지 못했습니다.")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "녹화 예약을 보관하지 못했습니다.")?;
    drop(file);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            },
        };
        let from: Vec<_> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<_> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
            .map_err(|_| "녹화 예약을 교체하지 못했습니다.")?;
        }
    }
    #[cfg(not(windows))]
    fs::rename(temporary, target).map_err(|_| "녹화 예약을 교체하지 못했습니다.")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FakeHost {
        observation: Mutex<Result<Observed, String>>,
        starts: Mutex<Vec<RecordingOwner>>,
        states: Mutex<HashMap<String, bool>>,
        stopped: AtomicUsize,
        capacity: AtomicBool,
        manual: AtomicBool,
        start_error: Mutex<Option<String>>,
        preparation_error: Mutex<Option<String>>,
    }
    impl FakeHost {
        fn new() -> Self {
            Self {
                observation: Mutex::new(Ok(Observed {
                    live: "live".into(),
                    broadcast: Some("123".into()),
                    restricted: false,
                })),
                starts: Mutex::new(Vec::new()),
                states: Mutex::new(HashMap::new()),
                stopped: AtomicUsize::new(0),
                capacity: AtomicBool::new(false),
                manual: AtomicBool::new(false),
                start_error: Mutex::new(None),
                preparation_error: Mutex::new(None),
            }
        }
        fn online(&self, key: &str) {
            *self.observation.lock().unwrap() = Ok(Observed {
                live: "live".into(),
                broadcast: Some(key.into()),
                restricted: false,
            });
        }
        fn offline(&self) {
            *self.observation.lock().unwrap() = Ok(Observed {
                live: "offline".into(),
                broadcast: None,
                restricted: false,
            });
        }
        fn finish(&self) {
            self.states
                .lock()
                .unwrap()
                .values_mut()
                .for_each(|v| *v = false);
        }
    }
    impl Host for FakeHost {
        fn observe(&self, _: &str) -> Result<Observed, String> {
            self.observation.lock().unwrap().clone()
        }
        fn start(
            &self,
            scheduler: &SoopAutoRecorder,
            attempt: &Attempt,
        ) -> Result<ScheduledCapture, String> {
            if let Some(error) = self.preparation_error.lock().unwrap().clone() {
                return Err(error);
            }
            scheduler.start_if_current(attempt, |owner| {
                if self.capacity.load(Ordering::Acquire) {
                    return Err("자리 대기".into());
                }
                if self.manual.load(Ordering::Acquire) {
                    return Err("이미 녹화 중입니다.".into());
                }
                if let Some(error) = self.start_error.lock().unwrap().clone() {
                    return Err(error);
                }
                self.starts.lock().unwrap().push(owner.clone());
                let capture = ScheduledCapture {
                    session_id: uuid::Uuid::new_v4().to_string(),
                    job_id: uuid::Uuid::new_v4().to_string(),
                    owner,
                };
                self.states
                    .lock()
                    .unwrap()
                    .insert(capture.job_id.clone(), true);
                Ok(capture)
            })
        }
        fn state(&self, capture: &ScheduledCapture) -> Result<ScheduledState, String> {
            let active = *self.states.lock().unwrap().get(&capture.job_id).unwrap();
            Ok(ScheduledState {
                state: if active { "recording" } else { "completed" }.into(),
                error: None,
                active,
            })
        }
        fn stop(&self, capture: &ScheduledCapture) -> Result<(), String> {
            self.stopped.fetch_add(1, Ordering::AcqRel);
            self.states
                .lock()
                .unwrap()
                .insert(capture.job_id.clone(), false);
            Ok(())
        }
    }
    fn scheduler(root: &Path) -> SoopAutoRecorder {
        let scheduler = SoopAutoRecorder::new(root.into());
        scheduler.load();
        scheduler
    }
    fn enable(s: &SoopAutoRecorder) {
        s.set(
            "fixture".into(),
            "fixture".into(),
            true,
            s.snapshot().revision,
        )
        .unwrap();
    }
    #[test]
    fn no_implicit_registration_and_revision_guard() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        let host = FakeHost::new();
        s.cycle(&host, 1);
        assert!(s.snapshot().channels.is_empty());
        assert!(!root.path().join(FILE).exists());
        s.set("fixture".into(), "fixture".into(), false, 1).unwrap();
        assert!(!root.path().join(FILE).exists());
        enable(&s);
        assert!(s.set("other".into(), "other".into(), true, 1).is_err());
        s.cycle(&host, 1);
        s.cycle(&host, 2);
        assert_eq!(host.starts.lock().unwrap().len(), 1);
        assert!(s.active_ids().is_empty());
        assert_eq!(s.snapshot().channels[0].status, "recording");
    }
    #[test]
    fn same_broadcast_intent_survives_restart_until_explicit_reenable() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        let host = FakeHost::new();
        enable(&s);
        s.cycle(&host, 1);
        let restarted = scheduler(root.path());
        restarted.cycle(&host, 1);
        assert_eq!(host.starts.lock().unwrap().len(), 1);
        restarted
            .set(
                "fixture".into(),
                "fixture".into(),
                false,
                restarted.snapshot().revision,
            )
            .unwrap();
        enable(&restarted);
        restarted.cycle(&host, 2);
        assert_eq!(host.starts.lock().unwrap().len(), 2);
    }
    #[test]
    fn two_explicit_offline_results_are_required_and_error_resets_confirmation() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        let host = FakeHost::new();
        enable(&s);
        s.cycle(&host, 1);
        host.offline();
        s.cycle(&host, 30_001);
        assert_eq!(host.stopped.load(Ordering::Acquire), 0);
        *host.observation.lock().unwrap() = Err("일시 연결 실패".into());
        s.cycle(&host, 60_001);
        s.cycle(&host, 60_002);
        assert_eq!(host.stopped.load(Ordering::Acquire), 0);
        host.offline();
        s.cycle(&host, 120_001);
        s.cycle(&host, 150_001);
        assert_eq!(host.stopped.load(Ordering::Acquire), 0);
        s.cycle(&host, 150_002);
        assert_eq!(host.stopped.load(Ordering::Acquire), 1);
    }
    #[test]
    fn manual_capture_is_never_stopped_or_restarted_implicitly() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        let host = FakeHost::new();
        host.manual.store(true, Ordering::Release);
        enable(&s);
        s.cycle(&host, 1);
        assert_eq!(s.snapshot().channels[0].status, "skipped");
        host.manual.store(false, Ordering::Release);
        s.cycle(&host, 30_001);
        assert!(host.starts.lock().unwrap().is_empty());
        assert_eq!(host.stopped.load(Ordering::Acquire), 0);
        s.set(
            "fixture".into(),
            "fixture".into(),
            false,
            s.snapshot().revision,
        )
        .unwrap();
        s.cycle(&host, 30_002);
        assert_eq!(host.stopped.load(Ordering::Acquire), 0);
    }
    #[test]
    fn disable_and_late_start_generation_and_shutdown_are_fenced() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        let host = FakeHost::new();
        enable(&s);
        let attempt = Attempt {
            id: "fixture".into(),
            generation: 1,
            broadcast: "123".into(),
            run_id: uuid::Uuid::new_v4().to_string(),
        };
        s.set(
            "fixture".into(),
            "fixture".into(),
            false,
            s.snapshot().revision,
        )
        .unwrap();
        assert!(s
            .start_if_current(&attempt, |_| panic!("stale start"))
            .is_err());
        enable(&s);
        s.cycle(&host, 1);
        s.set(
            "fixture".into(),
            "fixture".into(),
            false,
            s.snapshot().revision,
        )
        .unwrap();
        s.cycle(&host, 2);
        assert_eq!(host.stopped.load(Ordering::Acquire), 1);
        enable(&s);
        s.shutdown();
        s.cycle(&host, 30_001);
        assert_eq!(host.starts.lock().unwrap().len(), 1);
        assert!(s
            .set(
                "fixture".into(),
                "fixture".into(),
                false,
                s.snapshot().revision
            )
            .is_err());
    }
    #[test]
    fn capacity_retries_but_changed_broadcast_stops_old_owned_capture() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        let host = FakeHost::new();
        enable(&s);
        host.capacity.store(true, Ordering::Release);
        s.cycle(&host, 1);
        assert_eq!(s.snapshot().channels[0].status, "waiting");
        host.capacity.store(false, Ordering::Release);
        s.cycle(&host, 30_001);
        assert_eq!(host.starts.lock().unwrap().len(), 1);
        host.online("124");
        s.cycle(&host, 60_001);
        s.cycle(&host, 60_002);
        assert_eq!(host.stopped.load(Ordering::Acquire), 1);
        s.cycle(&host, 90_001);
        assert_eq!(host.starts.lock().unwrap().len(), 2);
    }
    #[test]
    fn malformed_settings_are_preserved_and_blocked() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(FILE);
        fs::write(&path, b"not json").unwrap();
        let s = scheduler(root.path());
        assert!(s.snapshot().error.is_some());
        assert!(s
            .set(
                "fixture".into(),
                "fixture".into(),
                true,
                s.snapshot().revision
            )
            .is_err());
        assert_eq!(fs::read(path).unwrap(), b"not json");
    }
    #[test]
    fn failed_disk_stop_still_fences_current_recording() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        let host = FakeHost::new();
        enable(&s);
        s.cycle(&host, 1);
        let path = root.path().join(FILE);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(s
            .set(
                "fixture".into(),
                "fixture".into(),
                false,
                s.snapshot().revision
            )
            .is_err());
        s.cycle(&host, 2);
        assert_eq!(host.stopped.load(Ordering::Acquire), 1);
        assert!(!s.snapshot().channels[0].enabled);
        assert!(s.snapshot().error.is_some());
    }
    #[test]
    fn runtime_fault_does_not_block_explicit_owner_stop_or_overwrite_disk() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        let host = FakeHost::new();
        enable(&s);
        s.cycle(&host, 1);
        let saved = fs::read(root.path().join(FILE)).unwrap();
        s.core.lock().unwrap().fault = Some("설정 보관 실패".into());
        assert!(s
            .set(
                "fixture".into(),
                "fixture".into(),
                false,
                s.snapshot().revision - 1
            )
            .is_err());
        assert!(s.snapshot().channels[0].enabled);
        let error = s
            .set(
                "fixture".into(),
                "fixture".into(),
                false,
                s.snapshot().revision,
            )
            .unwrap_err();
        assert!(error.contains("이번 실행"));
        s.cycle(&host, 2);
        assert_eq!(host.stopped.load(Ordering::Acquire), 1);
        assert!(!s.snapshot().channels[0].enabled);
        assert_eq!(fs::read(root.path().join(FILE)).unwrap(), saved);
    }
    #[test]
    fn restricted_unknown_and_ended_jobs_do_not_report_recording() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        let host = FakeHost::new();
        enable(&s);
        *host.observation.lock().unwrap() = Ok(Observed {
            live: "live".into(),
            broadcast: Some("123".into()),
            restricted: true,
        });
        s.cycle(&host, 1);
        assert!(host.starts.lock().unwrap().is_empty());
        host.online("123");
        s.cycle(&host, 30_001);
        host.finish();
        s.cycle(&host, 30_002);
        assert_eq!(s.snapshot().channels[0].status, "skipped");
    }
    #[test]
    fn start_failure_survives_healthy_poll_restart_and_acceptance_until_recording() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        let host = FakeHost::new();
        enable(&s);
        *host.start_error.lock().unwrap() = Some("저장 폴더 오류".into());
        s.cycle(&host, 1);
        s.cycle(&host, 30_001);
        assert_eq!(s.snapshot().channels[0].status, "failed");
        assert_eq!(
            s.snapshot().channels[0].error.as_deref(),
            Some("저장 폴더 오류")
        );
        let restarted = scheduler(root.path());
        assert_eq!(
            restarted.snapshot().channels[0].error.as_deref(),
            Some("저장 폴더 오류")
        );
        restarted
            .set(
                "fixture".into(),
                "fixture".into(),
                false,
                restarted.snapshot().revision,
            )
            .unwrap();
        enable(&restarted);
        *host.start_error.lock().unwrap() = None;
        restarted.cycle(&host, 1);
        assert_eq!(restarted.snapshot().channels[0].status, "starting");
        assert_eq!(
            restarted.snapshot().channels[0].error.as_deref(),
            Some("저장 폴더 오류")
        );
        restarted.cycle(&host, 2);
        assert_eq!(restarted.snapshot().channels[0].status, "recording");
        assert!(restarted.snapshot().channels[0].error.is_none());
    }
    #[test]
    fn admission_unwind_only_releases_its_own_identity() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        s.core
            .lock()
            .unwrap()
            .admissions
            .extend(["first".into(), "other-worker".into()]);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _owned = Admission {
                scheduler: &s,
                run_id: "first".into(),
            };
            panic!("synthetic unwind");
        }))
        .is_err());
        assert_eq!(s.active_ids(), vec!["soop-auto-start:other-worker"]);
    }
    #[test]
    fn shutdown_tracks_registered_workers_and_rejects_late_start() {
        let root = tempfile::tempdir().unwrap();
        let s = Arc::new(scheduler(root.path()));
        let registered = s.register_worker().unwrap();
        assert!(
            s.active_ids().is_empty(),
            "idle metadata workers are not recording jobs"
        );
        assert!(s.shutdown_and_wait(Duration::from_millis(2)).is_err());
        assert_eq!(s.active_ids(), vec!["soop-auto-drain:0"]);
        assert!(s.register_worker().is_none());
        drop(registered);
        s.shutdown_and_wait(Duration::from_millis(2)).unwrap();
        assert!(s.active_ids().is_empty());
    }
    #[test]
    fn metadata_worker_panic_and_unstarted_spawn_release_only_their_registration() {
        let root = tempfile::tempdir().unwrap();
        let s = Arc::new(scheduler(root.path()));
        let first = s.register_worker().unwrap();
        let second = s.register_worker().unwrap();
        s.shutdown();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _owned = first;
            panic!("synthetic metadata panic");
        }))
        .is_err());
        assert_eq!(s.workers.load(Ordering::Acquire), 1);
        drop(second);
        assert!(s.active_ids().is_empty());
    }
    #[test]
    fn preparation_failure_is_durable_and_does_not_loop_the_same_broadcast() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        let host = FakeHost::new();
        enable(&s);
        *host.preparation_error.lock().unwrap() = Some("저장 도구를 준비하지 못했습니다.".into());
        s.cycle(&host, 1);
        *host.preparation_error.lock().unwrap() = None;
        s.cycle(&host, 30_001);
        assert!(host.starts.lock().unwrap().is_empty());
        let restarted = scheduler(root.path());
        restarted.cycle(&host, 1);
        assert!(host.starts.lock().unwrap().is_empty());
        assert_eq!(
            restarted.snapshot().channels[0].error.as_deref(),
            Some("저장 도구를 준비하지 못했습니다.")
        );
        host.online("124");
        restarted.cycle(&host, 30_001);
        assert_eq!(host.starts.lock().unwrap().len(), 1);
    }
    #[test]
    fn enabled_limit_and_round_robin_do_not_starve_later_channels() {
        let root = tempfile::tempdir().unwrap();
        let s = scheduler(root.path());
        let host = FakeHost::new();
        host.offline();
        for index in 0..32 {
            s.set(
                format!("fixture{index}"),
                "fixture".into(),
                true,
                s.snapshot().revision,
            )
            .unwrap();
        }
        assert!(s
            .set(
                "overflow".into(),
                "fixture".into(),
                true,
                s.snapshot().revision
            )
            .is_err());
        for _ in 0..32 {
            s.cycle(&host, 1);
        }
        assert!(s
            .snapshot()
            .channels
            .iter()
            .all(|c| c.checked_at == Some(1)));
    }
}
