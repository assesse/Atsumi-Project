//! YouTube-only explicit channel reservations. Viewing and favorites never opt in.
use super::auto_record_observation::{self as discovery, Observation as Observed};
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

const FILE: &str = "youtube-auto-record.json";
const POLL: u64 = 30_000;
const MAX_BYTES: u64 = 512 * 1024;
const MAX_SEEN: usize = 128;
fn clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn fault() -> String {
    "YouTube 자동 녹화 설정을 읽지 못했습니다. 기존 파일을 보존하고 예약을 중지했습니다.".into()
}
fn clean(value: &str, maximum: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(maximum)
        .collect()
}
fn volatile(error: &str) -> String {
    format!("{error} 예약 해제는 이번 실행에만 적용됩니다. 다시 실행하면 예약이 켜질 수 있습니다.")
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Intent {
    video: String,
    run: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Channel {
    channel_id: String,
    name: String,
    enabled: bool,
    generation: u64,
    seen: Vec<Intent>,
    failure: Option<String>,
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
            channels: vec![],
        }
    }
}
#[derive(Clone, Default)]
struct PreparationRetry {
    failures: u8,
    next: u64,
    error: String,
}
#[derive(Clone, Default)]
struct Observation {
    checked: u64,
    next: u64,
    errors: u8,
    error: Option<String>,
    live: String,
    videos: Vec<String>,
    preparation_retries: HashMap<String, PreparationRetry>,
}
#[derive(Clone)]
struct Running {
    channel: String,
    capture: ScheduledCapture,
    state: String,
    stopping: bool,
    error: Option<String>,
}
#[derive(Default)]
struct Core {
    config: Config,
    loaded: bool,
    fault: Option<String>,
    observations: HashMap<String, Observation>,
    running: HashMap<String, Running>,
    admissions: HashMap<String, (String, String)>,
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
    pub recording_count: usize,
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
    channel: String,
    generation: u64,
    video: String,
    run: String,
}
#[derive(Debug)]
enum StartFailure {
    // No durable intent or native arm exists yet; safe to retry with backoff.
    Preparation(String),
    Reservation(String),
    Busy,
    // The intent already exists. Even an ambiguous ACK must remain suppressed.
    Reserved(String),
}
struct PolledCapture {
    state: ScheduledState,
    current: ScheduledCapture,
}
trait Host: Send + Sync {
    fn observe(&self, id: &str) -> Result<Observed, String>;
    fn start(
        &self,
        scheduler: &YouTubeAutoRecorder,
        attempt: &Attempt,
    ) -> Result<ScheduledCapture, StartFailure>;
    fn state(&self, capture: &ScheduledCapture) -> Result<PolledCapture, String>;
    fn stop(&self, capture: &ScheduledCapture) -> Result<(), String>;
}
struct NativeHost(tauri::AppHandle);
impl Host for NativeHost {
    fn observe(&self, id: &str) -> Result<Observed, String> {
        discovery::observe(id)
    }
    fn start(
        &self,
        scheduler: &YouTubeAutoRecorder,
        attempt: &Attempt,
    ) -> Result<ScheduledCapture, StartFailure> {
        let state = self
            .0
            .try_state::<crate::interface::commands::AppState>()
            .ok_or_else(|| StartFailure::Preparation("앱을 준비 중입니다.".into()))?;
        let settings = state
            .settings_snapshot()
            .map_err(|_| StartFailure::Preparation("저장 폴더를 읽지 못했습니다.".into()))?;
        if settings.download_root.trim().is_empty() {
            return Err(StartFailure::Preparation(
                "설정에서 다운로드 폴더를 먼저 선택하세요.".into(),
            ));
        }
        let media = state
            .platform_media(&self.0)
            .map_err(StartFailure::Preparation)?;
        let native = self.0.state::<super::original::YouTubeOriginalHost>();
        let token = native
            .prepare_scheduled(
                &self.0,
                media,
                attempt.channel.clone(),
                &attempt.video,
                attempt.run.clone(),
                settings.download_root.into(),
            )
            .map_err(StartFailure::Preparation)?;
        // No network/ACK wait under configuration or application admission locks.
        let owner = state
            .begin_platform_work(|| scheduler.reserve(attempt))
            .map_err(StartFailure::Reservation);
        let result = owner.and_then(|owner| {
            native
                .start_scheduled(&self.0, &token, owner)
                .map_err(|error| match error {
                    super::original::ScheduledStartError::Busy => StartFailure::Busy,
                    super::original::ScheduledStartError::Other(error) => {
                        StartFailure::Reserved(error)
                    }
                })
        });
        let _ = native.close(&self.0, &token);
        result
    }
    fn state(&self, capture: &ScheduledCapture) -> Result<PolledCapture, String> {
        let media=self.0
            .try_state::<crate::interface::commands::AppState>()
            .and_then(|state| state.existing_platform_media())
            .ok_or("녹화 작업을 찾을 수 없습니다.")?;
        let (current,state)=self.0.state::<super::original::YouTubeOriginalHost>().scheduled_poll(capture,&media)?;
        Ok(PolledCapture{state,current})
    }
    fn stop(&self, capture: &ScheduledCapture) -> Result<(), String> {
        self.0
            .state::<super::original::YouTubeOriginalHost>()
            .stop_scheduled(&self.0, capture)
    }
}
pub struct YouTubeAutoRecorder {
    directory: PathBuf,
    core: Mutex<Core>,
    wake: Condvar,
    started: AtomicBool,
    stopped: AtomicBool,
    workers: AtomicUsize,
}
struct Worker(Arc<YouTubeAutoRecorder>);
impl Drop for Worker {
    fn drop(&mut self) {
        self.0.workers.fetch_sub(1, Ordering::AcqRel);
        self.0.wake.notify_all();
    }
}
struct Admission<'a> {
    scheduler: &'a YouTubeAutoRecorder,
    run: String,
}
impl Drop for Admission<'_> {
    fn drop(&mut self) {
        self.scheduler
            .core
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .admissions
            .remove(&self.run);
        self.scheduler.wake.notify_all();
    }
}
struct PendingBatch<'a> {
    scheduler: &'a YouTubeAutoRecorder,
    runs: Vec<String>,
}
impl Drop for PendingBatch<'_> {
    fn drop(&mut self) {
        let mut core = self
            .scheduler
            .core
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        for run in &self.runs {
            core.admissions.remove(run);
        }
        self.scheduler.wake.notify_all();
    }
}
impl YouTubeAutoRecorder {
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
    fn register(self: &Arc<Self>) -> Option<Worker> {
        let _guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        if self.stopped.load(Ordering::Acquire) {
            return None;
        }
        self.workers.fetch_add(1, Ordering::AcqRel);
        Some(Worker(self.clone()))
    }
    pub fn start(self: &Arc<Self>, app: tauri::AppHandle) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        for _ in 0..2 {
            let Some(worker) = self.register() else { break };
            let scheduler = self.clone();
            let host = NativeHost(app.clone());
            if std::thread::Builder::new()
                .name("youtube-auto-record".into())
                .spawn(move || {
                    let _worker = worker;
                    scheduler.load();
                    while !scheduler.stopped.load(Ordering::Acquire) {
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            scheduler.cycle(&host, clock())
                        }))
                        .is_err()
                        {
                            scheduler
                                .core
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .fault = Some(
                                "YouTube 자동 녹화 확인이 중단됐습니다. 앱을 다시 실행하세요."
                                    .into(),
                            );
                            break;
                        }
                        let core = scheduler.core.lock().unwrap_or_else(|p| p.into_inner());
                        let _ = scheduler
                            .wake
                            .wait_timeout(core, Duration::from_millis(500));
                    }
                })
                .is_err()
            {
                self.core.lock().unwrap_or_else(|p| p.into_inner()).fault =
                    Some("YouTube 자동 녹화 확인을 시작하지 못했습니다.".into());
            }
        }
    }
    fn load(&self) {
        let mut core = self.core.lock().unwrap_or_else(|p| p.into_inner());
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
        let core = self.core.lock().unwrap_or_else(|p| p.into_inner());
        self.stopped.store(true, Ordering::Release);
        drop(core);
        self.wake.notify_all();
    }
    pub fn shutdown_and_wait(&self, timeout: Duration) -> Result<(), String> {
        self.shutdown();
        let began = std::time::Instant::now();
        let mut core = self.core.lock().unwrap_or_else(|p| p.into_inner());
        while self.workers.load(Ordering::Acquire) > 0 || !core.admissions.is_empty() {
            let Some(left) = timeout
                .checked_sub(began.elapsed())
                .filter(|v| !v.is_zero())
            else {
                return Err("YouTube 자동 녹화 작업이 끝날 때까지 기다려 주세요.".into());
            };
            core = self
                .wake
                .wait_timeout(core, left.min(Duration::from_millis(100)))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        Ok(())
    }
    pub fn active_ids(&self) -> Vec<String> {
        let core = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let mut ids: Vec<_> = core
            .admissions
            .keys()
            .map(|id| format!("youtube-auto-start:{id}"))
            .collect();
        if self.stopped.load(Ordering::Acquire) {
            ids.extend(
                (0..self.workers.load(Ordering::Acquire))
                    .map(|n| format!("youtube-auto-drain:{n}")),
            );
        }
        ids
    }
    pub fn snapshot(&self) -> AutoSnapshot {
        let core = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let channels =
            core.config
                .channels
                .iter()
                .map(|channel| {
                    let observation = core.observations.get(&channel.channel_id);
                    let runs: Vec<_> = core
                        .running
                        .values()
                        .filter(|run| run.channel == channel.channel_id)
                        .collect();
                    let status = if runs.iter().any(|r| r.state == "recording" && !r.stopping) {
                        "recording"
                    } else if runs.iter().any(|r| {
                        r.stopping || matches!(r.state.as_str(), "finalizing" | "verifying")
                    }) {
                        "stopping"
                    } else if !runs.is_empty()
                        || core
                            .admissions
                            .values()
                            .any(|(id, _)| id == &channel.channel_id)
                    {
                        "starting"
                    } else if observation.is_some_and(|o| !o.preparation_retries.is_empty()) {
                        "waiting"
                    } else if channel.failure.is_some()
                        || observation.is_some_and(|o| o.error.is_some())
                    {
                        "failed"
                    } else if channel.enabled
                        && observation.is_some_and(|o| {
                            !o.videos.is_empty()
                                && o.videos
                                    .iter()
                                    .all(|v| channel.seen.iter().any(|i| &i.video == v))
                        })
                    {
                        "skipped"
                    } else {
                        "idle"
                    };
                    AutoChannel {
                        channel_id: channel.channel_id.clone(),
                        name: channel.name.clone(),
                        enabled: channel.enabled,
                        live_status: observation
                            .map(|o| o.live.clone())
                            .filter(|s| matches!(s.as_str(), "live" | "offline"))
                            .unwrap_or_else(|| "unknown".into()),
                        status: status.into(),
                        job_id: runs.first().map(|r| r.capture.job_id.clone()),
                        checked_at: observation.map(|o| o.checked).filter(|v| *v > 0),
                        error: runs
                            .iter()
                            .find_map(|r| r.error.clone())
                            .or_else(|| channel.failure.clone())
                            .or_else(|| observation.and_then(|o| o.error.clone())),
                        recording_count: runs
                            .iter()
                            .filter(|r| r.state == "recording" && !r.stopping)
                            .count(),
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
        id: String,
        name: String,
        enabled: bool,
        revision: u64,
    ) -> Result<AutoSnapshot, String> {
        if !discovery::channel_id(&id) {
            return Err("확인된 YouTube 채널이 아닙니다.".into());
        }
        let mut core = self.core.lock().map_err(|_| fault())?;
        if self.stopped.load(Ordering::Acquire) {
            return Err("앱을 종료하는 중입니다.".into());
        }
        if !core.loaded {
            return Err("녹화 예약을 불러오는 중입니다.".into());
        }
        if core.config.revision != revision {
            return Err("예약 상태가 변경됐습니다. 다시 시도해 주세요.".into());
        }
        if let Some(error) = core.fault.clone() {
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
                    let error = volatile(&error);
                    core.fault = Some(error.clone());
                    self.wake.notify_all();
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
        let name = clean(&name, 120);
        if let Some(channel) = next.channels.iter_mut().find(|c| c.channel_id == id) {
            channel.enabled = enabled;
            channel.name = if name.trim().is_empty() {
                id.clone()
            } else {
                name
            };
            channel.generation = channel.generation.checked_add(1).ok_or_else(fault)?;
            if enabled {
                channel.seen.clear();
            }
        } else {
            next.channels.push(Channel {
                channel_id: id.clone(),
                name: if name.trim().is_empty() {
                    id.clone()
                } else {
                    name
                },
                enabled,
                generation: 1,
                seen: vec![],
                failure: None,
            });
        }
        next.revision = next.revision.checked_add(1).ok_or_else(fault)?;
        validate(&next)?;
        if let Err(error) = save_config(&self.directory, &next) {
            if !enabled {
                core.config = next;
                let error = volatile(&error);
                core.fault = Some(error.clone());
                self.wake.notify_all();
                return Err(error);
            }
            return Err(error);
        }
        core.config = next;
        if let Some(observation) = core.observations.get_mut(&id) {
            observation.next = 0;
            observation.error = None;
            observation.preparation_retries.clear();
        }
        drop(core);
        self.wake.notify_all();
        Ok(self.snapshot())
    }
    fn current(&self, core: &Core, attempt: &Attempt) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && core.fault.is_none()
            && core.config.channels.iter().any(|c| {
                c.channel_id == attempt.channel && c.enabled && c.generation == attempt.generation
            })
    }
    /// Short synchronous native admission only. Never hold this across browser
    /// ACK/network work; off/generation updates cannot race the new archive.
    pub(super) fn with_current_owner<T>(&self,owner:&RecordingOwner,begin:impl FnOnce()->Result<T,String>)->Result<T,String>{
        let RecordingOwner::Scheduled{schedule_id,run_id,broadcast_key,generation}=owner else{return Err("예약 녹화 소유권이 아닙니다.".into());};
        let core=self.core.lock().map_err(|_|fault())?;
        if self.stopped.load(Ordering::Acquire)||!core.loaded||core.fault.is_some()||!core.config.channels.iter().any(|channel|
            channel.channel_id==*schedule_id&&channel.enabled&&channel.generation==*generation
            &&channel.seen.iter().any(|intent|intent.video==*broadcast_key&&intent.run==*run_id)){
            return Err("예약 녹화 시작이 취소됐습니다.".into());
        }
        begin()
    }
    /// Durable ownership intent precedes arm. This does not wait for browser ACK.
    fn reserve(&self, attempt: &Attempt) -> Result<RecordingOwner, String> {
        let mut core = self.core.lock().map_err(|_| fault())?;
        if !self.current(&core, attempt) {
            return Err("예약 시작이 취소됐습니다.".into());
        }
        if core
            .config
            .channels
            .iter()
            .find(|c| c.channel_id == attempt.channel)
            .is_none_or(|c| c.seen.iter().any(|i| i.video == attempt.video))
        {
            return Err("이미 처리한 방송입니다.".into());
        }
        let mut next = core.config.clone();
        let channel = next
            .channels
            .iter_mut()
            .find(|c| c.channel_id == attempt.channel)
            .ok_or_else(fault)?;
        if channel.seen.len() >= MAX_SEEN {
            return Err("녹화 예약 기록이 가득 찼습니다. 예약을 껐다 켜 주세요.".into());
        }
        channel.seen.push(Intent {
            video: attempt.video.clone(),
            run: attempt.run.clone(),
        });
        next.revision = next.revision.checked_add(1).ok_or_else(fault)?;
        save_config(&self.directory, &next)?;
        core.config = next;
        Ok(RecordingOwner::Scheduled {
            schedule_id: attempt.channel.clone(),
            run_id: attempt.run.clone(),
            broadcast_key: attempt.video.clone(),
            generation: attempt.generation,
        })
    }
    fn persist_failure(&self, core: &mut Core, id: &str, error: Option<String>) {
        let mut next = core.config.clone();
        let Some(channel) = next.channels.iter_mut().find(|c| c.channel_id == id) else {
            return;
        };
        let error = error.map(|s| clean(&s, 400));
        if channel.failure == error {
            return;
        }
        channel.failure = error;
        next.revision = next.revision.saturating_add(1);
        if let Err(error) = save_config(&self.directory, &next) {
            core.fault = Some(error);
        }
        core.config = next;
    }
    fn service_running(&self, host: &dyn Host) {
        let runs: Vec<_> = self
            .core
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .running
            .iter()
            .map(|(video, run)| (video.clone(), run.clone()))
            .collect();
        for (video, run) in runs {
            let should_stop =
                {
                    let core = self.core.lock().unwrap_or_else(|p| p.into_inner());
                    let generation = match run.capture.owner {
                        RecordingOwner::Scheduled { generation, .. } => generation,
                        _ => 0,
                    };
                    !core.config.channels.iter().any(|c| {
                        c.channel_id == run.channel && c.enabled && c.generation == generation
                    })
                };
            let stop_error = if should_stop && !run.stopping {
                host.stop(&run.capture).err()
            } else {
                None
            };
            let state = host.state(&run.capture);
            let mut core = self.core.lock().unwrap_or_else(|p| p.into_inner());
            if core
                .running
                .get(&video)
                .is_none_or(|current| current.capture.job_id != run.capture.job_id)
            {
                continue;
            }
            match state {
                Ok(polled) if polled.current.session_id!=run.capture.session_id||polled.current.owner!=run.capture.owner => {
                    core.running.get_mut(&video).unwrap().error=Some("예약 녹화 파일의 소유권이 일치하지 않습니다.".into());
                }
                Ok(polled) if !polled.state.active => {
                    core.running.remove(&video);
                    if let Some(error) = polled.state.error {
                        self.persist_failure(&mut core, &run.channel, Some(error));
                    }
                }
                Ok(polled) => {
                    let state=polled.state;
                    if state.state == "recording" {
                        self.persist_failure(&mut core, &run.channel, None);
                    }
                    let current = core.running.get_mut(&video).unwrap();
                    current.capture=polled.current;
                    current.state = state.state;
                    current.error = stop_error.clone().or(state.error);
                    current.stopping |= should_stop && stop_error.is_none();
                }
                Err(error) => core.running.get_mut(&video).unwrap().error = Some(error),
            }
        }
    }
    fn cycle(&self, host: &dyn Host, time: u64) {
        let cycle_started = std::time::Instant::now();
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        self.service_running(host);
        let selected = {
            let mut core = self.core.lock().unwrap_or_else(|p| p.into_inner());
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
                let busy = core
                    .admissions
                    .values()
                    .any(|(id, _)| id == &channel.channel_id);
                let observation = core
                    .observations
                    .entry(channel.channel_id.clone())
                    .or_default();
                if !busy && observation.next <= time {
                    observation.next = time.saturating_add(POLL);
                    selected = Some(channel.clone());
                    core.cursor = index + 1;
                    break;
                }
            }
            selected
        };
        let Some(channel) = selected else { return };
        let response = host.observe(&channel.channel_id);
        let attempts = {
            let mut core = self.core.lock().unwrap_or_else(|p| p.into_inner());
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
            observation.checked = time;
            match response {
                Err(error) => {
                    observation.live = "unknown".into();
                    observation.errors = observation.errors.saturating_add(1);
                    observation.error = Some(clean(&error, 400));
                    observation.next =
                        time.saturating_add(POLL.saturating_mul(1u64 << observation.errors.min(4)));
                    return;
                }
                Ok(observed) => {
                    if observed.identity.channel_id != channel.channel_id {
                        observation.live = "unknown".into();
                        observation.error = Some("채널 식별자가 일치하지 않습니다.".into());
                        return;
                    }
                    observation.live = if !observed.videos.is_empty() {
                        "live"
                    } else if observed.complete {
                        "offline"
                    } else {
                        "unknown"
                    }
                    .into();
                    observation.videos = observed.videos;
                    observation.errors = 0;
                    observation
                        .preparation_retries
                        .retain(|video, _| observation.videos.contains(video));
                    observation.error = observation.videos.iter().find_map(|video| {
                        observation
                            .preparation_retries
                            .get(video)
                            .map(|retry| retry.error.clone())
                    });
                }
            }
            let videos = observation.videos.clone();
            let current = core
                .config
                .channels
                .iter()
                .find(|c| c.channel_id == channel.channel_id)
                .unwrap();
            let candidates: Vec<_> = videos
                .into_iter()
                .filter(|v| {
                    discovery::video_id(v)
                        && !current.seen.iter().any(|i| &i.video == v)
                        && !core.running.contains_key(v)
                        && !core.admissions.values().any(|(_, active)| active == v)
                        && core
                            .observations
                            .get(&channel.channel_id)
                            .and_then(|o| o.preparation_retries.get(v))
                            .is_none_or(|retry| retry.next <= time)
                })
                .take(4)
                .collect();
            candidates
                .into_iter()
                .map(|video| {
                    let attempt = Attempt {
                        channel: channel.channel_id.clone(),
                        generation: channel.generation,
                        video,
                        run: uuid::Uuid::new_v4().to_string(),
                    };
                    core.admissions.insert(
                        attempt.run.clone(),
                        (attempt.channel.clone(), attempt.video.clone()),
                    );
                    attempt
                })
                .collect::<Vec<_>>()
        };
        let _pending = PendingBatch {
            scheduler: self,
            runs: attempts.iter().map(|a| a.run.clone()).collect(),
        };
        for attempt in attempts {
            let _admission = Admission {
                scheduler: self,
                run: attempt.run.clone(),
            };
            {
                let core = self.core.lock().unwrap_or_else(|p| p.into_inner());
                if !self.current(&core, &attempt) {
                    continue;
                }
            }
            let result = host.start(self, &attempt);
            // ACK may race off, revision change or shutdown. Stop only the exact
            // scheduled owner, outside core lock; keep its drain visible.
            let stale = {
                let core = self.core.lock().unwrap_or_else(|p| p.into_inner());
                !self.current(&core, &attempt)
            };
            let stop_error = if stale {
                result
                    .as_ref()
                    .ok()
                    .and_then(|capture| host.stop(capture).err())
            } else {
                None
            };
            let mut core = self.core.lock().unwrap_or_else(|p| p.into_inner());
            match result {
                Ok(capture) => {
                    if let Some(observation) = core.observations.get_mut(&attempt.channel) {
                        observation.preparation_retries.remove(&attempt.video);
                        observation.error = observation.videos.iter().find_map(|video| {
                            observation
                                .preparation_retries
                                .get(video)
                                .map(|retry| retry.error.clone())
                        });
                    }
                    core.running.insert(
                        attempt.video.clone(),
                        Running {
                            channel: attempt.channel.clone(),
                            capture,
                            state: "resolving".into(),
                            stopping: stale && stop_error.is_none(),
                            error: stop_error,
                        },
                    );
                }
                Err(error) => {
                    if stale {
                        continue;
                    }
                    let observation = core
                        .observations
                        .entry(attempt.channel.clone())
                        .or_default();
                    match error {
                        StartFailure::Preparation(error) => {
                            let retry = observation
                                .preparation_retries
                                .entry(attempt.video.clone())
                                .or_default();
                            retry.failures = retry.failures.saturating_add(1);
                            retry.error = clean(&error, 400);
                            let delay = POLL
                                .saturating_mul(1u64 << retry.failures.saturating_sub(1).min(4));
                            // Preparation can itself take 45s. Backoff starts after it,
                            // not at the stale metadata polling timestamp.
                            retry.next = time
                                .saturating_add(cycle_started.elapsed().as_millis() as u64)
                                .saturating_add(delay);
                            observation.error = Some(retry.error.clone());
                        }
                        StartFailure::Busy => {
                            observation.preparation_retries.remove(&attempt.video);
                            observation.error = observation.videos.iter().find_map(|video| {
                                observation
                                    .preparation_retries
                                    .get(video)
                                    .map(|retry| retry.error.clone())
                            });
                        }
                        StartFailure::Reservation(error) => {
                            observation.preparation_retries.remove(&attempt.video);
                            observation.error = Some(clean(&error, 400));
                        }
                        StartFailure::Reserved(error) => {
                            observation.preparation_retries.remove(&attempt.video);
                            observation.error = Some(clean(&error, 400));
                            self.persist_failure(&mut core, &attempt.channel, Some(error));
                        }
                    }
                }
            }
        }
    }
}
fn validate(config: &Config) -> Result<(), String> {
    if config.version != 1
        || config.revision == 0
        || config.revision > 9_007_199_254_740_000
        || config.channels.len() > 128
        || config.channels.iter().filter(|c| c.enabled).count() > 32
    {
        return Err(fault());
    }
    let mut ids = HashSet::new();
    for channel in &config.channels {
        if !discovery::channel_id(&channel.channel_id)
            || !ids.insert(&channel.channel_id)
            || channel.generation == 0
            || channel.name.chars().count() > 120
            || channel.name.chars().any(char::is_control)
            || channel
                .failure
                .as_ref()
                .is_some_and(|s| s.chars().count() > 400 || s.chars().any(char::is_control))
            || channel.seen.len() > MAX_SEEN
        {
            return Err(fault());
        }
        let mut videos = HashSet::new();
        for intent in &channel.seen {
            if !discovery::video_id(&intent.video)
                || !videos.insert(&intent.video)
                || uuid::Uuid::parse_str(&intent.run).is_err()
            {
                return Err(fault());
            }
        }
    }
    Ok(())
}
fn read_config(directory: &Path) -> Result<Config, String> {
    storage::safe_path(directory)?;
    let path = directory.join(FILE);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(_) => return Err(fault()),
        Ok(_) => {}
    }
    if storage::regular(&path)?.len() > MAX_BYTES {
        return Err(fault());
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(|_| fault())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fault())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(fault());
    }
    let config = serde_json::from_slice(&bytes).map_err(|_| fault())?;
    validate(&config)?;
    Ok(config)
}
fn save_config(directory: &Path, config: &Config) -> Result<(), String> {
    validate(config)?;
    storage::safe_path(directory)?;
    fs::create_dir_all(directory).map_err(|_| fault())?;
    let target = directory.join(FILE);
    if target.exists() {
        storage::regular(&target)?;
    }
    let temporary = directory.join(format!("{}.youtube-auto.tmp", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec(config).map_err(|_| fault())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(fault());
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| fault())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| fault())?;
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
            .map_err(|_| fault())?;
        }
    }
    #[cfg(not(windows))]
    fs::rename(temporary, target).map_err(|_| fault())?;
    Ok(())
}

