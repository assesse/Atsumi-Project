//! Offline, token-scoped replay. Source media and journals are never rewritten.
//! Disk work runs on blocking workers; IPC pages and protocol bodies are bounded.
use std::{
    collections::HashMap,
    fs::{self, File, Metadata},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
    time::{Duration, Instant, SystemTime},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, Webview};

use super::{
    browser_store::{BrowserCaptureStore, BrowserReplaySource},
    model::{ChatMessage, StreamError},
};
use crate::interface::ApiResult;

#[path = "replay_index.rs"]
mod index;
#[path = "replay_parts.rs"]
mod parts;
#[path = "replay_protocol.rs"]
mod protocol;

const MAX_SESSIONS: usize = 4;
const SESSION_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);
pub(super) const MAX_PAGE_ROWS: usize = 200;
pub(super) const MAX_PAGE_BYTES: usize = 1024 * 1024;
pub(super) const MAX_LINE_BYTES: usize = 64 * 1024;

pub(super) fn failure(code: &str, message: &str) -> StreamError {
    StreamError::new(code, message, false)
}
pub(super) fn storage() -> StreamError {
    failure(
        "REPLAY_STORAGE",
        "저장본을 읽지 못했습니다. 파일과 저장 위치를 확인해 주세요.",
    )
}
fn stale() -> StreamError {
    failure(
        "REPLAY_STALE",
        "재생 요청이 만료되거나 파일이 변경됐습니다. 다시 열어 주세요.",
    )
}
fn invalid() -> StreamError {
    failure("REPLAY_INVALID", "유효하지 않은 다시보기 요청입니다.")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileStamp {
    len: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
}
impl FileStamp {
    fn read(file: &File) -> Result<Self, StreamError> {
        let metadata = file.metadata().map_err(|_| storage())?;
        if !regular_metadata(&metadata) {
            return Err(storage());
        }
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().map_err(|_| storage())?,
            created: metadata.created().ok(),
        })
    }
    fn matches(&self, file: &File) -> bool {
        Self::read(file).is_ok_and(|current| current == *self)
    }
}

