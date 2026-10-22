//! One receiver, two consumers: local HLS viewing and verified archive jobs.
use super::*;
use std::{collections::HashSet, io::Write, path::Path, time::Instant};

const MAX_LIVE: usize = 4;
const RESERVE: u64 = 1024 * 1024 * 1024;
const MAX_SEGMENT: u64 = 32 * 1024 * 1024;
#[derive(Default)]
pub(super) struct LiveRegistry {
    pub(super) official: HashMap<String, Arc<super::encoded::OfficialReceiver>>,
    sessions: HashMap<String, Arc<LiveSession>>,
    leases: HashMap<String, String>,
    requests: HashMap<String, String>,
}
impl LiveRegistry {
    pub(super) fn legacy_conflict(&self, reference: &MediaRef) -> bool {
        self.sessions.values().any(|s| {
            super::encoded::same_reference(&s.reference, reference)
                && !s.finished.load(Ordering::Acquire)
        })
    }
    pub(super) fn legacy_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| !s.finished.load(Ordering::Acquire))
            .count()
    }
    fn official_count(&mut self) -> usize {
        self.official.retain(|_, receiver| receiver.is_active());
        self.official.len()
    }
    fn reject_official(&self, reference: &MediaRef) -> Result<(), String> {
        if self
            .official
            .values()
            .any(|r| super::encoded::same_reference(&r.reference, reference) && r.is_active())
        {
            return Err(
                "원본 플레이어에서 시청 중인 방송입니다. 같은 플레이어에서 녹화해 주세요.".into(),
            );
        }
        Ok(())
    }
}
struct LiveSession {
    id: String,
    reference: MediaRef,
    directory: PathBuf,
    download_root: PathBuf,
    stop: Arc<AtomicBool>,
    finished: AtomicBool,
    status: Mutex<Status>,
    chat: Arc<live_chat::SharedChat>,
}
struct Status {
    phase: String,
    error: Option<String>,
    title: String,
    channel: String,
    last_sequence: Option<u64>,
    received: u64,
    recording: Option<Recorder>,
    broadcast_key: Option<String>,
    profile: Option<profile::Profile>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RecordingOwner {
    Manual,
    Scheduled {
        schedule_id: String,
        run_id: String,
        broadcast_key: String,
        generation: u64,
    },
}
impl RecordingOwner {
    pub(super) fn valid_for(&self, provider: &str) -> bool {
        match self {
            Self::Manual => true,
            Self::Scheduled {
                schedule_id,
                run_id,
                broadcast_key,
                generation,
            } => {
                !schedule_id.is_empty()
                    && schedule_id.len() <= 40
                    && schedule_id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
                    && uuid::Uuid::parse_str(run_id).is_ok()
                    && *generation > 0
                    && match provider {
                        "soop" => !broadcast_key.is_empty()
                            && broadcast_key.len() <= 20
                            && broadcast_key.bytes().all(|b| b.is_ascii_digit()),
                        "youtube" => schedule_id.len() == 24 && schedule_id.starts_with("UC")
                            && broadcast_key.len() == 11
                            && broadcast_key.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')),
                        _ => false,
                    }
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledCapture {
    pub session_id: String,
    pub job_id: String,
    pub owner: RecordingOwner,
}
#[derive(Clone, Debug)]
pub struct ScheduledState {
    pub state: String,
    pub error: Option<String>,
    pub active: bool,
}
struct Recorder {
    job: MediaJob,
    directory: PathBuf,
    next_sequence: u64,
    segments: Vec<(String, f64)>,
    cancel: Arc<AtomicBool>,
    stop_after: Option<u64>,
    owner: RecordingOwner,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSnapshot {
    pub token: String,
    pub url: String,
    pub reference: MediaRef,
    pub state: String,
    pub title: String,
    pub channel_name: String,
    pub received_bytes: u64,
    pub recording_job: Option<String>,
    pub recording_stopping: bool,
    pub chat_status: String,
    pub error: Option<String>,
}
pub(super) struct NetworkPermit(Arc<Inner>);
impl Drop for NetworkPermit {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.network.lock() {
            *active = active.saturating_sub(1);
        }
    }
}

#[derive(Debug)]
struct Segment {
    name: String,
    sequence: u64,
    duration: f64,
}
fn sequence(name: &str) -> Option<u64> {
    let digits = name.strip_prefix('s')?.strip_suffix(".ts")?;
    if digits.len() != 8 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}
fn playlist(bytes: &[u8]) -> Result<Vec<Segment>, String> {
    if bytes.len() > 65536 {
        return Err("라이브 목록이 너무 큽니다.".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "라이브 목록을 읽지 못했습니다.")?;
    if !text.starts_with("#EXTM3U") {
        return Err("라이브 목록 형식이 올바르지 않습니다.".into());
    }
    let mut duration = None;
    let mut result = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("#EXTINF:") {
            duration = value.split(',').next().and_then(|v| v.parse::<f64>().ok());
        } else if !line.is_empty() && !line.starts_with('#') {
            let seq = sequence(line).ok_or("안전하지 않은 라이브 파일 경로입니다.")?;
            let length = duration.take().ok_or("라이브 길이가 없습니다.")?;
            if !length.is_finite() || length <= 0.0 || length > 120.0 {
                return Err("라이브 조각 길이가 올바르지 않습니다.".into());
            }
            result.push(Segment {
                name: line.to_string(),
                sequence: seq,
                duration: length,
            });
        } else if line.starts_with("#EXT-X-KEY") || line.starts_with("#EXT-X-MAP") {
            return Err("지원하지 않는 로컬 스트림 형식입니다.".into());
        }
    }
    if result.len() > 32 {
        return Err("라이브 목록 한도를 넘었습니다.".into());
    }
    Ok(result)
}
fn read_playlist(directory: &Path) -> Result<Vec<Segment>, String> {
    let path = directory.join("index.m3u8");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let meta = storage::regular(&path)?;
    if meta.len() > 65536 {
        return Err("라이브 목록이 너무 큽니다.".into());
    }
    playlist(&fs::read(path).map_err(|_| "라이브 목록을 읽지 못했습니다.")?)
}
fn recovery_segments(directory: &Path) -> Result<Vec<(String, f64)>, String> {
    let path = directory.join("segments.jsonl");
    if storage::regular(&path)?.len() > 16 * 1024 * 1024 {
        return Err("녹화 복구 기록이 너무 큽니다.".into());
    }
    let text = fs::read_to_string(&path).map_err(|_| "녹화 복구 기록을 읽지 못했습니다.")?;
    let mut segments = Vec::new();
    let mut previous = None;
    for line in text.lines() {
        if segments.len() >= 100_000 {
            return Err("녹화 복구 조각 한도를 넘었습니다.".into());
        }
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|_| "녹화 복구 기록이 손상됐습니다. 파일은 보존했습니다.")?;
        let name = value["name"]
            .as_str()
            .ok_or("녹화 복구 파일 이름이 없습니다.")?;
        let seq = sequence(name).ok_or("녹화 복구 파일 경로가 올바르지 않습니다.")?;
        let duration = value["duration"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0 && *v <= 120.0)
            .ok_or("녹화 복구 길이가 올바르지 않습니다.")?;
        if previous.is_some_and(|last| seq != last + 1) {
            return Err("녹화 복구 조각에 공백이 있습니다. 파일은 보존했습니다.".into());
        }
        previous = Some(seq);
        segments.push((name.to_string(), duration));
    }
    if segments.is_empty() {
        return Err("복구할 녹화 조각이 없습니다.".into());
    }
    Ok(segments)
}
fn merge_attempts(directory: &Path) -> Result<Vec<uuid::Uuid>, String> {
    let path = directory.join("merge-attempts.jsonl");
    if !path.exists() {
        return Ok(Vec::new());
    }
    if storage::regular(&path)?.len() > 1024 * 1024 {
        return Err("병합 작업 기록 한도를 넘었습니다.".into());
    }
    let text = fs::read_to_string(path).map_err(|_| "병합 작업 기록을 읽지 못했습니다.")?;
    text.lines()
        .take(4097)
        .map(|line| {
            let id: String =
                serde_json::from_str(line).map_err(|_| "병합 작업 기록이 손상됐습니다.")?;
            uuid::Uuid::parse_str(&id).map_err(|_| "병합 작업 식별자가 올바르지 않습니다.".into())
        })
        .collect::<Result<Vec<_>, _>>()
        .and_then(|ids| {
            if ids.len() > 4096 {
                Err("병합 작업 기록 한도를 넘었습니다.".into())
            } else {
                Ok(ids)
            }
        })
}
fn register_merge_attempt(directory: &Path, id: uuid::Uuid) -> Result<(), String> {
    if merge_attempts(directory)?.len() >= 4096 {
        return Err("병합 재시도 한도를 넘었습니다.".into());
    }
    let mut manifest = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("merge-attempts.jsonl"))
        .map_err(|_| "병합 작업 기록을 열지 못했습니다.")?;
    serde_json::to_writer(&mut manifest, &id.to_string())
        .map_err(|_| "병합 작업 기록을 만들지 못했습니다.")?;
    manifest
        .write_all(b"\n")
        .and_then(|_| manifest.sync_all())
        .map_err(|_| "병합 작업 기록을 보존하지 못했습니다.".into())
}

impl PlatformMediaService {
    pub(super) fn acquire_network(
        &self,
        cancel: &Arc<AtomicBool>,
    ) -> Result<NetworkPermit, String> {
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            {
                let mut active = self
                    .inner
                    .network
                    .lock()
                    .map_err(|_| "영상 수신 상태 오류")?;
                if *active < 2 {
                    *active += 1;
                    return Ok(NetworkPermit(self.inner.clone()));
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    pub fn live_open(
        &self,
        reference: MediaRef,
        request_id: String,
        download_root: PathBuf,
    ) -> Result<LiveSnapshot, String> {
        if reference.kind != "live" {
            return Err("라이브 방송을 선택해 주세요.".into());
        }
        reference.canonical_url()?;
        if uuid::Uuid::parse_str(&request_id).is_err() {
            return Err("요청 식별자가 올바르지 않습니다.".into());
        }
        storage::safe_path(&download_root)?;
        if self
            .inner
            .state
            .lock()
            .map_err(|_| "영상 상태 오류")?
            .shutting_down
        {
            return Err("앱을 종료하는 중입니다.".into());
        }
        // A layout replacement closes old leases asynchronously. Allow only
        // already-stopping sessions a bounded drain window; never exceed four.
        let drain_started = Instant::now();
        let mut registry = loop {
            let registry = self.inner.live.lock().map_err(|_| "라이브 상태 오류")?;
            let ending = registry
                .sessions
                .values()
                .any(|s| s.stop.load(Ordering::Acquire) && !s.finished.load(Ordering::Acquire));
            let running = registry
                .sessions
                .values()
                .filter(|s| !s.finished.load(Ordering::Acquire))
                .count();
            if running < MAX_LIVE || !ending || drain_started.elapsed() >= Duration::from_secs(20) {
                break registry;
            }
            drop(registry);
            std::thread::sleep(Duration::from_millis(100));
        };
        registry.reject_official(&reference)?;
        if let Some(token) = registry.requests.get(&request_id) {
            let token = token.clone();
            drop(registry);
            return self.live_snapshot(&token);
        }
        if registry.leases.len() >= 8 {
            return Err("열려 있는 라이브를 먼저 닫아 주세요.".into());
        }
        let existing = registry
            .sessions
            .values()
            .find(|s| {
                s.reference == reference
                    && !s.stop.load(Ordering::Acquire)
                    && !s.finished.load(Ordering::Acquire)
            })
            .cloned();
        let session = if let Some(session) = existing {
            session
        } else {
            registry
                .sessions
                .retain(|_, session| !session.finished.load(Ordering::Acquire));
            if registry.sessions.len() + registry.official_count() >= MAX_LIVE {
                return Err("라이브는 동시에 네 개까지 재생할 수 있습니다.".into());
            }
            let id = uuid::Uuid::new_v4().to_string();
            let directory = download_root
                .join(if reference.provider == "youtube" {
                    "YouTube"
                } else {
                    "SOOP"
                })
                .join(".live")
                .join(&id);
            storage::safe_path(&directory)?;
            fs::create_dir_all(&directory).map_err(|_| "라이브 임시 폴더를 만들지 못했습니다.")?;
            let session = Arc::new(LiveSession {
                id: id.clone(),
                reference,
                directory,
                download_root,
                stop: Arc::new(AtomicBool::new(false)),
                finished: AtomicBool::new(false),
                chat: live_chat::SharedChat::new(),
                status: Mutex::new(Status {
                    phase: "resolving".into(),
                    error: None,
                    title: String::new(),
                    channel: String::new(),
                    last_sequence: None,
                    received: 0,
                    recording: None,
                    broadcast_key: None,
                    profile: None,
                }),
            });
            registry.sessions.insert(id, session.clone());
            self.spawn_live_receiver(&session)?;
            session
        };
        let token = uuid::Uuid::new_v4().to_string();
        registry.leases.insert(token.clone(), session.id.clone());
        registry.requests.insert(request_id, token.clone());
        drop(registry);
        self.live_snapshot(&token)
    }
    fn live_session(&self, token: &str) -> Result<Arc<LiveSession>, String> {
        if uuid::Uuid::parse_str(token).is_err() {
            return Err("라이브 세션이 올바르지 않습니다.".into());
        }
        let registry = self.inner.live.lock().map_err(|_| "라이브 상태 오류")?;
        let id = registry.leases.get(token).ok_or("종료된 라이브입니다.")?;
        registry
            .sessions
            .get(id)
            .cloned()
            .ok_or("라이브 세션이 없습니다.".into())
    }
    pub(super) fn screenshot_context(
        &self,
        token: &str,
    ) -> Result<screenshots::CaptureContext, String> {
        if uuid::Uuid::parse_str(token).is_err() {
            return Err("라이브 세션이 올바르지 않습니다.".into());
        }
        let registry = self.inner.live.lock().map_err(|_| "라이브 상태 오류")?;
        let id = registry.leases.get(token).ok_or("종료된 라이브입니다.")?;
        let session = registry.sessions.get(id).ok_or("라이브 세션이 없습니다.")?;
        let status = session.status.lock().map_err(|_| "라이브 상태 오류")?;
        if status.phase != "playing"
            || session.stop.load(Ordering::Acquire)
            || session.finished.load(Ordering::Acquire)
        {
            return Err("재생 중인 라이브에서 스크린샷을 저장할 수 있습니다.".into());
        }
        Ok(screenshots::CaptureContext {
            download_root: session.download_root.clone(),
            reference: session.reference.clone(),
            title: status.title.clone(),
            channel_name: status.channel.clone(),
        })
    }
    pub fn live_snapshot(&self, token: &str) -> Result<LiveSnapshot, String> {
        let session = self.live_session(token)?;
        let status = session.status.lock().map_err(|_| "라이브 상태 오류")?;
        Ok(LiveSnapshot {
            token: token.into(),
            url: format!("http://platform-media.localhost/live/{token}/index.m3u8"),
            reference: session.reference.clone(),
            state: status.phase.clone(),
            title: status.title.clone(),
            channel_name: status.channel.clone(),
            received_bytes: status.received,
            recording_job: status.recording.as_ref().map(|r| r.job.job_id.clone()),
            recording_stopping: status
                .recording
                .as_ref()
                .is_some_and(|r| r.stop_after.is_some()),
            chat_status: session.chat.status(),
            error: status.error.clone(),
        })
    }
    pub fn live_close(&self, token: &str) -> Result<(), String> {
        let session = self.live_session(token)?;
        let mut registry = self.inner.live.lock().map_err(|_| "라이브 상태 오류")?;
        registry.leases.remove(token);
        registry.requests.retain(|_, value| value != token);
        let viewed = registry.leases.values().any(|id| id == &session.id);
        drop(registry);
        if !viewed
            && session
                .status
                .lock()
                .map_err(|_| "라이브 상태 오류")?
                .recording
                .is_none()
        {
            session.stop.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub fn live_chat(
        &self,
        token: &str,
        after: u64,
        limit: usize,
    ) -> Result<live_chat::LiveChatPage, String> {
        self.live_session(token)?.chat.snapshot(after, limit)
    }
    pub fn live_chat_asset(
        &self,
        token: &str,
        asset_id: &str,
    ) -> Result<live_chat::ChatAssetRead, String> {
        self.live_session(token)?.chat.asset(asset_id)
    }
    pub fn live_profile(&self, token: &str) -> Result<Option<String>, String> {
        let session = self.live_session(token)?;
        let profile = {
            let status = session.status.lock().map_err(|_| "라이브 상태 오류")?;
            status
                .profile
                .as_ref()
                .map(profile::Profile::bounded_snapshot)
                .transpose()?
        };
        // Decoding/raster/hash validation runs outside both registry/status locks.
        // This is the existing in-memory portrait, never another network request.
        let image = profile
            .as_ref()
            .map(profile::Profile::data_url)
            .transpose()?;
        let current = self.live_session(token)?;
        if !Arc::ptr_eq(&session, &current) {
            return Err("종료된 라이브입니다.".into());
        }
        Ok(image)
    }
    pub fn live_record(&self, token: &str, enabled: bool) -> Result<LiveSnapshot, String> {
        let session = self.live_session(token)?;
        let mut status = session.status.lock().map_err(|_| "라이브 상태 오류")?;
        if enabled {
            if status.recording.is_some() {
                drop(status);
                return self.live_snapshot(token);
            }
            if status.phase != "playing" || session.finished.load(Ordering::Acquire) {
                return Err("라이브 영상이 준비된 뒤 녹화할 수 있습니다.".into());
            }
            let recorder = self.create_recorder(&session, &status, RecordingOwner::Manual)?;
            status.recording = Some(recorder);
        } else if let Some(recorder) = status.recording.as_mut() {
            recorder.stop_after = Some(recorder.next_sequence);
            self.update(&recorder.job.job_id, |job| job.state = "finalizing".into())?;
        }
        drop(status);
        self.live_snapshot(token)
    }
    fn create_recorder(
        &self,
        session: &LiveSession,
        status: &Status,
        owner: RecordingOwner,
    ) -> Result<Recorder, String> {
        let job = MediaJob {
            broadcast_id: status.broadcast_key.clone(),
            job_id: uuid::Uuid::new_v4().to_string(),
            request_id: uuid::Uuid::new_v4().to_string(),
            reference: session.reference.clone(),
            revision: 1,
            state: if status.phase == "playing" {
                "recording"
            } else {
                "resolving"
            }
            .into(),
            title: status.title.clone(),
            channel_name: status.channel.clone(),
            created_at: now(),
            received_bytes: 0,
            total_bytes: None,
            error: None,
            library: None,
            download_root: session.download_root.clone(),
        };
        let directory = storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id)?;
        fs::create_dir_all(&directory).map_err(|_| "녹화 폴더를 만들지 못했습니다.")?;
        if let Some(profile) = &status.profile {
            let _ = profile::save(&directory, profile);
        }
        // A separate immutable owner proof keeps old job DTOs compatible.
        let mut proof = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join("recording-owner.json"))
            .map_err(|_| "녹화 소유 정보를 만들지 못했습니다.")?;
        serde_json::to_writer(&mut proof, &owner)
            .map_err(|_| "녹화 소유 정보를 기록하지 못했습니다.")?;
        proof
            .sync_all()
            .map_err(|_| "녹화 소유 정보를 보관하지 못했습니다.")?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut state = self.inner.state.lock().map_err(|_| "작업 상태 오류")?;
            if state.shutting_down {
                return Err("앱을 종료하는 중입니다.".into());
            }
            if state.jobs.values().filter(|j| j.active()).count()
                >= super::queries::ACTIVE_JOB_LIMIT
            {
                return Err("진행 중인 작업이 많습니다.".into());
            }
            session.chat.attach_recording_archive(
                &directory,
                &job.reference.provider,
                &job.job_id,
            )?;
            if let Err(error) = storage::save_job(&self.inner.journal, &job) {
                let _ = session.chat.detach_archive();
                return Err(error);
            }
            state.cancel.insert(job.job_id.clone(), cancel.clone());
            state.jobs.insert(job.job_id.clone(), job.clone());
        }
        Ok(Recorder {
            job: job.clone(),
            directory: directory.clone(),
            next_sequence: status.last_sequence.map_or(0, |n| n + 1),
            segments: Vec::new(),
            cancel,
            stop_after: None,
            owner,
        })
    }
    fn spawn_live_receiver(&self, session: &Arc<LiveSession>) -> Result<(), String> {
        #[cfg(test)]
        if self.inner.synthetic_live_receiver.load(Ordering::Acquire) {
            return Ok(());
        }
        let worker = self.clone();
        let receiver = session.clone();
        if std::thread::Builder::new()
            .name("platform-live".into())
            .spawn(move || {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker.capture_live(receiver.clone())
                }))
                .is_err()
                {
                    receiver.chat.stop();
                    while !receiver.chat.is_done() {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    let record = {
                        let mut status = receiver.status.lock().unwrap_or_else(|e| e.into_inner());
                        status.phase = "failed".into();
                        status.error =
                            Some("라이브 수신 작업이 중단됐습니다. 원본은 보존했습니다.".into());
                        status.recording.take()
                    };
                    if let Some(record) = record {
                        let _owner = super::lifecycle::JobOwnership::new(
                            worker.clone(),
                            record.job.job_id.clone(),
                            record.cancel.clone(),
                        );
                        let _ = receiver.chat.detach_archive();
                        worker.terminal_error(
                            &record.job.job_id,
                            &record.cancel,
                            "라이브 수신 작업이 중단됐습니다. 원본은 보존했습니다.",
                        );
                    }
                    receiver.finished.store(true, Ordering::Release);
                }
            })
            .is_err()
        {
            session.finished.store(true, Ordering::Release);
            if let Ok(mut status) = session.status.lock() {
                status.phase = "failed".into();
                if let Some(record) = status.recording.take() {
                    self.finish_recording(
                        session,
                        record,
                        Some("라이브 수신을 준비하지 못했습니다.".into()),
                    );
                }
            }
            return Err("라이브 수신을 준비하지 못했습니다.".into());
        }
        Ok(())
    }
    /// Called only inside the native admission gate. No network or waiting here.
    pub fn scheduled_start(
        &self,
        owner: RecordingOwner,
        download_root: PathBuf,
    ) -> Result<ScheduledCapture, String> {
        if !owner.valid_for("soop") {
            return Err("녹화 예약 정보가 올바르지 않습니다.".into());
        }
        let RecordingOwner::Scheduled {
            schedule_id,
            broadcast_key,
            ..
        } = &owner
        else {
            return Err("자동 녹화 요청이 아닙니다.".into());
        };
        storage::safe_path(&download_root)?;
        if self
            .inner
            .state
            .lock()
            .map_err(|_| "영상 상태 오류")?
            .shutting_down
        {
            return Err("앱을 종료하는 중입니다.".into());
        }
        let reference = MediaRef {
            provider: "soop".into(),
            kind: "live".into(),
            external_id: schedule_id.clone(),
        };
        reference.canonical_url()?;
        let mut registry = self.inner.live.lock().map_err(|_| "라이브 상태 오류")?;
        registry.reject_official(&reference)?;
        registry
            .sessions
            .retain(|_, session| !session.finished.load(Ordering::Acquire));
        let existing = registry
            .sessions
            .values()
            .find(|session| session.reference == reference && !session.stop.load(Ordering::Acquire))
            .cloned();
        let created = existing.is_none();
        let session = if let Some(session) = existing {
            session
        } else {
            if registry.sessions.len() + registry.official_count() >= MAX_LIVE {
                return Err("자리 대기".into());
            }
            let id = uuid::Uuid::new_v4().to_string();
            let directory = download_root.join("SOOP/.live").join(&id);
            storage::safe_path(&directory)?;
            fs::create_dir_all(&directory).map_err(|_| "라이브 임시 폴더를 만들지 못했습니다.")?;
            Arc::new(LiveSession {
                id,
                reference,
                directory,
                download_root,
                stop: Arc::new(AtomicBool::new(false)),
                finished: AtomicBool::new(false),
                chat: live_chat::SharedChat::new(),
                status: Mutex::new(Status {
                    phase: "resolving".into(),
                    error: None,
                    title: String::new(),
                    channel: String::new(),
                    last_sequence: None,
                    received: 0,
                    recording: None,
                    broadcast_key: None,
                    profile: None,
                }),
            })
        };
        let mut status = session.status.lock().map_err(|_| "라이브 상태 오류")?;
        if let Some(record) = &status.recording {
            if record.owner == owner {
                return Ok(ScheduledCapture {
                    session_id: session.id.clone(),
                    job_id: record.job.job_id.clone(),
                    owner,
                });
            }
            return Err("이미 녹화 중입니다.".into());
        }
        if matches!(status.phase.as_str(), "failed" | "ended") {
            return Err("라이브 수신을 마무리하는 중입니다.".into());
        }
        // An already-playing receiver must be the observed broadcast, not a later
        // program under the same channel ID. Resolving receivers validate below.
        if status
            .broadcast_key
            .as_ref()
            .is_some_and(|key| key != broadcast_key)
        {
            return Err("방송이 변경됐습니다. 다음 확인을 기다립니다.".into());
        }
        let recorder = self.create_recorder(&session, &status, owner.clone())?;
        let job_id = recorder.job.job_id.clone();
        status.recording = Some(recorder);
        drop(status);
        if created {
            registry
                .sessions
                .insert(session.id.clone(), session.clone());
            self.spawn_live_receiver(&session)?;
        }
        Ok(ScheduledCapture {
            session_id: session.id.clone(),
            job_id,
            owner,
        })
    }
    pub fn scheduled_state(&self, capture: &ScheduledCapture) -> Result<ScheduledState, String> {
        let state = self.inner.state.lock().map_err(|_| "작업 상태 오류")?;
        let job = state
            .jobs
            .get(&capture.job_id)
            .ok_or("녹화 작업을 찾을 수 없습니다.")?;
        Ok(ScheduledState {
            state: job.state.clone(),
            error: job.error.clone(),
            active: job.active(),
        })
    }
    pub fn scheduled_stop(&self, capture: &ScheduledCapture) -> Result<(), String> {
        let registry = self.inner.live.lock().map_err(|_| "라이브 상태 오류")?;
        let Some(session) = registry.sessions.get(&capture.session_id).cloned() else {
            return Ok(());
        };
        let viewed = registry.leases.values().any(|id| id == &session.id);
        let mut status = session.status.lock().map_err(|_| "라이브 상태 오류")?;
        let Some(record) = status
            .recording
            .as_mut()
            .filter(|record| record.owner == capture.owner && record.job.job_id == capture.job_id)
        else {
            return Ok(());
        };
        record.stop_after = Some(record.next_sequence);
        self.update(&record.job.job_id, |job| job.state = "finalizing".into())?;
        // No viewer: stopping the receiver finalizes the committed archive at once.
        // A viewer retains the receiver, taking one final committed segment.
        if !viewed {
            session.stop.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub(super) fn live_active_ids(&self) -> Vec<String> {
        let registry = self.inner.live.lock().unwrap_or_else(|e| e.into_inner());
        registry
            .sessions
            .values()
            .filter(|s| !s.finished.load(Ordering::Acquire))
            .map(|s| format!("platform-live:{}", s.id))
            .chain(
                registry
                    .official
                    .values()
                    .filter(|r| r.is_active())
                    .map(|r| format!("platform-official:{}", r.id)),
            )
            .collect()
    }
    pub(super) fn live_shutdown(&self) {
        if let Ok(registry) = self.inner.live.lock() {
            for session in registry.sessions.values() {
                session.stop.store(true, Ordering::Release);
            }
            for receiver in registry.official.values() {
                receiver.close();
            }
        }
    }
    fn capture_live(&self, session: Arc<LiveSession>) {
        let result = self.receive_live(&session);
        session.chat.stop();
        // Keep the session active until its chat-only worker has released its
        // process/socket and drained pending bytes into the owned archive sink.
        let chat_drain = Instant::now();
        while !session.chat.is_done() {
            if chat_drain.elapsed() > Duration::from_secs(30) {
                if let Ok(mut status) = session.status.lock() {
                    status.error = Some(
                        "채팅 수신 작업이 종료되지 않았습니다. 기록을 보존하며 종료를 기다립니다."
                            .into(),
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let clean_buffer = result.is_ok();
        if let Ok(mut status) = session.status.lock() {
            status.phase = if result.is_err() { "failed" } else { "ended" }.into();
            status.error = result
                .as_ref()
                .err()
                .filter(|e| e.as_str() != "cancelled")
                .cloned();
            if let Some(recorder) = status.recording.take() {
                let error = result.err();
                drop(status);
                self.finish_recording(&session, recorder, error);
            }
        }
        session.finished.store(true, Ordering::Release);
        session.chat.cleanup_source(&session.directory);
        // Watch-only files are disposable; archive copies have separate owned names.
        if clean_buffer {
            if let Ok(entries) = fs::read_dir(&session.directory) {
                for entry in entries.filter_map(Result::ok) {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if sequence(&name).is_some()
                        || matches!(name.as_str(), "index.m3u8" | "index.m3u8.tmp")
                    {
                        if storage::regular(&entry.path()).is_ok() {
                            let _ = fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }
        // Remove only this UUID leaf if empty (including cancelled-before-start).
        // Never recursively delete a failed buffer or its provider/root directory.
        if storage::safe_path(&session.directory).is_ok() {
            let _ = fs::remove_dir(&session.directory);
        }
    }
    fn receive_live(&self, session: &Arc<LiveSession>) -> Result<(), String> {
        // The live registry admits at most four independent sessions. Do not hold
        // the two VOD worker permits for the lifetime of an unbounded broadcast.
        session.status.lock().map_err(|_| "라이브 상태 오류")?.phase = "resolving".into();
        for (tool, expected) in [
            (
                &self.inner.tools.yt_dlp,
                "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a",
            ),
            (
                &self.inner.tools.deno,
                "e020f3e232bd16e33768dee528e5983349c962952051ced0a5d58ad42f5d9b33",
            ),
        ] {
            if storage::hash(tool, &session.stop)? != expected {
                return Err("영상 도구 검증에 실패했습니다.".into());
            }
        }
        let canonical = session.reference.canonical_url()?;
        let mut args = self.tool_args();
        args.extend([
            "--simulate".into(),
            "--dump-single-json".into(),
            "--".into(),
            canonical.clone(),
        ]);
        let result = process::run(
            &self.inner.tools.yt_dlp,
            &args,
            &session.directory,
            &session.stop,
            Duration::from_secs(90),
            4 * 1024 * 1024,
        )?;
        if !result.success {
            return Err(classify_failure(&result.stderr));
        }
        let metadata: serde_json::Value =
            serde_json::from_slice(&result.stdout).map_err(|_| "라이브 정보를 읽지 못했습니다.")?;
        let scheduled_id = {
            let status = session.status.lock().map_err(|_| "라이브 상태 오류")?;
            match status.recording.as_ref().map(|record| &record.owner) {
                Some(RecordingOwner::Scheduled { broadcast_key, .. }) => {
                    Some(broadcast_key.clone())
                }
                _ => None,
            }
        };
        let (binding, profile) = if session.reference.provider == "soop" {
            let observation = crate::soop::observe_channel(&session.reference.external_id)?;
            if observation.status != "live" || observation.restricted {
                return Err("현재 공개 방송을 확인할 수 없습니다.".into());
            }
            let binding = broadcast::Binding::preflight(
                &session.reference,
                &metadata,
                observation.broadcast_key.as_deref(),
                scheduled_id.as_deref(),
            )?;
            let profile = if session.stop.load(Ordering::Acquire) {
                None
            } else {
                observation
                    .recording_profile_url
                    .as_deref()
                    .and_then(crate::soop::recording_portrait)
                    .and_then(|url| profile::Profile::from_data_url(&url))
            };
            (binding, profile)
        } else {
            let binding = broadcast::Binding::preflight(
                &session.reference,
                &metadata,
                None,
                scheduled_id.as_deref(),
            )?;
            // Optional identity resolves before playing. Failure never blocks the
            // stream and a video cover is never substituted for its channel.
            let profile = metadata["channel_id"]
                .as_str()
                .and_then(|channel| {
                    crate::youtube::recording_portrait(
                        &session.reference.external_id,
                        channel,
                        &session.stop,
                    )
                })
                .and_then(|data| profile::Profile::from_data_url(&data));
            (binding, profile)
        };
        if session.stop.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        {
            let mut status = session.status.lock().map_err(|_| "라이브 상태 오류")?;
            if let Some(Recorder {
                owner:
                    RecordingOwner::Scheduled {
                        broadcast_key: expected,
                        ..
                    },
                ..
            }) = &status.recording
            {
                if binding.id() != expected {
                    return Err(broadcast::IDENTITY_ERROR.into());
                }
            }
            // This is only preflight. The confirmed key stays absent until a
            // committed segment from the identically guarded receiver exists.
            status.profile = profile;
            status.title = clean(metadata["title"].as_str(), 300);
            status.channel = clean(
                metadata["channel"]
                    .as_str()
                    .or(metadata["uploader"].as_str()),
                160,
            );
            if let Some(record) = &status.recording {
                if let Some(profile) = &status.profile {
                    let _ = profile::save(&record.directory, profile);
                }
                self.update(&record.job.job_id, |job| {
                    job.title = status.title.clone();
                    job.channel_name = status.channel.clone();
                })?;
            }
        }
        let mut receive = self.tool_args();
        binding.append_receiver_filter(&mut receive);
        session.chat.start(
            self.clone(),
            session.reference.clone(),
            session.directory.clone(),
            if session.reference.provider == "youtube" {
                super::youtube_chat::Descriptor::from_metadata(&session.reference, &metadata)
            } else {
                Ok(None)
            },
        );
        receive.extend(["--no-progress","--no-part","--hls-use-mpegts","--no-write-info-json","--no-write-thumbnail","--no-write-subs","--format","bv[protocol^=m3u8][vcodec^=avc1][height<=1080]+ba[protocol^=m3u8]/b[protocol^=m3u8][height<=1080]/b[height<=1080]","--output","-","--"].into_iter().map(String::from));
        receive.push(canonical);
        let segment = [
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-protocol_whitelist",
            "pipe",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-map",
            "0:a?",
            "-c",
            "copy",
            "-f",
            "hls",
            "-hls_time",
            "4",
            "-hls_list_size",
            "12",
            "-hls_flags",
            "temp_file",
            "-hls_segment_filename",
            "s%08d.ts",
            "index.m3u8",
        ]
        .map(String::from);
        let started = Instant::now();
        let mut last_advance = Instant::now();
        let mut previous = None;
        let mut inspection_failure: Option<String> = None;
        process::stream_pipeline(
            &self.inner.tools.yt_dlp,
            &receive,
            &self.inner.tools.ffmpeg,
            &segment,
            &session.directory,
            &session.stop,
            || {
                if let Some(error) = &inspection_failure {
                    return Err(error.clone());
                }
                if let Err(error) = self.inspect_segments(session, &binding) {
                    inspection_failure = Some(error.clone());
                    return Err(error);
                }
                let status = session.status.lock().map_err(|_| "라이브 상태 오류")?;
                if status.last_sequence != previous {
                    previous = status.last_sequence;
                    last_advance = Instant::now();
                }
                if previous.is_none() && started.elapsed() > Duration::from_secs(90) {
                    return Err("라이브 영상을 받지 못했습니다. 받은 파일은 보존합니다.".into());
                }
                if previous.is_some() && last_advance.elapsed() > Duration::from_secs(90) {
                    return Err("라이브 영상 수신이 90초 이상 멈췄습니다.".into());
                }
                Ok(())
            },
        )
    }
    fn inspect_segments(
        &self,
        session: &Arc<LiveSession>,
        binding: &broadcast::Binding,
    ) -> Result<(), String> {
        if fs2::available_space(&session.directory)
            .map_err(|_| "저장 공간을 확인하지 못했습니다.")?
            < RESERVE
        {
            return Err(
                "저장 공간이 부족해 라이브 수신을 중단했습니다. 녹화 조각은 보존했습니다.".into(),
            );
        }
        for entry in fs::read_dir(&session.directory)
            .map_err(|_| "라이브 폴더를 확인하지 못했습니다.")?
            .filter_map(Result::ok)
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name
                .strip_suffix(".tmp")
                .is_some_and(|name| sequence(name).is_some())
                && entry
                    .metadata()
                    .map_err(|_| "라이브 파일을 확인하지 못했습니다.")?
                    .len()
                    > MAX_SEGMENT
            {
                return Err(
                    "라이브 조각이 지나치게 커져 수신을 중단했습니다. 받은 파일은 보존했습니다."
                        .into(),
                );
            }
        }
        let segments = read_playlist(&session.directory)?;
        if segments.is_empty() {
            return Ok(());
        }
        let mut status = session.status.lock().map_err(|_| "라이브 상태 오류")?;
        if status
            .broadcast_key
            .as_deref()
            .is_some_and(|id| id != binding.id())
        {
            return Err(broadcast::IDENTITY_ERROR.into());
        }
        if status.broadcast_key.is_none() {
            // Validate an actually committed, nonempty source before recording
            // the identity. Persist before the first archive copy or playing.
            let length = storage::regular(&session.directory.join(&segments[0].name))?.len();
            if length == 0 || length > MAX_SEGMENT {
                return Err("라이브 조각을 확인하지 못했습니다. 받은 파일은 보존합니다.".into());
            }
            if let Some(record) = &status.recording {
                if let RecordingOwner::Scheduled { broadcast_key, .. } = &record.owner {
                    if broadcast_key != binding.id() {
                        return Err(broadcast::IDENTITY_ERROR.into());
                    }
                }
                self.update(&record.job.job_id, |job| {
                    job.broadcast_id = Some(binding.id().into())
                })?;
            }
            status.broadcast_key = Some(binding.id().into());
        }
        let last = segments.last().unwrap().sequence;
        if status.last_sequence != Some(last) {
            for segment in &segments {
                if status.last_sequence.is_none_or(|n| segment.sequence > n) {
                    let length = storage::regular(&session.directory.join(&segment.name))?.len();
                    if length > MAX_SEGMENT {
                        return Err("라이브 조각 한도를 넘었습니다.".into());
                    }
                    status.received = status.received.saturating_add(length);
                }
            }
            status.last_sequence = Some(last);
            status.phase = "playing".into();
        }
        let mut record_error = None;
        let mut finished_record = false;
        if let Some(record) = status.recording.as_mut() {
            if record.cancel.load(Ordering::Acquire) {
                record_error = Some("녹화가 취소됐습니다. 조각은 보존했습니다.".into());
            } else {
                for segment in &segments {
                    if segment.sequence < record.next_sequence {
                        continue;
                    }
                    if segment.sequence != record.next_sequence {
                        record_error =
                            Some("녹화 조각에 공백이 있습니다. 원본을 보존했습니다.".into());
                        break;
                    }
                    let source = session.directory.join(&segment.name);
                    let target = record.directory.join(&segment.name);
                    let length = storage::regular(&source)?.len();
                    if length > MAX_SEGMENT {
                        return Err("녹화 조각 한도를 넘었습니다.".into());
                    }
                    let mut output = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&target)
                        .map_err(|_| "녹화 조각을 보존하지 못했습니다.")?;
                    let mut input =
                        fs::File::open(source).map_err(|_| "라이브 조각을 읽지 못했습니다.")?;
                    std::io::copy(&mut input, &mut output)
                        .and_then(|_| output.sync_all())
                        .map_err(|_| "녹화 조각을 저장하지 못했습니다.")?;
                    let mut manifest = fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(record.directory.join("segments.jsonl"))
                        .map_err(|_| "녹화 복구 정보를 기록하지 못했습니다.")?;
                    let entry =
                        serde_json::json!({"name":segment.name,"duration":segment.duration});
                    serde_json::to_writer(&mut manifest, &entry)
                        .map_err(|_| "녹화 복구 정보를 기록하지 못했습니다.")?;
                    manifest
                        .write_all(b"\n")
                        .and_then(|_| manifest.sync_all())
                        .map_err(|_| "녹화 복구 정보를 보관하지 못했습니다.")?;
                    record
                        .segments
                        .push((segment.name.clone(), segment.duration));
                    record.next_sequence += 1;
                    if record.segments.len() == 1 {
                        let timeline = serde_json::json!({"version":1,"firstSegmentCommittedAtMs":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,"firstSegmentDurationMs":(segment.duration*1000.0).round() as u64,"recordingRequestedAtMs":record.job.created_at.saturating_mul(1000),"timelineQuality":"receive-time-approximate"});
                        let mut file = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(record.directory.join("chat-timeline.json"))
                            .map_err(|_| "채팅 시각 정보를 만들지 못했습니다.")?;
                        serde_json::to_writer(&mut file, &timeline)
                            .map_err(|_| "채팅 시각 정보를 기록하지 못했습니다.")?;
                        file.sync_all()
                            .map_err(|_| "채팅 시각 정보를 보존하지 못했습니다.")?;
                    }
                    self.update(&record.job.job_id, |job| {
                        if job.state == "resolving" {
                            job.state = "recording".into();
                        }
                        job.received_bytes = job.received_bytes.saturating_add(length)
                    })?;
                    if record
                        .stop_after
                        .is_some_and(|last| segment.sequence >= last)
                    {
                        finished_record = true;
                        break;
                    }
                }
            }
        }
        if let Some(error) = record_error {
            if let Some(record) = status.recording.take() {
                self.finish_recording(session, record, Some(error));
            }
        }
        if finished_record {
            if let Some(record) = status.recording.take() {
                self.finish_recording(session, record, None);
            }
        }
        let recording = status.recording.is_some();
        drop(status);
        if !recording
            && !self
                .inner
                .live
                .lock()
                .map_err(|_| "라이브 상태 오류")?
                .leases
                .values()
                .any(|id| id == &session.id)
        {
            session.stop.store(true, Ordering::Release);
        }
        let oldest = segments[0].sequence.saturating_sub(6);
        let keep: HashSet<_> = segments.iter().map(|s| s.name.as_str()).collect();
        for entry in fs::read_dir(&session.directory)
            .map_err(|_| "라이브 폴더를 읽지 못했습니다.")?
            .filter_map(Result::ok)
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            if sequence(&name).is_some_and(|n| n < oldest)
                && !keep.contains(name.as_str())
                && storage::regular(&entry.path()).is_ok()
            {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(())
    }
    fn finish_recording(
        &self,
        session: &Arc<LiveSession>,
        record: Recorder,
        incoming_error: Option<String>,
    ) {
        // Detach synchronously under the chat lock before another recording can
        // attach. The live connection remains session-owned and keeps receiving.
        let chat_result = session.chat.detach_archive();
        let service = self.clone();
        let owner = super::lifecycle::JobOwnership::new(
            service.clone(),
            record.job.job_id.clone(),
            record.cancel.clone(),
        );
        let _spawn = std::thread::Builder::new()
            .name("platform-finalize".into())
            .spawn(move || {
                let _owner = owner;
                let id = record.job.job_id.clone();
                let result = (|| {
                    let detached = chat_result?
                        .ok_or("녹화 채팅 종료 상태를 확인하지 못했습니다. 원본은 보존했습니다.")?;
                    detached.persist_cancellable(
                        &record.directory,
                        &record.job.reference.provider,
                        &id,
                        &record.cancel,
                    )?;
                    if let Some(error) = incoming_error {
                        return Err(error);
                    }
                    service.finalize_recording(&record)
                })();
                if let Err(error) = result {
                    service.chat_aware_terminal_error(&id, &record.cancel, &error);
                }
            });
    }
    fn finalize_recording(&self, record: &Recorder) -> Result<(), String> {
        self.finalize_recording_as(record, false)
    }
    fn legacy_publication_job(&self, record: &Recorder) -> Result<MediaJob, String> {
        const CHANGED: &str = "녹화 마무리 소유권이 변경됐습니다. 원본은 보존했습니다.";
        let expected = &record.job;
        if record.directory != storage::job_dir(&expected.download_root, &expected.reference.provider, &expected.job_id)? {
            return Err(CHANGED.into());
        }
        let state = self.inner.state.lock().map_err(|_| CHANGED)?;
        let current = state.jobs.get(&expected.job_id).ok_or(CHANGED)?;
        if record.cancel.load(Ordering::Acquire) || state.shutting_down
            || !state.cancel.get(&expected.job_id).is_some_and(|known| Arc::ptr_eq(known, &record.cancel))
            || !current.active() || current.library.is_some()
            || current.job_id != expected.job_id || current.request_id != expected.request_id
            || current.reference != expected.reference || current.download_root != expected.download_root
            || current.created_at != expected.created_at
            || expected.broadcast_id.as_ref().is_some_and(|id| current.broadcast_id.as_ref() != Some(id))
            || current.broadcast_id.as_deref().is_some_and(|id| !broadcast::valid(&current.reference, id))
        {
            return Err(CHANGED.into());
        }
        // First-source identity is persisted by inspect_segments, while the
        // Recorder retains its original clone. Refresh only this owned legacy
        // caller; the final publication fence still compares exact identity.
        Ok(current.clone())
    }
    pub(super) fn finalize_encoded_recording(
        &self,
        job: &MediaJob,
        directory: PathBuf,
        cancel: Arc<AtomicBool>,
        segments: Vec<(String, f64)>,
    ) -> Result<(), String> {
        super::encoded::validate_proof(job, &directory)?;
        self.finalize_recording_as(
            &Recorder {
                job: job.clone(),
                directory,
                cancel,
                segments,
                next_sequence: 0,
                stop_after: None,
                owner: RecordingOwner::Manual,
            },
            true,
        )
    }
    fn finalize_recording_as(&self, record: &Recorder, encoded: bool) -> Result<(), String> {
        if record.segments.is_empty() {
            return Err("아직 저장된 라이브 조각이 없습니다.".into());
        }
        let publication_job = if encoded { record.job.clone() } else { self.legacy_publication_job(record)? };
        // Both normal stop and recovery use the same immutable text proof.
        let chat = super::chat_status::recovery_with_cancel(
            &record.directory,
            &record.job.reference.provider,
            &record.job.job_id,
            &record.cancel,
        )?;
        let soop_encoded = encoded && record.job.reference.provider == "soop"
            && record.job.reference.kind == "live";
        let interrupted = super::encoded::soop_finalize::outcome(
            &record.job, &record.directory, &record.cancel,
        )?.is_some();
        let _merge = loop {
            if record.cancel.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            if let Ok(lock) = self.inner.merge.try_lock() {
                break lock;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        if soop_encoded {
            super::encoded::soop_finalize::start(self, &record.job, &record.cancel)?;
        } else {
            self.update(&record.job.job_id, |job| job.state = "finalizing".into())?;
        }
        let already_published = record.directory.join("media.mp4").exists();
        let output_name = if already_published { "media.mp4".to_string() } else {
          super::encoded::soop_finalize::merge(soop_encoded, || {
            if soop_encoded {
                super::encoded::soop_finalize::preflight(
                    self, &record.job, &record.directory, &record.cancel, &record.segments, RESERVE,
                )?;
            }
            let attempt = uuid::Uuid::new_v4();
            let list_name = format!("merge-{attempt}.txt");
            let output_name = format!("merge-{attempt}.mp4");
            register_merge_attempt(&record.directory, attempt)?;
            let list_path = record.directory.join(&list_name);
            let mut list = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(list_path)
                .map_err(|_| "병합 목록을 만들지 못했습니다.")?;
            for (name, _) in &record.segments {
                if !(if encoded {
                    super::encoded::segment_sequence(name).is_some()
                } else {
                    sequence(name).is_some()
                }) {
                    return Err("녹화 파일 이름이 올바르지 않습니다.".into());
                }
                let meta = storage::regular(&record.directory.join(name))?;
                if meta.len() == 0 || meta.len() > MAX_SEGMENT {
                    return Err("녹화 조각 크기가 올바르지 않습니다.".into());
                }
                writeln!(list, "file '{name}'").map_err(|_| "병합 목록을 기록하지 못했습니다.")?;
            }
            list.sync_all()
                .map_err(|_| "병합 목록을 보관하지 못했습니다.")?;
            drop(list);
            let merge = process::run(
                &self.inner.tools.ffmpeg,
                &[
                    "-hide_banner",
                    "-nostdin",
                    "-v",
                    "error",
                    "-n",
                    "-protocol_whitelist",
                    "file,pipe",
                    "-f",
                    "concat",
                    "-safe",
                    "1",
                    "-i",
                    &list_name,
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a?",
                    "-c",
                    "copy",
                    "-movflags",
                    "+faststart",
                    &output_name,
                ]
                .map(String::from),
                &record.directory,
                &record.cancel,
                Duration::from_secs(24 * 3600),
                1024,
            )?;
            Ok(merge.success.then_some(output_name))
          }, || super::encoded::soop_finalize::backoff(self, &record.job, &record.cancel))?
        };
        if soop_encoded {
            super::encoded::soop_finalize::owner(self, &record.job, &record.cancel)?;
        }
        self.verify_publish_from(
            &publication_job,
            &record.cancel,
            &record.directory,
            Some(record.segments.iter().map(|(_, duration)| duration).sum()),
            &output_name,
        )?;
        // Video validation does not authorize deleting chat or unknown files.
        if interrupted || chat.originals_retained {
            return Ok(());
        }
        for (name, _) in &record.segments {
            let path = record.directory.join(name);
            if storage::regular(&path).is_ok() {
                let _ = fs::remove_file(path);
            }
        }
        // Filename shape alone is not ownership. Only durable attempts recorded
        // before starting FFmpeg may be reclaimed; legacy/unknown names remain.
        if let Ok(attempts) = merge_attempts(&record.directory) {
            for id in attempts {
                for extension in ["mp4", "txt"] {
                    let path = record.directory.join(format!("merge-{id}.{extension}"));
                    if storage::regular(&path).is_ok() {
                        let _ = fs::remove_file(path);
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn resume_live_recording(
        &self,
        job: &MediaJob,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        let directory = storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id)?;
        if job.reference.provider == "youtube"
            && super::youtube_spool::has_raw_proof(&directory)
                .map_err(|_| "YouTube 녹화 원본을 확인하지 못했습니다. 받은 파일은 보존했습니다.")?
        {
            let identity = super::encoded::recover_youtube_raw_identity(job, &directory, cancel)?;
            return self.finalize_raw_youtube_recovery(job.clone(), cancel.clone(), identity);
        }
        if directory.join(super::encoded::PROOF).exists() {
            let segments = super::encoded::recovery_segments(job, &directory)?;
            return self.finalize_encoded_recording(job, directory, cancel.clone(), segments);
        }
        let segments = recovery_segments(&directory)?;
        let record = Recorder {
            job: job.clone(),
            directory,
            next_sequence: 0,
            segments,
            cancel: cancel.clone(),
            stop_after: None,
            owner: RecordingOwner::Manual,
        };
        self.finalize_recording(&record)
    }
    pub(super) fn live_file(
        &self,
        token: &str,
        name: &str,
    ) -> Result<(Vec<u8>, &'static str), String> {
        if name != "index.m3u8" && sequence(name).is_none() {
            return Err("라이브 파일 경로가 올바르지 않습니다.".into());
        }
        let session = self.live_session(token)?;
        let path = session.directory.join(name);
        let meta = storage::regular(&path)?;
        let maximum = if name == "index.m3u8" {
            65536
        } else {
            MAX_SEGMENT
        };
        if meta.len() > maximum {
            return Err("라이브 파일이 너무 큽니다.".into());
        }
        let bytes = fs::read(path).map_err(|_| "라이브 파일이 아직 준비되지 않았습니다.")?;
        if name == "index.m3u8" {
            playlist(&bytes)?;
        }
        Ok((
            bytes,
            if name == "index.m3u8" {
                "application/vnd.apple.mpegurl"
            } else {
                "video/mp2t"
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn broadcast_identity_is_durable_before_first_copy_and_manual_record_inherits_it() {
        for fail_journal in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let service = PlatformMediaService::new(
                root.path().join("data"),
                PlatformMediaTools {
                    yt_dlp: root.path().join("never.exe"),
                    deno: root.path().join("never.exe"),
                    ffmpeg: root.path().join("never.exe"),
                    ffprobe: root.path().join("never.exe"),
                },
                None,
            )
            .unwrap();
            service
                .inner
                .synthetic_live_receiver
                .store(true, Ordering::Release);
            let capture = service
                .scheduled_start(
                    RecordingOwner::Scheduled {
                        schedule_id: "fixture".into(),
                        run_id: uuid::Uuid::new_v4().to_string(),
                        broadcast_key: "123".into(),
                        generation: 1,
                    },
                    root.path().join("media"),
                )
                .unwrap();
            let session = service.inner.live.lock().unwrap().sessions[&capture.session_id].clone();
            let directory = session
                .status
                .lock()
                .unwrap()
                .recording
                .as_ref()
                .unwrap()
                .directory
                .clone();
            assert!(service.jobs(None)[0].broadcast_id.is_none());
            assert!(session.status.lock().unwrap().broadcast_key.is_none());
            fs::write(
                session.directory.join("s00000000.ts"),
                b"synthetic-committed-segment",
            )
            .unwrap();
            fs::write(
                session.directory.join("index.m3u8"),
                b"#EXTM3U\n#EXTINF:4,\ns00000000.ts\n",
            )
            .unwrap();
            let binding = broadcast::Binding::preflight(
                &session.reference,
                &serde_json::json!({"id":"123","uploader_id":"fixture","is_live":true}),
                Some("123"),
                Some("123"),
            )
            .unwrap();
            if fail_journal {
                // Only this isolated job's publication target is blocked.
                let job_path = service
                    .inner
                    .journal
                    .join(format!("{}.json", capture.job_id));
                fs::rename(&job_path, job_path.with_extension("saved")).unwrap();
                fs::create_dir(&job_path).unwrap();
                assert!(service.inspect_segments(&session, &binding).is_err());
                assert!(!directory.join("s00000000.ts").exists());
                assert!(session.directory.join("s00000000.ts").exists());
                assert!(service.jobs(None)[0].broadcast_id.is_none());
                assert!(session.status.lock().unwrap().broadcast_key.is_none());
                assert_ne!(session.status.lock().unwrap().phase, "playing");
            } else {
                service.inspect_segments(&session, &binding).unwrap();
                assert!(directory.join("s00000000.ts").exists());
                let durable = storage::load_jobs(&service.inner.journal);
                assert_eq!(durable[0].broadcast_id.as_deref(), Some("123"));
                let mut status = session.status.lock().unwrap();
                // Deliberately stale Recorder.job must not be the publication source.
                assert!(status
                    .recording
                    .as_ref()
                    .unwrap()
                    .job
                    .broadcast_id
                    .is_none());
                let refreshed = service.legacy_publication_job(status.recording.as_ref().unwrap()).unwrap();
                assert_eq!(refreshed.broadcast_id.as_deref(), Some("123"));
                assert_eq!(refreshed.job_id, capture.job_id);
                assert!(status.recording.as_ref().unwrap().job.broadcast_id.is_none());
                status.recording.take();
                session.chat.detach_archive().unwrap();
                let manual = service
                    .create_recorder(&session, &status, RecordingOwner::Manual)
                    .unwrap();
                assert_eq!(manual.job.broadcast_id.as_deref(), Some("123"));
                session.chat.detach_archive().unwrap();
                assert_eq!(storage::load_jobs(&service.inner.journal).len(), 2);
            }
            session.chat.stop();
            session.finished.store(true, Ordering::Release);
            service.shutdown();
        }
    }
    #[test]
    fn legacy_publication_refresh_is_exact_owned_and_never_invents_a_broadcast() {
        let root = tempfile::tempdir().unwrap();
        let never = root.path().join("never.exe");
        let service = PlatformMediaService::new(root.path().join("data"), PlatformMediaTools {
            yt_dlp: never.clone(), deno: never.clone(), ffmpeg: never.clone(), ffprobe: never,
        }, None).unwrap();
        let mut record = Recorder {
            job: MediaJob {
                broadcast_id: None, job_id: uuid::Uuid::new_v4().to_string(),
                request_id: uuid::Uuid::new_v4().to_string(),
                reference: MediaRef { provider: "soop".into(), kind: "live".into(), external_id: "fixture".into() },
                revision: 1, state: "recording".into(), title: "fixture".into(), channel_name: "fixture".into(),
                created_at: 1, received_bytes: 0, total_bytes: None, error: None, library: None,
                download_root: root.path().join("media"),
            },
            directory: PathBuf::new(), next_sequence: 0, segments: Vec::new(),
            cancel: Arc::new(AtomicBool::new(false)), stop_after: None, owner: RecordingOwner::Manual,
        };
        record.directory = storage::job_dir(&record.job.download_root, "soop", &record.job.job_id).unwrap();
        let mut current = record.job.clone();
        current.broadcast_id = Some("123".into()); current.revision = 2;
        storage::save_job(&service.inner.journal, &current).unwrap();
        let journal = fs::read(service.inner.journal.join(format!("{}.json", current.job_id))).unwrap();
        for case in ["bound", "unknown", "same", "different_broadcast", "invalid_broadcast", "replacement", "missing_token",
            "cancel", "shutdown", "completed", "request", "reference", "root", "created", "directory"] {
            record.job.broadcast_id = (case == "same" || case == "different_broadcast").then(|| (if case == "same" { "123" } else { "456" }).to_string());
            record.cancel.store(case == "cancel", Ordering::Release);
            record.directory = storage::job_dir(&record.job.download_root, "soop", &record.job.job_id).unwrap();
            if case == "directory" { record.directory = root.path().join("foreign"); }
            {
                let mut state = service.inner.state.lock().unwrap();
                state.shutting_down = case == "shutdown";
                state.cancel.insert(record.job.job_id.clone(), if case == "replacement" { Arc::new(AtomicBool::new(false)) } else { record.cancel.clone() });
                if case == "missing_token" { state.cancel.remove(&record.job.job_id); }
                let mut changed = current.clone();
                match case {
                    "unknown" => changed.broadcast_id = None,
                    "invalid_broadcast" => changed.broadcast_id = Some("invalid".into()),
                    "completed" => changed.state = "completed".into(),
                    "request" => changed.request_id = uuid::Uuid::new_v4().to_string(),
                    "reference" => changed.reference.external_id = "other".into(),
                    "root" => changed.download_root = root.path().join("other"),
                    "created" => changed.created_at += 1,
                    _ => (),
                }
                state.jobs.insert(record.job.job_id.clone(), changed);
            }
            let before = serde_json::to_vec(&service.inner.state.lock().unwrap().jobs[&record.job.job_id]).unwrap();
            let result = service.legacy_publication_job(&record);
            if matches!(case, "bound" | "same" | "unknown") {
                let refreshed = result.unwrap();
                assert_eq!(refreshed.broadcast_id.as_deref(), if case == "unknown" { None } else { Some("123") });
                assert_eq!(refreshed.revision, 2);
            } else { assert!(result.is_err(), "{case}"); }
            assert_eq!(serde_json::to_vec(&service.inner.state.lock().unwrap().jobs[&record.job.job_id]).unwrap(), before);
            assert_eq!(fs::read(service.inner.journal.join(format!("{}.json", current.job_id))).unwrap(), journal);
        }
        service.shutdown();
    }
    #[test]
    fn live_profile_is_lease_scoped_read_only_and_rejects_closed_or_corrupt_data() {
        let root = tempfile::tempdir().unwrap();
        let service = PlatformMediaService::new(
            root.path().join("data"),
            PlatformMediaTools {
                yt_dlp: root.path().join("no-network.exe"),
                deno: root.path().join("no-deno.exe"),
                ffmpeg: root.path().join("no-ffmpeg.exe"),
                ffprobe: root.path().join("no-ffprobe.exe"),
            },
            None,
        )
        .unwrap();
        let token = uuid::Uuid::new_v4().to_string();
        let session = Arc::new(LiveSession {
            id: uuid::Uuid::new_v4().to_string(),
            reference: MediaRef {
                provider: "soop".into(),
                kind: "live".into(),
                external_id: "fixture".into(),
            },
            directory: root.path().join("never-created"),
            download_root: root.path().join("never-downloaded"),
            stop: Arc::new(AtomicBool::new(false)),
            finished: AtomicBool::new(false),
            chat: live_chat::SharedChat::new(),
            status: Mutex::new(Status {
                phase: "playing".into(),
                error: None,
                title: "Fixture".into(),
                channel: "Channel".into(),
                last_sequence: None,
                received: 0,
                recording: None,
                broadcast_key: None,
                profile: None,
            }),
        });
        {
            let mut registry = service.inner.live.lock().unwrap();
            registry
                .sessions
                .insert(session.id.clone(), session.clone());
            registry.leases.insert(token.clone(), session.id.clone());
        }
        assert!(service.live_profile(&token).unwrap().is_none());
        let capture = service.screenshot_context(&token).unwrap();
        assert_eq!(capture.reference, session.reference);
        assert_eq!(capture.download_root, session.download_root);
        assert_eq!(capture.title, "Fixture");
        assert_eq!(capture.channel_name, "Channel");
        assert!(service.screenshot_context("not-a-token").is_err());
        session.status.lock().unwrap().phase = "resolving".into();
        assert!(service.screenshot_context(&token).is_err());
        session.status.lock().unwrap().phase = "playing".into();
        session.stop.store(true, Ordering::Release);
        assert!(service.screenshot_context(&token).is_err());
        session.stop.store(false, Ordering::Release);
        session.finished.store(true, Ordering::Release);
        assert!(service.screenshot_context(&token).is_err());
        session.finished.store(false, Ordering::Release);
        assert!(service.live_profile("not-a-token").is_err());
        assert!(service
            .live_profile(&uuid::Uuid::new_v4().to_string())
            .is_err());
        let mut output = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(32, 32)
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        let expected = format!(
            "data:image/png;base64,{}",
            STANDARD.encode(output.into_inner())
        );
        let portrait = profile::Profile::from_data_url(&expected).unwrap();
        session.status.lock().unwrap().profile = Some(portrait.clone());
        assert_eq!(service.live_profile(&token).unwrap(), Some(expected));
        assert!(!session.directory.exists());
        assert!(!session.download_root.exists());
        assert!(service.jobs(None).is_empty());
        assert_eq!(session.status.lock().unwrap().received, 0);
        let mut corrupted = serde_json::to_value(portrait).unwrap();
        corrupted["sha256"] = serde_json::Value::String("0".repeat(64));
        session.status.lock().unwrap().profile = Some(serde_json::from_value(corrupted).unwrap());
        assert!(service.live_profile(&token).is_err());
        service.live_close(&token).unwrap();
        assert!(service.live_profile(&token).is_err());
        assert!(service.screenshot_context(&token).is_err());
        session.finished.store(true, Ordering::Release);
        service.shutdown();
    }
    #[test]
    #[ignore = "isolated synthetic media; no provider, account or running app"]
    fn scheduled_capture_without_viewer_shares_segments_and_owner_stop() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_owned();
        let root = repo
            .join(".runtime/platform-auto-record-tests")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let bin = repo
            .join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
        let service = PlatformMediaService::new(
            root.join("data"),
            PlatformMediaTools {
                yt_dlp: root.join("network-must-not-run.exe"),
                deno: root.join("no-deno.exe"),
                ffmpeg: bin.join("ffmpeg.exe"),
                ffprobe: bin.join("ffprobe.exe"),
            },
            None,
        )
        .unwrap();
        service
            .inner
            .synthetic_live_receiver
            .store(true, Ordering::Release);
        let owner = RecordingOwner::Scheduled {
            schedule_id: "fixture".into(),
            run_id: uuid::Uuid::new_v4().to_string(),
            broadcast_key: "123".into(),
            generation: 1,
        };
        let capture = service
            .scheduled_start(owner.clone(), root.join("media"))
            .unwrap();
        let repeated = service
            .scheduled_start(owner.clone(), root.join("media"))
            .unwrap();
        assert_eq!(capture.job_id, repeated.job_id);
        let session = service.inner.live.lock().unwrap().sessions[&capture.session_id].clone();
        assert!(service.inner.live.lock().unwrap().leases.is_empty());
        let generated = process::run(
            &bin.join("ffmpeg.exe"),
            &[
                "-hide_banner",
                "-nostdin",
                "-v",
                "error",
                "-n",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=160x90:r=10",
                "-t",
                "3",
                "-c:v",
                "libopenh264",
                "-threads",
                "1",
                "-f",
                "mpegts",
                "s00000000.ts",
            ]
            .map(String::from),
            &session.directory,
            &Arc::new(AtomicBool::new(false)),
            Duration::from_secs(20),
            1024,
        )
        .unwrap();
        assert!(generated.success);
        fs::write(
            session.directory.join("index.m3u8"),
            "#EXTM3U\n#EXTINF:3,\ns00000000.ts\n",
        )
        .unwrap();
        let binding = broadcast::Binding::preflight(
            &session.reference,
            &serde_json::json!({"id":"123","uploader_id":"fixture","is_live":true}),
            Some("123"),
            Some("123"),
        )
        .unwrap();
        service.inspect_segments(&session, &binding).unwrap();
        let viewer = service
            .live_open(
                session.reference.clone(),
                uuid::Uuid::new_v4().to_string(),
                root.join("media"),
            )
            .unwrap();
        assert!(Arc::ptr_eq(
            &service.live_session(&viewer.token).unwrap(),
            &session
        ));
        assert_eq!(service.live_active_ids().len(), 1);
        service.live_close(&viewer.token).unwrap();
        assert!(!session.stop.load(Ordering::Acquire));
        let mut wrong = capture.clone();
        wrong.owner = RecordingOwner::Manual;
        service.scheduled_stop(&wrong).unwrap();
        assert!(!session.stop.load(Ordering::Acquire));
        service.scheduled_stop(&capture).unwrap();
        assert!(session.stop.load(Ordering::Acquire));
        let record = session.status.lock().unwrap().recording.take().unwrap();
        let directory = record.directory.clone();
        let mut image = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(32, 32)
            .write_to(&mut image, image::ImageFormat::Png)
            .unwrap();
        let profile = profile::Profile::from_data_url(&format!(
            "data:image/png;base64,{}",
            STANDARD.encode(image.into_inner())
        ))
        .unwrap();
        profile::save(&directory, &profile).unwrap();
        assert_eq!(
            storage::hash(&session.directory.join("s00000000.ts"), &record.cancel).unwrap(),
            storage::hash(&directory.join("s00000000.ts"), &record.cancel).unwrap()
        );
        service.finish_recording(&session, record, None);
        session.finished.store(true, Ordering::Release);
        let started = Instant::now();
        let completed = loop {
            let job = service
                .jobs(Some("soop"))
                .into_iter()
                .find(|j| j.job_id == capture.job_id)
                .unwrap();
            if !job.active() && service.active_ids().is_empty() {
                break job;
            }
            assert!(started.elapsed() < Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(completed.state, "completed", "{:?}", completed.error);
        assert_eq!(completed.broadcast_id.as_deref(), Some("123"));
        assert_eq!(
            completed.library.as_ref().unwrap().broadcast_id.as_deref(),
            Some("123")
        );
        assert!(completed.library.unwrap().duration > 2.9);
        assert!(directory.join("media.mp4").is_file());
        assert!(directory.join("chat.events.jsonl").is_file());
        assert!(!directory.join("s00000000.ts").exists());
        assert!(directory.join("recording-owner.json").is_file());
        assert!(service.profile(&capture.job_id).unwrap().is_some());
        assert!(service.active_ids().is_empty());
        service.shutdown();
    }
    #[test]
    fn playlist_accepts_only_own_committed_segments() {
        let valid = b"#EXTM3U\n#EXTINF:4.000,\ns00000001.ts\n";
        let parsed = playlist(valid).unwrap();
        assert_eq!(parsed[0].sequence, 1);
        assert_eq!(parsed[0].duration, 4.0);
        for name in [
            "../secret",
            "https://example.test/x.ts",
            "s00000001.ts.tmp",
            "s000000000.ts",
        ] {
            assert!(playlist(format!("#EXTM3U\n#EXTINF:4,\n{name}\n").as_bytes()).is_err());
        }
    }
    #[test]
    fn live_ids_are_provider_scoped() {
        for id in ["../secret", "https://localhost/", "x?foo"] {
            assert!(MediaRef {
                provider: "soop".into(),
                kind: "live".into(),
                external_id: id.into()
            }
            .canonical_url()
            .is_err());
        }
        assert_eq!(
            MediaRef {
                provider: "soop".into(),
                kind: "live".into(),
                external_id: "afreeca".into()
            }
            .canonical_url()
            .unwrap(),
            "https://play.sooplive.co.kr/afreeca"
        );
    }
    #[test]
    fn recovery_manifest_rejects_gaps_and_external_paths() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("segments.jsonl");
        for text in ["{\"name\":\"../secret.ts\",\"duration\":4}\n","{\"name\":\"s00000001.ts\",\"duration\":4}\n{\"name\":\"s00000003.ts\",\"duration\":4}\n","{\"name\":\"s00000001.ts\",\"duration\":0}\n"]{fs::write(&path,text).unwrap();assert!(recovery_segments(root.path()).is_err());}
        fs::write(&path,"{\"name\":\"s00000004.ts\",\"duration\":4}\n{\"name\":\"s00000005.ts\",\"duration\":4}\n").unwrap();
        assert_eq!(recovery_segments(root.path()).unwrap().len(), 2);
    }
    #[test]
    #[ignore = "isolated synthetic three-second media fixture; no provider or account requests"]
    fn interrupted_recording_retries_only_local_segments() {
        local_recovery_fixture(false);
    }
    #[test]
    #[ignore = "isolated synthetic media and partial chat; no provider or account requests"]
    fn partial_chat_keeps_verified_video_warning_and_all_inputs_on_restart() {
        local_recovery_fixture(true);
    }
    fn local_recovery_fixture(partial: bool) {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let root = repo
            .join(".runtime/platform-media-recovery-tests")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let bin = repo
            .join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
        // Nonexistent network-tool paths prove a recovery does not invoke extraction.
        let tools = PlatformMediaTools {
            yt_dlp: root.join("do-not-start-ytdlp.exe"),
            deno: root.join("do-not-start-deno.exe"),
            ffmpeg: bin.join("ffmpeg.exe"),
            ffprobe: bin.join("ffprobe.exe"),
        };
        let job = MediaJob {
            broadcast_id: None,
            job_id: uuid::Uuid::new_v4().to_string(),
            request_id: uuid::Uuid::new_v4().to_string(),
            reference: MediaRef {
                provider: "youtube".into(),
                kind: "live".into(),
                external_id: "abcdefghijk".into(),
            },
            revision: 1,
            state: "recording".into(),
            title: "Synthetic recovery fixture".into(),
            channel_name: "Test".into(),
            created_at: now(),
            received_bytes: 0,
            total_bytes: None,
            error: None,
            library: None,
            download_root: root.join("media"),
        };
        let directory = storage::job_dir(&job.download_root, "youtube", &job.job_id).unwrap();
        fs::create_dir_all(&directory).unwrap();
        let marker = chat_status::begin(&directory, "youtube", &job.job_id).unwrap();
        fs::write(directory.join("chat.events.jsonl"), b"{}").unwrap();
        chat_status::finish(
            &directory,
            &marker,
            partial.then_some(chat_status::Reason::ArchiveLimit),
            false,
        )
        .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let generated = process::run(
            &tools.ffmpeg,
            &[
                "-hide_banner",
                "-nostdin",
                "-v",
                "error",
                "-n",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=160x90:r=10",
                "-t",
                "3",
                "-c:v",
                "libopenh264",
                "-threads",
                "1",
                "-f",
                "mpegts",
                "s00000000.ts",
            ]
            .map(String::from),
            &directory,
            &cancel,
            Duration::from_secs(20),
            1024,
        )
        .unwrap();
        assert!(generated.success);
        fs::write(
            directory.join("segments.jsonl"),
            "{\"name\":\"s00000000.ts\",\"duration\":3}\n",
        )
        .unwrap();
        fs::write(
            directory.join("chat.fixture.jsonl"),
            b"preserve this synthetic chat fixture\n",
        )
        .unwrap();
        let old_id = uuid::Uuid::new_v4();
        register_merge_attempt(&directory, old_id).unwrap();
        let old_attempt = directory.join(format!("merge-{old_id}.mp4"));
        fs::write(&old_attempt, b"incomplete").unwrap();
        let unrelated = directory.join(format!("merge-{}.mp4", uuid::Uuid::new_v4()));
        fs::write(&unrelated, b"unknown file must survive").unwrap();
        let journal = root.join("data/platform-media/jobs");
        storage::save_job(&journal, &job).unwrap();
        let service = PlatformMediaService::new(root.join("data"), tools.clone(), None).unwrap();
        let interrupted = service.jobs(Some("youtube")).pop().unwrap();
        assert_eq!(interrupted.state, "interrupted");
        service.retry(&job.job_id, interrupted.revision).unwrap();
        let await_completed = |service: &PlatformMediaService| {
            let started = Instant::now();
            loop {
                let item = service
                    .jobs(Some("youtube"))
                    .into_iter()
                    .find(|j| j.job_id == job.job_id)
                    .unwrap();
                if !item.active() && service.active_ids().is_empty() {
                    assert_eq!(item.state, "completed", "{:?}", item.error);
                    break item;
                }
                assert!(started.elapsed() < Duration::from_secs(30));
                std::thread::sleep(Duration::from_millis(100));
            }
        };
        let completed = await_completed(&service);
        assert_eq!(directory.join("s00000000.ts").exists(), partial);
        assert_eq!(old_attempt.exists(), partial);
        assert_eq!(
            completed
                .library
                .as_ref()
                .unwrap()
                .chat_archive
                .as_ref()
                .unwrap()
                .originals_retained,
            partial
        );
        assert!(unrelated.exists());
        assert!(directory.join("chat.fixture.jsonl").exists());
        service.shutdown();
        // Simulate a crash after publishing final media but before journal completion.
        let mut interrupted = completed.clone();
        interrupted.state = "interrupted".into();
        interrupted.library = None;
        interrupted.revision += 1;
        storage::save_job(&journal, &interrupted).unwrap();
        let resumed = PlatformMediaService::new(root.join("data"), tools, None).unwrap();
        resumed.retry(&job.job_id, interrupted.revision).unwrap();
        let verified = await_completed(&resumed);
        assert_eq!(
            verified.library.as_ref().unwrap().chat_archive,
            completed.library.as_ref().unwrap().chat_archive
        );
        assert_eq!(directory.join("s00000000.ts").exists(), partial);
        assert_eq!(
            verified.library.unwrap().sha256,
            completed.library.unwrap().sha256
        );
        assert!(directory.join("chat.fixture.jsonl").exists());
        assert!(resumed.live_active_ids().is_empty());
        resumed.shutdown();
    }
    #[test]
    #[ignore = "explicit short public animal-camera live capture, isolated .runtime output, no account"]
    fn public_live_shared_receiver_roundtrip() {
        let soop_sample = std::env::var("ATSUMI_LIVE_TEST_PROVIDER").is_ok_and(|v| v == "soop");
        let (provider, target_id) = if soop_sample {
            ("soop", "phonics1".to_string())
        } else {
            let page =
                crate::youtube::search::search(crate::youtube::search::YouTubeSearchRequest {
                    query: "Kitkat Playroom live".into(),
                    filter: "live".into(),
                    sort: "relevance".into(),
                    channel: None,
                    cursor: None,
                })
                .unwrap();
            let target=page.items.iter().find(|item|item.is_live == Some(true)&&item.id=="cswFQYam8Ic"&&item.author.eq_ignore_ascii_case("Kitkat Playroom")).expect("The public sample's live state was not confirmed by this search response (absence is unknown, not offline). Do not replace it with a user recording; use the exact-player metadata/native QA for badge-less results.");
            ("youtube", target.id.clone())
        };
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let root = repo
            .join(".runtime/platform-media-live-tests")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let bin = repo
            .join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
        let service = PlatformMediaService::new(
            root.join("data"),
            PlatformMediaTools {
                yt_dlp: repo.join(".runtime/platform-tools/bin/yt-dlp.exe"),
                deno: repo.join(".runtime/platform-tools/bin/deno.exe"),
                ffmpeg: bin.join("ffmpeg.exe"),
                ffprobe: bin.join("ffprobe.exe"),
            },
            None,
        )
        .unwrap();
        let reference = MediaRef {
            provider: provider.into(),
            kind: "live".into(),
            external_id: target_id.clone(),
        };
        let first = service
            .live_open(
                reference.clone(),
                uuid::Uuid::new_v4().to_string(),
                root.join("media"),
            )
            .unwrap();
        let second = service
            .live_open(
                reference,
                uuid::Uuid::new_v4().to_string(),
                root.join("media"),
            )
            .unwrap();
        assert_eq!(
            service.live_active_ids().len(),
            1,
            "viewers must share the same receiver"
        );
        let started = Instant::now();
        loop {
            let snapshot = service.live_snapshot(&first.token).unwrap();
            if snapshot.state == "playing" {
                break;
            }
            if snapshot.state == "failed" || started.elapsed() > Duration::from_secs(150) {
                service.shutdown();
                panic!("public live not ready: {:?}", snapshot.error);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let (manifest, mime) = service.live_file(&first.token, "index.m3u8").unwrap();
        assert_eq!(mime, "application/vnd.apple.mpegurl");
        let committed = playlist(&manifest).unwrap();
        let (segment, _) = service
            .live_file(&second.token, &committed[0].name)
            .unwrap();
        assert!(!segment.is_empty());
        let recording = service.live_record(&first.token, true).unwrap();
        let id = recording.recording_job.unwrap();
        std::thread::sleep(Duration::from_secs(12));
        service.live_record(&first.token, false).unwrap();
        let finish = Instant::now();
        let item = loop {
            let jobs = service.jobs(Some(provider));
            let job = jobs.iter().find(|j| j.job_id == id).unwrap();
            if !job.active() {
                if job.state != "completed" {
                    service.shutdown();
                    panic!("live finalization failed: {:?}", job.error);
                }
                break job.library.clone().unwrap();
            }
            if finish.elapsed() > Duration::from_secs(120) {
                service.shutdown();
                panic!("live finalization timeout");
            }
            std::thread::sleep(Duration::from_millis(500));
        };
        assert!(
            service.live_snapshot(&second.token).unwrap().received_bytes > first.received_bytes
        );
        let saved = service.open(&id).unwrap();
        assert_eq!(saved.item.sha256.len(), 64);
        assert!(service.thumbnail(&id).unwrap().is_some());
        let chat_page = service.live_chat(&second.token, 0, 120).unwrap();
        let session = service.live_session(&first.token).unwrap();
        assert!(Arc::ptr_eq(
            &session.chat,
            &service.live_session(&second.token).unwrap().chat
        ));
        let archive_dir = storage::job_dir(&root.join("media"), provider, &id).unwrap();
        assert!(archive_dir.join("chat.events.jsonl").exists());
        assert!(archive_dir.join("chat.source.jsonl").exists());
        let archive_length = fs::metadata(archive_dir.join("chat.events.jsonl"))
            .unwrap()
            .len();
        let still_receiving = service
            .live_chat(&first.token, chat_page.cursor, 120)
            .unwrap();
        assert!(
            !still_receiving.ended,
            "stopping recording must not stop shared live chat"
        );
        service.close(&saved.token);
        service.live_close(&first.token).unwrap();
        service.live_close(&second.token).unwrap();
        service.shutdown();
        let drained = Instant::now();
        while !service.active_ids().is_empty() {
            assert!(
                drained.elapsed() < Duration::from_secs(20),
                "all receiver processes must drain after shutdown"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!(
            "shared live receiver and verified archive: sample={} duration={} bytes={} root={}",
            target_id,
            item.duration,
            item.bytes,
            root.display()
        );
        eprintln!(
            "shared chat ring_rows={} archive_bytes={} provider={}",
            chat_page.items.len(),
            archive_length,
            provider
        );
    }
}