fn trusted(window: &tauri::Webview) -> Result<tauri::AppHandle, String> {
    let url = window.url().map_err(|_| "앱 창 확인 실패")?;
    if !crate::frontend_origin::trusted_main(
        window.label(),
        &url,
        cfg!(debug_assertions),
        window.app_handle().config().build.dev_url.as_ref(),
    ) {
        return Err("Atsumi 기본 창에서만 사용할 수 있습니다.".into());
    }
    Ok(window.app_handle().clone())
}
#[tauri::command]
pub async fn youtube_auto_record_snapshot(window: tauri::Webview) -> Result<AutoSnapshot, String> {
    let app = trusted(&window)?;
    Ok(app
        .state::<crate::interface::commands::AppState>()
        .youtube_auto_recorder()
        .snapshot())
}
#[tauri::command]
pub async fn youtube_auto_record_set(
    window: tauri::Webview,
    channel_id: String,
    name: String,
    enabled: bool,
    expected_revision: u64,
) -> Result<AutoSnapshot, String> {
    let app = trusted(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::interface::commands::AppState>();
        state.begin_platform_work(|| {
            state
                .youtube_auto_recorder()
                .set(channel_id, name, enabled, expected_revision)
        })
    })
    .await
    .map_err(|_| "녹화 예약을 변경하지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn youtube_auto_record_resolve(
    window: tauri::Webview,
    input: String,
) -> Result<discovery::Identity, String> {
    let _app = trusted(&window)?;
    static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    let _permit = SLOTS
        .try_acquire()
        .map_err(|_| "다른 채널을 확인 중입니다.")?;
    tauri::async_runtime::spawn_blocking(move || discovery::resolve(&input))
        .await
        .map_err(|_| "채널을 확인하지 못했습니다.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "UC4QobU6STFB0P71PMvOGN5A";
    const VIDEO: &str = "abcdefghijk";
    struct FakeHost {
        videos: Mutex<Result<Vec<String>, String>>,
        starts: AtomicUsize,
        stops: AtomicUsize,
        active: AtomicBool,
        capacity: AtomicBool,
        manual: AtomicBool,
        off_in_ack: AtomicBool,
        preparations: AtomicUsize,
        preparation_failures: AtomicUsize,
        reserved_error: AtomicBool,
        rollover: Mutex<Option<ScheduledCapture>>,
        pending: AtomicBool,
        stopped_capture: Mutex<Option<ScheduledCapture>>,
    }
    impl FakeHost {
        fn new() -> Self {
            Self {
                videos: Mutex::new(Ok(vec![VIDEO.into()])),
                starts: AtomicUsize::new(0),
                stops: AtomicUsize::new(0),
                active: AtomicBool::new(true),
                capacity: AtomicBool::new(false),
                manual: AtomicBool::new(false),
                off_in_ack: AtomicBool::new(false),
                preparations: AtomicUsize::new(0),
                preparation_failures: AtomicUsize::new(0),
                reserved_error: AtomicBool::new(false),
                rollover: Mutex::new(None),
                pending: AtomicBool::new(false),
                stopped_capture: Mutex::new(None),
            }
        }
    }
    impl Host for FakeHost {
        fn observe(&self, id: &str) -> Result<Observed, String> {
            Ok(Observed {
                identity: discovery::Identity {
                    channel_id: id.into(),
                    name: "fixture".into(),
                },
                videos: self.videos.lock().unwrap().clone()?,
                complete: true,
            })
        }
        fn start(
            &self,
            scheduler: &YouTubeAutoRecorder,
            attempt: &Attempt,
        ) -> Result<ScheduledCapture, StartFailure> {
            self.preparations.fetch_add(1, Ordering::AcqRel);
            if attempt.video == VIDEO
                && self
                    .preparation_failures
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                        value.checked_sub(1)
                    })
                    .is_ok()
            {
                return Err(StartFailure::Preparation("same failure text".into()));
            }
            if self.capacity.load(Ordering::Acquire) {
                return Err(StartFailure::Preparation("자리 대기".into()));
            }
            let owner = scheduler
                .reserve(attempt)
                .map_err(StartFailure::Reservation)?;
            if self.manual.load(Ordering::Acquire) {
                return Err(StartFailure::Busy);
            }
            if self.reserved_error.load(Ordering::Acquire) {
                return Err(StartFailure::Reserved("same failure text".into()));
            }
            self.starts.fetch_add(1, Ordering::AcqRel);
            if self.off_in_ack.load(Ordering::Acquire) {
                let revision = scheduler.snapshot().revision;
                scheduler
                    .set(attempt.channel.clone(), "fixture".into(), false, revision)
                    .map_err(StartFailure::Reserved)?;
            }
            Ok(ScheduledCapture {
                session_id: uuid::Uuid::new_v4().to_string(),
                job_id: uuid::Uuid::new_v4().to_string(),
                owner,
            })
        }
        fn state(&self, capture: &ScheduledCapture) -> Result<PolledCapture, String> {
            Ok(PolledCapture{current:self.rollover.lock().unwrap().clone().unwrap_or_else(||capture.clone()),state:ScheduledState {
                state: if self.pending.load(Ordering::Acquire){"starting"}else{"recording"}.into(),
                error: None,
                active: self.active.load(Ordering::Acquire)||self.pending.load(Ordering::Acquire),
            }})
        }
        fn stop(&self, capture: &ScheduledCapture) -> Result<(), String> {
            self.stops.fetch_add(1, Ordering::AcqRel);
            *self.stopped_capture.lock().unwrap()=Some(capture.clone());
            Ok(())
        }
    }
    fn fixture() -> (tempfile::TempDir, Arc<YouTubeAutoRecorder>, FakeHost) {
        let directory = tempfile::tempdir().unwrap();
        let service = Arc::new(YouTubeAutoRecorder::new(directory.path().into()));
        service.load();
        (directory, service, FakeHost::new())
    }
    fn enable(service: &YouTubeAutoRecorder) {
        service
            .set(
                CHANNEL.into(),
                "fixture".into(),
                true,
                service.snapshot().revision,
            )
            .unwrap();
    }
    #[test]
    fn follows_same_owner_successor_and_off_targets_current_file() {
        let(_directory,service,host)=fixture();enable(&service);service.cycle(&host,clock());
        let old=service.core.lock().unwrap().running[VIDEO].capture.clone();let mut next=old.clone();next.job_id=uuid::Uuid::new_v4().to_string();
        *host.rollover.lock().unwrap()=Some(next.clone());host.active.store(false,Ordering::Release);host.pending.store(true,Ordering::Release);
        service.service_running(&host);assert_eq!(service.core.lock().unwrap().running[VIDEO].capture.job_id,next.job_id);
        service.set(CHANNEL.into(),"fixture".into(),false,service.snapshot().revision).unwrap();service.service_running(&host);
        assert_eq!(host.stopped_capture.lock().unwrap().as_ref().unwrap().job_id,next.job_id);
        host.pending.store(false,Ordering::Release);service.service_running(&host);assert!(service.core.lock().unwrap().running.is_empty());
    }
    #[test]
    fn successor_with_different_owner_cannot_replace_the_running_capture() {
        let(_directory,service,host)=fixture();enable(&service);service.cycle(&host,clock());
        let old=service.core.lock().unwrap().running[VIDEO].capture.clone();let mut next=old.clone();next.job_id=uuid::Uuid::new_v4().to_string();next.owner=RecordingOwner::Manual;
        *host.rollover.lock().unwrap()=Some(next);service.service_running(&host);
        let core=service.core.lock().unwrap();assert_eq!(core.running[VIDEO].capture.job_id,old.job_id);assert!(core.running[VIDEO].error.is_some());
    }
    #[test]
    fn native_successor_admission_requires_enabled_exact_durable_owner() {
        let(_directory,service,host)=fixture();enable(&service);service.cycle(&host,clock());
        let owner=service.core.lock().unwrap().running[VIDEO].capture.owner.clone();assert_eq!(service.with_current_owner(&owner,||Ok(7)).unwrap(),7);
        let mut wrong=owner.clone();if let RecordingOwner::Scheduled{run_id,..}=&mut wrong{*run_id=uuid::Uuid::new_v4().to_string();}
        assert!(service.with_current_owner(&wrong,||Ok(7)).is_err());
        service.set(CHANNEL.into(),"fixture".into(),false,service.snapshot().revision).unwrap();assert!(service.with_current_owner(&owner,||Ok(7)).is_err());
        enable(&service);assert!(service.with_current_owner(&owner,||Ok(7)).is_err());service.shutdown();assert!(service.with_current_owner(&owner,||Ok(7)).is_err());
    }
    fn retry_at(service: &YouTubeAutoRecorder) -> u64 {
        let core = service.core.lock().unwrap();
        let observation = &core.observations[CHANNEL];
        observation
            .preparation_retries
            .values()
            .map(|retry| retry.next)
            .fold(observation.next, u64::max)
            .saturating_add(1)
    }
    #[test]
    fn default_off_and_favorites_are_not_scheduler_input() {
        let (_dir, service, host) = fixture();
        service.cycle(&host, 1);
        assert!(service.snapshot().channels.is_empty());
        assert_eq!(host.starts.load(Ordering::Acquire), 0);
    }
    #[test]
    fn multiple_current_live_ids_start_once_and_survive_restart_suppression() {
        let (dir, service, host) = fixture();
        *host.videos.lock().unwrap() = Ok(vec![VIDEO.into(), "bcdefghijkl".into()]);
        enable(&service);
        service.cycle(&host, 1);
        assert_eq!(host.starts.load(Ordering::Acquire), 2);
        service.cycle(&host, POLL + 2);
        assert_eq!(host.starts.load(Ordering::Acquire), 2);
        let restored = YouTubeAutoRecorder::new(dir.path().into());
        restored.load();
        restored.cycle(&host, 1);
        assert_eq!(host.starts.load(Ordering::Acquire), 2);
    }
    #[test]
    fn errors_and_empty_listings_never_stop_an_owned_capture() {
        let (_dir, service, host) = fixture();
        enable(&service);
        service.cycle(&host, 1);
        *host.videos.lock().unwrap() = Err("network".into());
        service.cycle(&host, POLL + 2);
        *host.videos.lock().unwrap() = Ok(vec![]);
        service.cycle(&host, POLL * 5);
        assert_eq!(host.stops.load(Ordering::Acquire), 0);
    }
    #[test]
    fn manual_owner_is_not_stopped_or_replaced_after_manual_finish() {
        let (_dir, service, host) = fixture();
        enable(&service);
        host.manual.store(true, Ordering::Release);
        service.cycle(&host, 1);
        assert_eq!(service.snapshot().channels[0].status, "skipped");
        assert!(service.snapshot().channels[0].error.is_none());
        host.manual.store(false, Ordering::Release);
        service.cycle(&host, POLL + 1);
        assert_eq!(host.starts.load(Ordering::Acquire), 0);
        service
            .set(
                CHANNEL.into(),
                "fixture".into(),
                false,
                service.snapshot().revision,
            )
            .unwrap();
        service.cycle(&host, POLL * 2);
        assert_eq!(host.stops.load(Ordering::Acquire), 0);
    }
    #[test]
    fn capacity_wait_is_retryable_but_explicit_off_fences_the_owner() {
        let (_dir, service, host) = fixture();
        enable(&service);
        host.capacity.store(true, Ordering::Release);
        service.cycle(&host, 1);
        assert_eq!(service.snapshot().channels[0].status, "waiting");
        host.capacity.store(false, Ordering::Release);
        let next = retry_at(&service);
        service.cycle(&host, next);
        assert_eq!(host.starts.load(Ordering::Acquire), 1);
        service
            .set(
                CHANNEL.into(),
                "fixture".into(),
                false,
                service.snapshot().revision,
            )
            .unwrap();
        service.cycle(&host, next + 1);
        assert_eq!(host.stops.load(Ordering::Acquire), 1);
    }
    #[test]
    fn preparation_backoff_survives_successful_metadata_without_persisting_intent() {
        let (dir, service, host) = fixture();
        enable(&service);
        host.preparation_failures.store(2, Ordering::Release);
        service.cycle(&host, 1);
        assert!(read_config(dir.path()).unwrap().channels[0].seen.is_empty());
        assert_eq!(service.snapshot().channels[0].status, "waiting");
        let first_retry = retry_at(&service);
        service.cycle(&host, POLL);
        assert_eq!(host.preparations.load(Ordering::Acquire), 1);
        service.cycle(&host, first_retry);
        {
            let core = service.core.lock().unwrap();
            let retry = &core.observations[CHANNEL].preparation_retries[VIDEO];
            assert_eq!(retry.failures, 2);
            assert!(retry.next >= first_retry + POLL * 2);
        }
        // A successful metadata poll cannot erase this per-video backoff/error.
        service.cycle(&host, first_retry + POLL);
        assert_eq!(host.preparations.load(Ordering::Acquire), 2);
        assert_eq!(
            service.snapshot().channels[0].error.as_deref(),
            Some("same failure text")
        );
        service.cycle(&host, retry_at(&service));
        assert_eq!(host.starts.load(Ordering::Acquire), 1);
        assert_eq!(read_config(dir.path()).unwrap().channels[0].seen.len(), 1);
        assert!(service.core.lock().unwrap().observations[CHANNEL]
            .preparation_retries
            .is_empty());
    }
    #[test]
    fn failed_preparation_does_not_block_a_different_live_video() {
        let (dir, service, host) = fixture();
        enable(&service);
        *host.videos.lock().unwrap() = Ok(vec![VIDEO.into(), "bcdefghijkl".into()]);
        host.preparation_failures.store(1, Ordering::Release);
        service.cycle(&host, 1);
        assert_eq!(host.starts.load(Ordering::Acquire), 1);
        let seen = &read_config(dir.path()).unwrap().channels[0].seen;
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].video, "bcdefghijkl");
        service.cycle(&host, retry_at(&service));
        assert_eq!(host.starts.load(Ordering::Acquire), 2);
    }
    #[test]
    fn ambiguous_post_reservation_failure_never_retries_even_after_restart() {
        let (dir, service, host) = fixture();
        enable(&service);
        host.reserved_error.store(true, Ordering::Release);
        service.cycle(&host, 1);
        assert_eq!(service.snapshot().channels[0].status, "failed");
        assert_eq!(read_config(dir.path()).unwrap().channels[0].seen.len(), 1);
        host.reserved_error.store(false, Ordering::Release);
        service.cycle(&host, POLL * 20);
        let restored = YouTubeAutoRecorder::new(dir.path().into());
        restored.load();
        restored.cycle(&host, POLL * 40);
        assert_eq!(host.preparations.load(Ordering::Acquire), 1);
        assert_eq!(host.starts.load(Ordering::Acquire), 0);
    }
    #[test]
    fn preparation_backoff_is_bounded_and_explicit_off_cancels_retry() {
        let (_dir, service, host) = fixture();
        enable(&service);
        host.preparation_failures.store(50, Ordering::Release);
        let mut time = 1;
        for failure in 1..=7 {
            service.cycle(&host, time);
            let core = service.core.lock().unwrap();
            let retry = &core.observations[CHANNEL].preparation_retries[VIDEO];
            assert_eq!(retry.failures, failure);
            assert!(retry.next >= time + POLL * (1u64 << (failure - 1).min(4)));
            assert!(retry.next < time + POLL * 17);
            drop(core);
            time = retry_at(&service);
        }
        service
            .set(
                CHANNEL.into(),
                "fixture".into(),
                false,
                service.snapshot().revision,
            )
            .unwrap();
        service.cycle(&host, time + POLL * 20);
        assert_eq!(host.preparations.load(Ordering::Acquire), 7);
        assert_eq!(host.starts.load(Ordering::Acquire), 0);
        assert!(service.core.lock().unwrap().observations[CHANNEL]
            .preparation_retries
            .is_empty());
    }
    #[test]
    fn off_during_ack_stops_only_the_returned_exact_capture_without_deadlock() {
        let (_dir, service, host) = fixture();
        enable(&service);
        host.off_in_ack.store(true, Ordering::Release);
        service.cycle(&host, 1);
        assert_eq!(host.starts.load(Ordering::Acquire), 1);
        assert_eq!(host.stops.load(Ordering::Acquire), 1);
        assert!(!service.snapshot().channels[0].enabled);
        assert!(service.active_ids().is_empty());
    }
    #[test]
    fn stale_revision_generation_and_shutdown_cannot_reserve() {
        let (_dir, service, _host) = fixture();
        enable(&service);
        assert!(service
            .set(CHANNEL.into(), "fixture".into(), false, 1)
            .is_err());
        let attempt = Attempt {
            channel: CHANNEL.into(),
            generation: 1,
            video: VIDEO.into(),
            run: uuid::Uuid::new_v4().to_string(),
        };
        service
            .set(
                CHANNEL.into(),
                "fixture".into(),
                false,
                service.snapshot().revision,
            )
            .unwrap();
        assert!(service.reserve(&attempt).is_err());
        service.shutdown();
        assert!(service
            .set(
                CHANNEL.into(),
                "fixture".into(),
                true,
                service.snapshot().revision
            )
            .is_err());
    }
    #[test]
    fn corrupt_config_is_preserved_without_scheduling() {
        let (dir, service, host) = fixture();
        enable(&service);
        fs::write(dir.path().join(FILE), b"broken").unwrap();
        let restored = YouTubeAutoRecorder::new(dir.path().into());
        restored.load();
        restored.cycle(&host, 1);
        assert!(restored.snapshot().error.is_some());
        assert_eq!(fs::read(dir.path().join(FILE)).unwrap(), b"broken");
        assert_eq!(host.starts.load(Ordering::Acquire), 0);
    }
    #[test]
    fn damaged_storage_off_is_volatile_but_fences_pending_start() {
        let (dir, service, host) = fixture();
        enable(&service);
        service.cycle(&host, 1);
        fs::remove_file(dir.path().join(FILE)).unwrap();
        fs::create_dir(dir.path().join(FILE)).unwrap();
        let error = service
            .set(
                CHANNEL.into(),
                "fixture".into(),
                false,
                service.snapshot().revision,
            )
            .unwrap_err();
        assert!(error.contains("이번 실행"));
        assert!(!service.snapshot().channels[0].enabled);
        service.cycle(&host, POLL + 1);
        assert_eq!(host.stops.load(Ordering::Acquire), 1);
    }
    #[test]
    fn workers_count_before_spawn_and_shutdown_wait_does_not_claim_drain() {
        let (_dir, service, _) = fixture();
        let worker = service.register().unwrap();
        service.shutdown();
        assert!(service.register().is_none());
        assert!(!service.active_ids().is_empty());
        assert!(service.shutdown_and_wait(Duration::from_millis(1)).is_err());
        drop(worker);
        assert!(service.shutdown_and_wait(Duration::from_millis(5)).is_ok());
    }
    #[test]
    fn pending_batch_cleans_unentered_admissions_after_panic() {
        let (_dir, service, _) = fixture();
        service
            .core
            .lock()
            .unwrap()
            .admissions
            .insert("pending".into(), (CHANNEL.into(), VIDEO.into()));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = PendingBatch {
                scheduler: &service,
                runs: vec!["pending".into()],
            };
            panic!("fixture");
        }));
        assert!(service.active_ids().is_empty());
    }
}