pub(super) fn regular_metadata(metadata: &Metadata) -> bool {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaySession {
    pub token: String,
    pub recording_id: String,
    pub title: String,
    pub recorded_at: u64,
    pub channel_name: Option<String>,
    pub channel_profile_image: Option<String>,
    pub duration_seconds: f64,
    pub mime_type: String,
    pub chat_status: String,
    pub index_state: String,
    pub sync_quality: String,
    pub manual_offset_seconds: f64,
    pub warnings: Vec<String>,
    pub parts: Vec<parts::ReplayPart>,
    pub recording_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayChatMessage {
    #[serde(flatten)]
    pub message: ChatMessage,
    pub media_time_seconds: f64,
    pub sync_quality: String,
    pub asset_ids: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayChatPage {
    pub generation: u64,
    pub items: Vec<ReplayChatMessage>,
    pub previous_cursor: Option<String>,
    pub next_cursor: Option<String>,
    pub index_state: String,
    pub sync_quality: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayTimelineBucket {
    pub start_seconds: f64,
    pub chat_count: u64,
    pub unique_sender_count: Option<u64>,
    pub viewer_count: Option<u64>,
    pub viewer_sample_count: u64,
    pub viewer_coverage_seconds: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayTimeline {
    pub bucket_seconds: f64,
    pub buckets: Vec<ReplayTimelineBucket>,
    pub viewer_metric_status: String,
    pub index_state: String,
}

#[derive(Clone)]
pub(super) struct IndexStatus {
    state: String,
    warnings: Vec<String>,
    observed_rows: u64,
    approximate_rows: u64,
}
impl Default for IndexStatus {
    fn default() -> Self {
        Self {
            state: "building".into(),
            warnings: vec![],
            observed_rows: 0,
            approximate_rows: 0,
        }
    }
}

pub(super) struct Session {
    token: String,
    recording_id: String,
    recording_root: PathBuf,
    title: String,
    recorded_at: u64,
    channel_name: Option<String>,
    channel_profile_image: Option<String>,
    mime_type: String,
    duration: f64,
    chat_status: String,
    media: Mutex<File>,
    media_stamp: FileStamp,
    media_parts: Vec<parts::PartMedia>,
    recording_active: bool,
    _temporary_files: parts::TemporaryFiles,
    chat: Option<File>,
    chat_stamp: Option<FileStamp>,
    viewers: Option<File>,
    viewer_stamp: Option<FileStamp>,
    timeline: File,
    timeline_stamp: FileStamp,
    fingerprint: String,
    index_path: PathBuf,
    cancel: AtomicBool,
    generation: AtomicU64,
    offset: Mutex<f64>,
    index: Mutex<IndexStatus>,
    opened: Instant,
}
impl Session {
    fn descriptor(&self) -> Result<ReplaySession, StreamError> {
        let status = self.index.lock().map_err(|_| storage())?.clone();
        Ok(ReplaySession {
            token: self.token.clone(),
            recording_id: self.recording_id.clone(),
            title: self.title.clone(),
            recorded_at: self.recorded_at,
            channel_name: self.channel_name.clone(),
            channel_profile_image: self.channel_profile_image.clone(),
            duration_seconds: self.duration,
            mime_type: self.mime_type.clone(),
            chat_status: self.chat_status.clone(),
            index_state: status.state,
            sync_quality: if status.observed_rows > 0 && status.approximate_rows == 0 {
                "observed_media"
            } else {
                "receive_time_approximate"
            }
            .into(),
            manual_offset_seconds: *self.offset.lock().map_err(|_| storage())?,
            warnings: status.warnings,
            parts: self
                .media_parts
                .iter()
                .map(|p| p.descriptor.clone())
                .collect(),
            recording_active: self.recording_active,
        })
    }
    fn valid(&self) -> Result<(), StreamError> {
        if self.cancel.load(Ordering::Acquire)
            || self.opened.elapsed() >= SESSION_LIFETIME
            || !self
                .media_stamp
                .matches(&*self.media.lock().map_err(|_| storage())?)
            || !self.timeline_stamp.matches(&self.timeline)
            || self
                .chat
                .as_ref()
                .zip(self.chat_stamp.as_ref())
                .is_some_and(|(file, stamp)| !stamp.matches(file))
            || self
                .viewers
                .as_ref()
                .zip(self.viewer_stamp.as_ref())
                .is_some_and(|(file, stamp)| !stamp.matches(file))
        {
            return Err(stale());
        }
        Ok(())
    }
    fn accept_generation(&self, generation: u64) -> Result<(), StreamError> {
        if generation > 9_007_199_254_740_991 {
            return Err(invalid());
        }
        let previous = self.generation.fetch_max(generation, Ordering::AcqRel);
        if generation < previous {
            return Err(stale());
        }
        Ok(())
    }
    fn check_generation(&self, generation: u64) -> Result<(), StreamError> {
        self.valid()?;
        if self.generation.load(Ordering::Acquire) != generation {
            return Err(stale());
        }
        Ok(())
    }
    fn empty_page(&self, generation: u64) -> Result<ReplayChatPage, StreamError> {
        let state = self.index.lock().map_err(|_| storage())?.clone();
        Ok(ReplayChatPage {
            generation,
            items: vec![],
            previous_cursor: None,
            next_cursor: None,
            index_state: state.state,
            sync_quality: if state.observed_rows > 0 && state.approximate_rows == 0 {
                "observed_media"
            } else {
                "receive_time_approximate"
            }
            .into(),
            warnings: state.warnings,
        })
    }
}

struct Worker {
    session: Weak<Session>,
    handle: std::thread::JoinHandle<()>,
}

struct Inner {
    store: Arc<Mutex<BrowserCaptureStore>>,
    lifecycle: Mutex<()>,
    data_dir: PathBuf,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    settings: Mutex<()>,
    workers: Mutex<Vec<Worker>>,
    index_locks: Mutex<HashMap<String, Weak<Mutex<()>>>>,
    closing: AtomicBool,
}
impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(sessions) = self.sessions.get_mut() {
            for session in sessions.values() {
                session.cancel.store(true, Ordering::Release);
            }
        }
        if let Ok(workers) = self.workers.get_mut() {
            for worker in workers.iter() {
                if let Some(session) = worker.session.upgrade() {
                    session.cancel.store(true, Ordering::Release);
                }
            }
            for worker in workers.drain(..) {
                let _ = worker.handle.join();
            }
        }
    }
}

#[derive(Clone)]
pub struct ReplayService {
    inner: Arc<Inner>,
}
impl ReplayService {
    /// Initialization does not read recordings, build indexes, or touch a database.
    pub fn new(data_dir: &Path, store: Arc<Mutex<BrowserCaptureStore>>) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                lifecycle: Mutex::new(()),
                data_dir: data_dir.to_owned(),
                sessions: Mutex::new(HashMap::new()),
                settings: Mutex::new(()),
                workers: Mutex::new(Vec::new()),
                index_locks: Mutex::new(HashMap::new()),
                closing: AtomicBool::new(false),
            }),
        }
    }
    pub fn open(&self, recording_id: &str) -> Result<ReplaySession, StreamError> {
        let _lifecycle = self.inner.lifecycle.lock().map_err(|_| storage())?;
        if self.inner.closing.load(Ordering::Acquire) {
            return Err(stale());
        }
        let source = self
            .inner
            .store
            .lock()
            .map_err(|_| storage())?
            .replay_source(recording_id)?;
        let mut sessions = self.inner.sessions.lock().map_err(|_| storage())?;
        if self.inner.closing.load(Ordering::Acquire) {
            return Err(stale());
        }
        sessions.retain(|_, session| {
            let live = !session.cancel.load(Ordering::Acquire)
                && session.opened.elapsed() < SESSION_LIFETIME;
            if !live {
                session.cancel.store(true, Ordering::Release);
            }
            live
        });
        if sessions.len() >= MAX_SESSIONS {
            return Err(failure(
                "REPLAY_LIMIT",
                "다시보기는 동시에 네 개까지 열 수 있습니다.",
            ));
        }
        let mut workers = self.inner.workers.lock().map_err(|_| storage())?;
        let mut index = 0;
        while index < workers.len() {
            if workers[index].handle.is_finished() {
                let _ = workers.swap_remove(index).handle.join();
            } else {
                index += 1;
            }
        }
        if workers.len() >= MAX_SESSIONS {
            return Err(failure(
                "REPLAY_INDEX_BUSY",
                "채팅 인덱스 작업을 마무리하고 있습니다. 잠시 후 다시 열어 주세요.",
            ));
        }
        let root = self.cache_root()?;
        let session = Arc::new(self.make_session(source, &root)?);
        let index_lock = {
            let mut locks = self.inner.index_locks.lock().map_err(|_| storage())?;
            locks.retain(|_, lock| lock.strong_count() > 0);
            let lock = locks
                .get(&session.fingerprint)
                .and_then(Weak::upgrade)
                .unwrap_or_else(|| Arc::new(Mutex::new(())));
            locks.insert(session.fingerprint.clone(), Arc::downgrade(&lock));
            lock
        };
        // The worker owns its slot until actual thread exit, including after close.
        // Same-source builders serialize and reuse the finished immutable index.
        let worker_session = session.clone();
        let handle = std::thread::Builder::new()
            .name("atsumi-replay-index".into())
            .spawn(move || {
                let _index_guard = match index_lock.lock() {
                    Ok(guard) => guard,
                    Err(_) => return,
                };
                match index::build(&worker_session) {
                    Ok(status) => {
                        if let Ok(mut state) = worker_session.index.lock() {
                            *state = status;
                        }
                    }
                    Err(error) => {
                        if let Ok(mut state) = worker_session.index.lock() {
                            state.state = "failed".into();
                            state.warnings = vec![error.message];
                        }
                    }
                }
            })
            .map_err(|_| storage())?;
        workers.push(Worker {
            session: Arc::downgrade(&session),
            handle,
        });
        let descriptor = session.descriptor()?;
        sessions.insert(session.token.clone(), session);
        Ok(descriptor)
    }
    pub(crate) fn prepare_recording_delete(
        &self,
        id: &str,
    ) -> Result<super::browser_store::deletion::DeleteJob, StreamError> {
        let _lifecycle = self.inner.lifecycle.lock().map_err(|_| storage())?;
        let sessions = self.inner.sessions.lock().map_err(|_| storage())?;
        let workers = self.inner.workers.lock().map_err(|_| storage())?;
        if sessions.values().any(|s| s.recording_id == id)
            || workers
                .iter()
                .any(|w| w.session.upgrade().is_some_and(|s| s.recording_id == id))
        {
            return Err(failure(
                "BROWSER_DELETE_REPLAY_BUSY",
                "다시보기 창을 닫고 잠시 후 삭제해 주세요.",
            ));
        }
        self.inner
            .store
            .lock()
            .map_err(|_| storage())?
            .prepare_delete(id)
    }

    pub(crate) fn delete_recording_cache(&self, id: &str) -> Result<(), StreamError> {
        // The store tombstone blocks new replay sessions throughout this I/O.
        let root = self.cache_root()?;
        super::browser_store::deletion::remove_replay_cache(&root, id)?;
        let _settings = self.inner.settings.lock().map_err(|_| storage())?;
        settings_connection(&root)?
            .execute("DELETE FROM offsets WHERE recording_id=?1", [id])
            .map_err(|_| storage())?;
        Ok(())
    }

    fn make_session(
        &self,
        source: BrowserReplaySource,
        root: &Path,
    ) -> Result<Session, StreamError> {
        let BrowserReplaySource {
            recording,
            media,
            mut chat,
            mut timeline,
            duration,
            parts: media_parts,
            combined_timeline,
        } = source;
        let mut temporary_files = parts::TemporaryFiles::default();
        let token = uuid::Uuid::new_v4().simple().to_string();
        let progressive = !media_parts.is_empty();
        if let Some(bytes) = combined_timeline {
            timeline =
                parts::snapshot_bytes(root, &token, "timeline", &bytes, &mut temporary_files.0)?;
        }
        if progressive {
            chat = chat
                .map(|file| {
                    parts::snapshot_file(root, &token, "chat", file, &mut temporary_files.0)
                })
                .transpose()?;
        }
        let media_parts = media_parts
            .into_iter()
            .enumerate()
            .map(|(index, (file, part))| {
                Ok(parts::PartMedia {
                    stamp: FileStamp::read(&file)?,
                    file: Mutex::new(file),
                    descriptor: parts::ReplayPart {
                        index,
                        start_seconds: part.offset,
                        duration_seconds: part.duration,
                    },
                })
            })
            .collect::<Result<Vec<_>, StreamError>>()?;
        let media_stamp = FileStamp::read(&media)?;
        let chat_stamp = chat.as_ref().map(FileStamp::read).transpose()?;
        let timeline_stamp = FileStamp::read(&timeline)?;
        let mut viewers = super::viewer_metrics::open_recording(Path::new(&recording.output_dir))?;
        if progressive {
            viewers = viewers
                .map(|file| {
                    parts::snapshot_file(root, &token, "viewers", file, &mut temporary_files.0)
                })
                .transpose()?;
        }
        let viewer_stamp = viewers.as_ref().map(FileStamp::read).transpose()?;
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(
                format!(
                    "v2|{}|{}|{:?}|{:?}|{:?}|{:?}",
                    recording.id,
                    recording.updated_at,
                    media_stamp,
                    chat_stamp,
                    timeline_stamp,
                    viewer_stamp
                )
                .as_bytes()
            )
        );
        let index_path = root.join(format!("index-v2-{}-{fingerprint}.sqlite", recording.id));
        if progressive {
            // A prefix snapshot is per-lease; never accumulate a full chat index
            // for every refresh. The stable final recording keeps its cache.
            temporary_files.0.push(index_path.clone());
            temporary_files
                .0
                .push(index_path.with_extension("sqlite-journal"));
        }
        let offset = self.read_offset(root, &recording.id)?;
        let chat_status = if recording.capture_chat == Some(false) {
            "disabled".into()
        } else if chat.is_none() {
            if recording.capture_chat == Some(true) {
                "missing".into()
            } else {
                "unknown".into()
            }
        } else {
            recording
                .chat_status
                .clone()
                .unwrap_or_else(|| "unknown".into())
        };
        let (channel_name, channel_profile_image) = super::replay_assets::channel_profile::read(
            Path::new(&recording.output_dir),
            &recording.channel_id,
        );
        Ok(Session {
            token,
            recording_root: PathBuf::from(&recording.output_dir),
            recording_id: recording.id,
            title: recording.title,
            recorded_at: recording.started_at,
            channel_name,
            channel_profile_image,
            mime_type: recording
                .mime_type
                .split(';')
                .next()
                .unwrap_or("video/mp4")
                .to_owned(),
            duration,
            chat_status,
            media: Mutex::new(media),
            media_stamp,
            media_parts,
            recording_active: recording.status
                == super::browser_store::BrowserRecordingStatus::Recording,
            _temporary_files: temporary_files,
            chat,
            chat_stamp,
            viewers,
            viewer_stamp,
            timeline,
            timeline_stamp,
            fingerprint,
            index_path,
            cancel: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            offset: Mutex::new(offset),
            index: Mutex::new(IndexStatus::default()),
            opened: Instant::now(),
        })
    }
    fn session(&self, token: &str) -> Result<Arc<Session>, StreamError> {
        if !valid_token(token) {
            return Err(invalid());
        }
        let session = self
            .inner
            .sessions
            .lock()
            .map_err(|_| storage())?
            .get(token)
            .cloned()
            .ok_or_else(stale)?;
        session.valid()?;
        Ok(session)
    }
    pub fn close(&self, token: &str) -> Result<(), StreamError> {
        if !valid_token(token) {
            return Err(invalid());
        }
        if let Some(session) = self
            .inner
            .sessions
            .lock()
            .map_err(|_| storage())?
            .remove(token)
        {
            session.cancel.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub fn shutdown_and_wait(&self) {
        self.inner.closing.store(true, Ordering::Release);
        if let Ok(mut sessions) = self.inner.sessions.lock() {
            for session in sessions.values() {
                session.cancel.store(true, Ordering::Release);
            }
            sessions.clear();
        }
        if let Ok(mut workers) = self.inner.workers.lock() {
            for worker in workers.iter() {
                if let Some(session) = worker.session.upgrade() {
                    session.cancel.store(true, Ordering::Release);
                }
            }
            for worker in workers.drain(..) {
                let _ = worker.handle.join();
            }
        }
    }
    pub fn chat_at(
        &self,
        token: &str,
        media_time: f64,
        generation: u64,
        limit: Option<usize>,
    ) -> Result<ReplayChatPage, StreamError> {
        let session = self.session(token)?;
        if !media_time.is_finite() || media_time < 0.0 || media_time > session.duration + 1.0 {
            return Err(invalid());
        }
        session.accept_generation(generation)?;
        let result = index::page(
            &session,
            Some(media_time),
            None,
            generation,
            bounded_limit(limit),
            None,
        )?;
        session.check_generation(generation)?;
        Ok(result)
    }
    pub fn chat_page(
        &self,
        token: &str,
        cursor: Option<&str>,
        generation: u64,
        limit: Option<usize>,
    ) -> Result<ReplayChatPage, StreamError> {
        let session = self.session(token)?;
        session.accept_generation(generation)?;
        let result = index::page(
            &session,
            None,
            cursor,
            generation,
            bounded_limit(limit),
            None,
        )?;
        session.check_generation(generation)?;
        Ok(result)
    }
    pub fn chat_search(
        &self,
        token: &str,
        query: &str,
        field: &str,
        cursor: Option<&str>,
        generation: u64,
        limit: Option<usize>,
    ) -> Result<ReplayChatPage, StreamError> {
        if query.len() > 4096
            || query.chars().count() > 256
            || !matches!(field, "all" | "body" | "nickname")
            || cursor.is_some_and(|value| value.len() > 256)
        {
            return Err(invalid());
        }
        let session = self.session(token)?;
        session.accept_generation(generation)?;
        let query = index::normalize_search(query);
        let result = index::page(
            &session,
            None,
            cursor,
            generation,
            bounded_limit(limit),
            Some((&query, field)),
        )?;
        session.check_generation(generation)?;
        Ok(result)
    }
    pub fn timeline(
        &self,
        token: &str,
        bucket_seconds: Option<f64>,
    ) -> Result<ReplayTimeline, StreamError> {
        let session = self.session(token)?;
        let requested = bucket_seconds.unwrap_or(60.0);
        if !requested.is_finite() || !(1.0..=86400.0).contains(&requested) {
            return Err(invalid());
        }
        let result = index::buckets(
            &session,
            requested.max((session.duration / 2000.0).ceil()).max(1.0),
        )?;
        session.valid()?;
        Ok(result)
    }
    pub fn profile_url(&self, token: &str, sequence: u64) -> Result<String, StreamError> {
        let session = self.session(token)?;
        let url = index::profile_url(&session, sequence)?;
        session.valid()?;
        Ok(url)
    }
    pub fn set_offset(&self, token: &str, offset: f64) -> Result<f64, StreamError> {
        if !offset.is_finite() || offset.abs() > 3600.0 {
            return Err(invalid());
        }
        let session = self.session(token)?;
        let root = self.cache_root()?;
        let _guard = self.inner.settings.lock().map_err(|_| storage())?;
        let connection = settings_connection(&root)?;
        connection.execute("INSERT INTO offsets(recording_id, seconds) VALUES (?1,?2) ON CONFLICT(recording_id) DO UPDATE SET seconds=excluded.seconds", rusqlite::params![session.recording_id, offset]).map_err(|_| storage())?;
        *session.offset.lock().map_err(|_| storage())? = offset;
        Ok(offset)
    }
    fn read_offset(&self, root: &Path, id: &str) -> Result<f64, StreamError> {
        let _guard = self.inner.settings.lock().map_err(|_| storage())?;
        let connection = settings_connection(root)?;
        use rusqlite::OptionalExtension;
        let offset = connection
            .query_row(
                "SELECT seconds FROM offsets WHERE recording_id=?1",
                [id],
                |row| row.get::<_, f64>(0),
            )
            .optional()
            .map_err(|_| storage())?
            .unwrap_or(0.0);
        Ok(if offset.is_finite() && offset.abs() <= 3600.0 {
            offset
        } else {
            0.0
        })
    }
    fn cache_root(&self) -> Result<PathBuf, StreamError> {
        let base = fs::canonicalize(&self.inner.data_dir).map_err(|_| storage())?;
        let mut root = base;
        for child in ["streaming", "replay"] {
            root = root.join(child);
            match fs::create_dir(&root) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(storage()),
            }
            let meta = fs::symlink_metadata(&root).map_err(|_| storage())?;
            if !meta.is_dir()
                || meta.file_type().is_symlink()
                || fs::canonicalize(&root).map_err(|_| storage())? != root
            {
                return Err(storage());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err(storage());
                }
            }
        }
        Ok(root)
    }
}

