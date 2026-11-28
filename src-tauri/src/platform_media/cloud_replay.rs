//! Verified remote-video chat capabilities. Never opens or serves local video.
//! Contract: requirements-audit/20261117-cloud-goal-resume/sidecar-lease-pre-edit.json.
use super::*;
use crate::cloud_recording::replay::CloudReplayAuthorization;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const UNAVAILABLE: &str = "이 보관 영상과 저장된 채팅 기록의 연결을 확인하지 못했습니다.";
const CLOSED: &str = "채팅 재생이 종료됐습니다. 다시 열어 주세요.";
const CHANGED: &str = "저장된 채팅 기록이 변경됐습니다. 다시 열어 주세요.";
const MAX_SESSIONS: usize = 8;
const MAX_LEGACY: u64 = 128 * 1024 * 1024;
#[cfg(test)]
#[path = "cloud_replay_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "cloud_replay_actual_test.rs"]
mod actual_tests;
#[cfg(all(test, windows))]
#[path = "cloud_replay_receipt_actual_test.rs"]
mod receipt_actual_tests;

pub(super) struct Session {
    pub(super) library_id: String,
    provider: String,
    directory: PathBuf,
    original_job: MediaJob,
    _owner_channel_id: String,
    _cohort_id: String,
    closed: AtomicBool,
    read: Mutex<Pins>,
    _lease: UploadLease,
}
struct Pins {
    _parents: Vec<File>,
    controls: Vec<(PathBuf, Option<storage::SidecarPin>)>,
    payloads: Vec<storage::SidecarPin>,
    assets: HashMap<String, Option<storage::SidecarPin>>,
    metadata_unavailable: bool,
}
fn hash_text(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
fn owned_job(state: &State, authorization: &CloudReplayAuthorization) -> Result<MediaJob, String> {
    let source = &authorization.source;
    if state.shutting_down
        || !matches!(source.provider.as_str(), "soop" | "youtube")
        || source.part_index.is_some()
        || authorization.input_parts
        || !uuid::Uuid::parse_str(&source.recording_id)
            .is_ok_and(|value| value.to_string() == source.recording_id)
        || authorization.original_inputs.len() != 1
        || !hash_text(&authorization.cohort_id)
        || authorization.owner_channel_id.is_empty()
        || authorization.owner_channel_id.len() > 128
        || !authorization.matches_recording_generation("")
    {
        return Err(UNAVAILABLE.into());
    }
    let original = &authorization.original_inputs[0];
    if original.index != 0
        || original.part_index.is_some()
        || original.media_offset_seconds != 0.0
        || original.bytes == 0
        || !hash_text(&original.sha256)
        || !original.duration_seconds.is_finite()
        || original.duration_seconds <= 0.0
    {
        return Err(UNAVAILABLE.into());
    }
    let job = state
        .jobs
        .get(&source.recording_id)
        .filter(|job| {
            job.state == "completed"
                && job.reference.provider == source.provider
                && !state.cancel.contains_key(&source.recording_id)
        })
        .ok_or(UNAVAILABLE)?;
    let item = job.library.as_ref().ok_or(UNAVAILABLE)?;
    if job.job_id != source.recording_id
        || item.library_id != job.job_id
        || item.reference != job.reference
        || job.reference.canonical_url().is_err()
        || item.sha256 != original.sha256
        || item.bytes != original.bytes
        || !item.duration.is_finite()
        || (item.duration - original.duration_seconds).abs() > 0.001
        || original.modified_nanos.is_some_and(|value| {
            item.modified_millis == 0 || value / 1_000_000 != item.modified_millis as u128
        })
    {
        return Err(UNAVAILABLE.into());
    }
    Ok(job.clone())
}
fn same_job(left: &MediaJob, right: &MediaJob) -> bool {
    let (Some(a), Some(b)) = (left.library.as_ref(), right.library.as_ref()) else {
        return false;
    };
    left.job_id == right.job_id
        && left.state == "completed"
        && right.state == "completed"
        && left.download_root == right.download_root
        && left.reference == right.reference
        && a.library_id == b.library_id
        && a.reference == b.reference
        && a.sha256 == b.sha256
        && a.bytes == b.bytes
        && a.duration == b.duration
        && a.modified_millis == b.modified_millis
        && a.revision == b.revision
}
impl Pins {
    fn open(directory: &Path, job: &MediaJob) -> Result<Self, String> {
        let parents = chat_status::pin_parents(&directory.join("chat-timeline.json"))?;
        let mut pins = Self {
            _parents: parents,
            controls: Vec::new(),
            payloads: Vec::new(),
            assets: HashMap::new(),
            metadata_unavailable: false,
        };
        for (name, limit) in [
            ("chat-timeline.json", 4096),
            ("youtube-raw-anchor.json", 4096),
            ("chat-status.json", 16 * 1024),
            ("chat-status.final.json", 16 * 1024),
            ("chat-parts.jsonl", chat_parts::CATALOG_BYTES),
            ("channel-profile.json", 180 * 1024),
            ("chat-assets/manifest.jsonl", 2048 * 512),
        ] {
            let path = directory.join(name);
            let file = storage::SidecarPin::open(&path, limit)?;
            pins.controls.push((path, file));
        }
        if directory.join("chat-assets").exists() {
            pins._parents.extend(chat_status::pin_parents(
                &directory.join("chat-assets/manifest.jsonl"),
            )?);
        }
        let timeline = &mut pins.controls[0].1;
        if let Some(timeline) = timeline {
            let mut bytes = Vec::new();
            timeline
                .file
                .seek(SeekFrom::Start(0))
                .map_err(|_| CHANGED)?;
            (&mut timeline.file)
                .take(4097)
                .read_to_end(&mut bytes)
                .map_err(|_| CHANGED)?;
            let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| CHANGED)?;
            if job.reference.kind == "live" && value["version"] != 2 {
                let end = value["firstSegmentCommittedAtMs"].as_i64();
                let duration = value["firstSegmentDurationMs"].as_i64();
                if value["timelineQuality"] != "receive-time-approximate"
                    || !end.is_some_and(|n| (1_000_000_000_000..=9_000_000_000_000).contains(&n))
                    || !duration.is_some_and(|n| (1..=120000).contains(&n))
                {
                    return Err("원격 영상에 맞출 채팅 시간 기록이 없습니다.".into());
                }
            }
        } else if job.reference.kind == "live" {
            return Err("원격 영상에 맞출 채팅 시간 기록이 없습니다.".into());
        }
        if let Some(parts) =
            chat_parts::replay_source(directory, &job.reference.provider, &job.job_id)?
        {
            for part in &parts.events {
                let path = directory.join(chat_parts::part_name(
                    chat_parts::Stream::Events,
                    part.sequence,
                )?);
                let pin =
                    storage::SidecarPin::open(&path, chat_parts::PART_BYTES)?.ok_or(UNAVAILABLE)?;
                if pin.length() != part.bytes {
                    return Err(CHANGED.into());
                }
                pins.payloads.push(pin);
            }
            // Raw-source display enrichment is optional. Its existing reader
            // reports unavailable metadata instead of losing normalized chat.
            for part in &parts.source {
                let path = directory.join(chat_parts::part_name(
                    chat_parts::Stream::Source,
                    part.sequence,
                )?);
                match storage::SidecarPin::open(&path, u64::MAX) {
                    Ok(Some(pin)) => pins.payloads.push(pin),
                    _ => pins.metadata_unavailable = true,
                }
            }
        } else {
            let names: &[&str] = if job.reference.provider == "soop" {
                &["chat.events.jsonl", "chat.soop.jsonl"]
            } else {
                &[
                    "chat.events.jsonl",
                    "chat.live_chat.json",
                    "chat.live_chat.json.part",
                ]
            };
            let mut present = false;
            for name in names {
                let path = directory.join(name);
                let file = storage::SidecarPin::open(&path, MAX_LEGACY)?;
                present = file.is_some();
                pins.controls.push((path, file));
                if present {
                    break;
                }
            }
            if !present {
                return Err("저장된 채팅 기록을 찾을 수 없습니다.".into());
            }
            let raw = directory.join("chat.source.jsonl");
            match storage::SidecarPin::open(&raw, u64::MAX) {
                Ok(Some(pin)) => pins.payloads.push(pin),
                Ok(None) => pins.controls.push((raw, None)),
                Err(_) => pins.metadata_unavailable = true,
            }
        }
        pins.valid()?;
        Ok(pins)
    }
    fn valid(&self) -> Result<(), String> {
        for (path, pin) in &self.controls {
            match pin {
                Some(pin) => pin.valid()?,
                None => match fs::symlink_metadata(path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    _ => return Err(CHANGED.into()),
                },
            }
        }
        // Windows payload handles exclude writes/deletes for the whole lease.
        // Do not reopen/stat512 parts for every playback tick. The replay index
        // still validates each event part digest when first consuming its data.
        #[cfg(not(windows))]
        for pin in &self.payloads {
            pin.valid()?;
        }
        for pin in self.assets.values().flatten() {
            pin.valid()?;
        }
        Ok(())
    }
}
impl Session {
    fn valid(&self, service: &PlatformMediaService, pins: &Pins) -> Result<(), String> {
        if self.closed.load(Ordering::Acquire) {
            return Err(CLOSED.into());
        }
        let state = service.inner.state.lock().map_err(|_| CLOSED)?;
        if state.shutting_down
            || !state
                .jobs
                .get(&self.library_id)
                .is_some_and(|job| same_job(job, &self.original_job))
        {
            return Err(CHANGED.into());
        }
        drop(state);
        pins.valid()
    }
}
impl PlatformMediaService {
    /// Uses the same persisted source identity as chat admission, without video I/O.
    pub(crate) fn cloud_capture_warning(
        &self,
        authorization: &CloudReplayAuthorization,
    ) -> Result<Option<String>, String> {
        let state = self.inner.state.lock().map_err(|_| UNAVAILABLE)?;
        let job = owned_job(&state, authorization)?;
        Ok(job.library.as_ref().and_then(|item| item.capture_outcome.as_ref())
            .and_then(|outcome| outcome.warning()).map(str::to_owned))
    }
    pub(crate) fn validate_cloud_chat_source(
        &self,
        authorization: &CloudReplayAuthorization,
    ) -> Result<(), String> {
        let state = self.inner.state.lock().map_err(|_| UNAVAILABLE)?;
        owned_job(&state, authorization).map(|_| ())
    }
    pub(crate) fn open_cloud_chat(
        &self,
        authorization: &CloudReplayAuthorization,
    ) -> Result<String, String> {
        let (job, lease) = {
            let state = self.inner.state.lock().map_err(|_| UNAVAILABLE)?;
            let job = owned_job(&state, authorization)?;
            let mut leases = self.inner.upload_leases.lock().map_err(|_| UNAVAILABLE)?;
            if leases.values().sum::<usize>() >= 16 {
                return Err("열려 있는 보관 영상을 먼저 닫아 주세요.".into());
            }
            *leases.entry(job.job_id.clone()).or_default() += 1;
            let lease = UploadLease {
                registry: self.inner.upload_leases.clone(),
                id: job.job_id.clone(),
            };
            (job, lease)
        };
        let directory = storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id)?;
        let pins = Pins::open(&directory, &job)?;
        let session = Arc::new(Session {
            library_id: job.job_id.clone(),
            provider: job.reference.provider.clone(),
            directory,
            original_job: job,
            _owner_channel_id: authorization.owner_channel_id.clone(),
            _cohort_id: authorization.cohort_id.clone(),
            closed: AtomicBool::new(false),
            read: Mutex::new(pins),
            _lease: lease,
        });
        let token = uuid::Uuid::new_v4().to_string();
        {
            let state = self.inner.state.lock().map_err(|_| UNAVAILABLE)?;
            let current = owned_job(&state, authorization)?;
            if !same_job(&current, &session.original_job) {
                return Err(CHANGED.into());
            }
            let mut sessions = self.inner.cloud_chat.lock().map_err(|_| UNAVAILABLE)?;
            if sessions.len() >= MAX_SESSIONS {
                return Err("열려 있는 보관 영상을 먼저 닫아 주세요.".into());
            }
            sessions.insert(token.clone(), session);
        }
        // Reuse the actual bounded parser/anchor/proof validation. Registration
        // precedes the read so sibling close cannot cancel this new index.
        if let Err(error) = self.cloud_chat_page(&token, 0, None, 1) {
            self.close_cloud_chat(&token);
            return Err(error);
        }
        Ok(token)
    }
    fn cloud_session(&self, token: &str) -> Result<Arc<Session>, String> {
        if !uuid::Uuid::parse_str(token).is_ok_and(|value| value.to_string() == token) {
            return Err(CLOSED.into());
        }
        self.inner
            .cloud_chat
            .lock()
            .map_err(|_| CLOSED)?
            .get(token)
            .cloned()
            .ok_or(CLOSED.into())
    }
    pub(crate) fn cloud_chat_page(
        &self,
        token: &str,
        position_ms: u64,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<replay_chat::ReplayChatPage, String> {
        let session = self.cloud_session(token)?;
        let pins = session.read.lock().map_err(|_| CLOSED)?;
        session.valid(self, &pins)?;
        let read = if pins.metadata_unavailable && session.provider == "soop" {
            replay_chat::read_page_for_job_without_raw_metadata
        } else {
            replay_chat::read_page_for_job
        };
        let result = read(
            &session.directory,
            &session.provider,
            &session.library_id,
            position_ms,
            cursor,
            limit,
        );
        session.valid(self, &pins)?;
        result
    }
    pub(crate) fn cloud_chat_profile(&self, token: &str) -> Result<Option<String>, String> {
        let session = self.cloud_session(token)?;
        let pins = session.read.lock().map_err(|_| CLOSED)?;
        session.valid(self, &pins)?;
        let result = profile::read(&session.directory);
        session.valid(self, &pins)?;
        result
    }
    pub(crate) fn cloud_chat_asset(
        &self,
        token: &str,
        asset_id: &str,
    ) -> Result<live_chat::ChatAssetRead, String> {
        if !chat_assets::valid_id(asset_id) {
            return Err("이모티콘 식별자가 올바르지 않습니다.".into());
        }
        let session = self.cloud_session(token)?;
        let mut pins = session.read.lock().map_err(|_| CLOSED)?;
        session.valid(self, &pins)?;
        if !pins.assets.contains_key(asset_id) {
            if pins.assets.len() >= 2048 {
                return Err("열람한 이모티콘이 표시 한도를 넘었습니다.".into());
            }
            let file = storage::SidecarPin::open(
                &session.directory.join("chat-assets").join(asset_id),
                1024 * 1024,
            )?;
            pins.assets.insert(asset_id.into(), file);
        }
        if pins.assets.get(asset_id).is_some_and(Option::is_none) {
            return Ok(chat_assets::ChatAssetRead::local(None));
        }
        let result = chat_assets::replay_asset(&session.directory, asset_id)
            .map(chat_assets::ChatAssetRead::local);
        session.valid(self, &pins)?;
        result
    }
    pub(crate) fn close_cloud_chat(&self, token: &str) {
        let session = self
            .inner
            .cloud_chat
            .lock()
            .ok()
            .and_then(|mut sessions| sessions.remove(token));
        if let Some(session) = session {
            session.closed.store(true, Ordering::Release);
            // Close waits only for this capability's bounded read. Late work
            // cannot return data or recreate an index after final cancellation.
            let _read = session
                .read
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            self.release_chat_if_unused(&session.library_id, &session.directory);
        }
    }
    pub(super) fn release_chat_if_unused(&self, library_id: &str, directory: &Path) {
        let playback = self
            .inner
            .playback
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let sessions = self
            .inner
            .cloud_chat
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !playback
            .values()
            .any(|session| session.library_id == library_id)
            && !sessions
                .values()
                .any(|session| session.library_id == library_id)
        {
            replay_chat::release_directory(directory);
        }
    }
    pub(super) fn shutdown_cloud_chat(&self) {
        let sessions: Vec<_> = self
            .inner
            .cloud_chat
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .drain()
            .map(|(_, value)| value)
            .collect();
        for session in &sessions {
            session.closed.store(true, Ordering::Release);
        }
        for session in sessions {
            let _read = session
                .read
                .lock()
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}