fn settings_connection(root: &Path) -> Result<rusqlite::Connection, StreamError> {
    let path = root.join("settings-v1.sqlite");
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if !regular_metadata(&meta) || meta.len() > 8 * 1024 * 1024 {
            return Err(storage());
        }
    }
    let connection = rusqlite::Connection::open(path).map_err(|_| storage())?;
    connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA journal_mode=DELETE; PRAGMA busy_timeout=1000; PRAGMA max_page_count=2048; CREATE TABLE IF NOT EXISTS offsets(recording_id TEXT PRIMARY KEY, seconds REAL NOT NULL);").map_err(|_| storage())?;
    Ok(connection)
}
fn valid_token(token: &str) -> bool {
    token.len() == 32
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn bounded_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(MAX_PAGE_ROWS).clamp(1, MAX_PAGE_ROWS)
}

fn require_main(window: &Webview) -> Result<(), StreamError> {
    if window.label() != "main" || window.window().label() != "main" {
        return Err(invalid());
    }
    let url = window.url().map_err(|_| invalid())?;
    let trusted = matches!(
        (url.scheme(), url.host_str(), url.port()),
        ("tauri", Some("localhost"), None) | ("http" | "https", Some("tauri.localhost"), None)
    ) || cfg!(debug_assertions)
        && url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port() == Some(1420);
    if !trusted {
        return Err(invalid());
    }
    Ok(())
}
fn service(app: &AppHandle, window: &Webview) -> Result<ReplayService, StreamError> {
    require_main(window)?;
    app.try_state::<ReplayService>()
        .map(|state| state.inner().clone())
        .ok_or_else(stale)
}
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, StreamError> + Send + 'static,
) -> ApiResult<T> {
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(result) => result.into(),
        Err(_) => Err::<T, _>(storage()).into(),
    }
}

#[tauri::command]
pub async fn replay_open(
    app: AppHandle,
    window: Webview,
    recording_id: String,
) -> ApiResult<ReplaySession> {
    let service = match service(&app, &window) {
        Ok(service) => service,
        Err(error) => return Err::<ReplaySession, _>(error).into(),
    };
    blocking(move || service.open(&recording_id)).await
}
#[tauri::command]
pub async fn replay_close(app: AppHandle, window: Webview, token: String) -> ApiResult<()> {
    service(&app, &window)
        .and_then(|service| service.close(&token))
        .into()
}
#[tauri::command]
pub async fn replay_open_profile(
    app: AppHandle,
    window: Webview,
    token: String,
    sequence: u64,
) -> ApiResult<()> {
    let service = match service(&app, &window) {
        Ok(service) => service,
        Err(error) => return Err::<(), _>(error).into(),
    };
    blocking(move || {
        // A saved row identity, not a caller-controlled URL. Indexing/rendering
        // never visits it; this command is an explicit user action only.
        let url = service.profile_url(&token, sequence)?;
        #[cfg(windows)]
        {
            use windows::{
                core::PCWSTR,
                Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
            };
            let wide = url.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let result = unsafe {
                ShellExecuteW(
                    None,
                    windows::core::w!("open"),
                    PCWSTR(wide.as_ptr()),
                    None,
                    None,
                    SW_SHOWNORMAL,
                )
            };
            if result.0 as isize <= 32 {
                return Err(failure(
                    "REPLAY_PROFILE_OPEN",
                    "공개 프로필을 여는 브라우저를 찾지 못했습니다.",
                ));
            }
        }
        #[cfg(not(windows))]
        {
            let _ = url;
            return Err(failure(
                "REPLAY_PROFILE_OPEN",
                "이 환경에서는 공개 프로필 열기를 지원하지 않습니다.",
            ));
        }
        #[cfg(windows)]
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn replay_chat_at(
    app: AppHandle,
    window: Webview,
    token: String,
    media_time: f64,
    generation: u64,
    limit: Option<usize>,
) -> ApiResult<ReplayChatPage> {
    let service = match service(&app, &window) {
        Ok(service) => service,
        Err(error) => return Err::<ReplayChatPage, _>(error).into(),
    };
    blocking(move || service.chat_at(&token, media_time, generation, limit)).await
}
#[tauri::command]
pub async fn replay_chat_page(
    app: AppHandle,
    window: Webview,
    token: String,
    cursor: Option<String>,
    generation: u64,
    limit: Option<usize>,
) -> ApiResult<ReplayChatPage> {
    let service = match service(&app, &window) {
        Ok(service) => service,
        Err(error) => return Err::<ReplayChatPage, _>(error).into(),
    };
    blocking(move || service.chat_page(&token, cursor.as_deref(), generation, limit)).await
}
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Stable frontend IPC argument contract"
)]
pub async fn replay_chat_search(
    app: AppHandle,
    window: Webview,
    token: String,
    query: String,
    field: String,
    cursor: Option<String>,
    generation: u64,
    limit: Option<usize>,
) -> ApiResult<ReplayChatPage> {
    let service = match service(&app, &window) {
        Ok(service) => service,
        Err(error) => return Err::<ReplayChatPage, _>(error).into(),
    };
    blocking(move || {
        service.chat_search(&token, &query, &field, cursor.as_deref(), generation, limit)
    })
    .await
}
#[tauri::command]
pub async fn replay_timeline(
    app: AppHandle,
    window: Webview,
    token: String,
    bucket_seconds: Option<f64>,
) -> ApiResult<ReplayTimeline> {
    let service = match service(&app, &window) {
        Ok(service) => service,
        Err(error) => return Err::<ReplayTimeline, _>(error).into(),
    };
    blocking(move || service.timeline(&token, bucket_seconds)).await
}
#[tauri::command]
pub async fn replay_set_offset(
    app: AppHandle,
    window: Webview,
    token: String,
    offset_seconds: f64,
) -> ApiResult<f64> {
    let service = match service(&app, &window) {
        Ok(service) => service,
        Err(error) => return Err::<f64, _>(error).into(),
    };
    blocking(move || service.set_offset(&token, offset_seconds)).await
}

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;
