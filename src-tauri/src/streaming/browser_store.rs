//! Durable storage for independently finalized browser MediaRecorder segments.
//! Public methods do bounded blocking work and must run on `spawn_blocking`.
//! The journal is authoritative; only the most recent segments are kept in RAM.

use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File, Metadata, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use serde::{Deserialize, Serialize};

use super::model::{now_ms, StreamError};

#[path = "browser_cleanup.rs"]
pub(crate) mod cleanup;
pub use cleanup::{BrowserSourceCleanup, BrowserSourceCleanupStatus};
#[path = "browser_delete.rs"]
pub(crate) mod deletion;

const MAX_CHUNK: usize = 1024 * 1024;
const MAX_SEGMENT: u64 = 64 * 1024 * 1024;
const MAX_CHUNKS: usize = 4096;
const MAX_RECENT: usize = 128;
const MAX_RECORDINGS: usize = 256;
const MAX_ACTIVE: usize = 4;
const MAX_CATALOG: u64 = 2 * 1024 * 1024;
const MAX_METADATA: u64 = 256 * 1024;
const MAX_JOURNAL: u64 = 64 * 1024 * 1024;
const MAX_LINE: usize = 8192;
const MAX_SEGMENTS: u64 = 262_144;
const MAX_DURATION: f64 = 120.0;
const FREE_SPACE_RESERVE: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserRecordingStatus {
    Recording,
    Stopped,
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSegment {
    pub index: u64,
    pub file: String,
    pub bytes: u64,
    pub duration_seconds: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_start_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_end_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPartial {
    pub index: u64,
    /// May also name an unjournaled final file left by an interrupted commit.
    pub file: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRecording {
    pub id: String,
    pub channel_id: String,
    pub title: String,
    pub started_at: u64,
    pub updated_at: u64,
    pub status: BrowserRecordingStatus,
    pub mime_type: String,
    pub output_dir: String,
    pub segment_count: u64,
    /// Durably committed bytes only; unfinished bytes are reported in `partial`.
    pub bytes_written: u64,
    pub duration_seconds: f64,
    pub last_error: Option<String>,
    /// The latest 128 segments. The full index is `segments.jsonl` on disk.
    pub segments: Vec<BrowserSegment>,
    #[serde(default)]
    pub partial: Option<BrowserPartial>,
    /// None means an older/unfinalized recording whose chat completeness is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_chat: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_count: Option<u64>,
    /// Verified playback derivative. Source cleanup never changes chat or history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<BrowserMerge>,
    /// A durable user-confirmed deletion, possibly interrupted and retryable.
    #[serde(default)]
    pub deletion_pending: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMergeStatus {
    Queued,
    Merging,
    Complete,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserMerge {
    pub status: BrowserMergeStatus,
    pub segment_count: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_cleanup: Option<BrowserSourceCleanup>,
}
impl BrowserMerge {
    fn pending(count: u64) -> Self {
        Self {
            status: BrowserMergeStatus::Queued,
            segment_count: count,
            updated_at: now_ms(),
            file: None,
            timeline_file: None,
            bytes: None,
            duration_seconds: None,
            last_error: None,
            source_cleanup: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct BrowserMergeJob {
    pub recording: BrowserRecording,
    pub token: String,
    journal_len: u64,
    journal_modified: std::time::SystemTime,
}

pub(crate) struct BrowserMergedOutput {
    pub file: String,
    pub timeline_file: String,
    pub bytes: u64,
    pub duration_seconds: f64,
    /// Only the merge worker creates this after a successful full decode.
    pub cleanup: Option<cleanup::CleanupProofReference>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAppendAck {
    pub recording_id: String,
    pub segment_index: u64,
    pub chunk_index: u64,
    pub bytes_accepted: u64,
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogEntry {
    id: String,
    channel_id: String,
    title: String,
    started_at: u64,
    mime_type: String,
    output_dir: String,
    #[serde(default)]
    deletion_pending: bool,
}

impl CatalogEntry {
    fn recording(&self) -> BrowserRecording {
        BrowserRecording {
            id: self.id.clone(),
            channel_id: self.channel_id.clone(),
            title: self.title.clone(),
            started_at: self.started_at,
            updated_at: self.started_at,
            status: BrowserRecordingStatus::Recording,
            mime_type: self.mime_type.clone(),
            output_dir: self.output_dir.clone(),
            segment_count: 0,
            bytes_written: 0,
            duration_seconds: 0.0,
            last_error: None,
            segments: Vec::new(),
            partial: None,
            capture_chat: None,
            chat_status: None,
            chat_count: None,
            merge: None,
            deletion_pending: self.deletion_pending,
        }
    }
}

#[derive(Clone)]
struct ChunkReceipt {
    offset: u64,
    length: usize,
}

struct OpenSegment {
    file: File,
    bytes: u64,
    chunks: Vec<ChunkReceipt>,
}

struct Replay {
    index: u64,
    file: String,
    chunks: Vec<ChunkReceipt>,
}

struct Active {
    journal: File,
    current: Option<OpenSegment>,
    previous: Option<Replay>,
}

struct State {
    catalog: Vec<CatalogEntry>,
    recordings: VecDeque<BrowserRecording>,
    active: HashMap<String, Active>,
    closing: bool,
    merge_job: Option<(String, String)>,
    deleting: HashMap<String, String>,
}

pub struct BrowserCaptureStore {
    catalog_root: PathBuf,
    state: Mutex<State>,
}

/// An immutable, catalog-authorized completed recording. Handles remain local
/// to the replay service; IPC never accepts or returns these filesystem paths.
pub(crate) struct BrowserReplaySource {
    pub recording: BrowserRecording,
    pub media: File,
    pub chat: Option<File>,
    pub timeline: File,
}

impl BrowserCaptureStore {
    pub fn new(data_dir: &Path) -> Result<Self, StreamError> {
        let base = checked_directory(data_dir)?;
        let streaming = child_directory(&base, "streaming")?;
        let catalog_root = child_directory(&streaming, "browser")?;
        let catalog = read_catalog(&catalog_root.join("catalog.jsonl"))?;
        let recordings = catalog.iter().map(recover_recording).collect();
        Ok(Self {
            catalog_root,
            state: Mutex::new(State {
                catalog,
                recordings,
                active: HashMap::new(),
                closing: false,
                merge_job: None,
                deleting: HashMap::new(),
            }),
        })
    }

    pub fn begin(
        &self,
        download_root: &Path,
        channel_id: &str,
        title: &str,
        mime_type: &str,
    ) -> Result<BrowserRecording, StreamError> {
        if !valid_channel(channel_id) {
            return Err(invalid());
        }
        let mime_type = normalized_mime(mime_type).ok_or_else(invalid)?;
        let title = bounded_text(title, 256);
        let mut state = self.lock()?;
        if state.closing || state.active.len() >= MAX_ACTIVE {
            return Err(error(
                "BROWSER_CAPTURE_BUSY",
                "브라우저 녹화는 동시에 최대 네 개까지 진행할 수 있습니다.",
                true,
            ));
        }
        if state.catalog.len() >= MAX_RECORDINGS {
            return Err(error(
                "BROWSER_CAPTURE_HISTORY_FULL",
                "브라우저 녹화 목록의 보관 한도에 도달했습니다.",
                false,
            ));
        }
        let base = checked_directory(download_root)?;
        ensure_space(&base, 0)?;
        let chzzk = child_directory(&base, "CHZZK")?;
        let parent = child_directory(&chzzk, "BrowserCapture")?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let root = parent.join(&id);
        fs::create_dir(&root).map_err(|_| storage())?;
        let root = checked_directory(&root)?;
        let entry = CatalogEntry {
            id: id.clone(),
            channel_id: channel_id.to_ascii_lowercase(),
            title,
            started_at: now_ms(),
            mime_type,
            output_dir: root.to_string_lossy().into_owned(),
            deletion_pending: false,
        };
        let recording = entry.recording();
        let journal = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(root.join("segments.jsonl"))
            .map_err(|_| storage())?;
        journal.sync_all().map_err(|_| storage())?;
        save_metadata(&recording)?;
        let mut next_catalog = state.catalog.clone();
        next_catalog.push(entry);
        save_catalog(&self.catalog_root, &next_catalog)?;
        state.catalog = next_catalog;
        state.recordings.push_back(recording.clone());
        state.active.insert(
            id.clone(),
            Active {
                journal,
                current: None,
                previous: None,
            },
        );
        Ok(recording)
    }

    pub fn append(
        &self,
        recording_id: &str,
        segment_index: u64,
        chunk_index: u64,
        bytes: &[u8],
    ) -> Result<BrowserAppendAck, StreamError> {
        validate_id(recording_id)?;
        if bytes.is_empty() || bytes.len() > MAX_CHUNK || chunk_index >= MAX_CHUNKS as u64 {
            return Err(invalid());
        }
        let mut state = self.lock()?;
        let index = active_index(&state, recording_id)?;
        let mut active = state.active.remove(recording_id).ok_or_else(inactive)?;
        let recording = &mut state.recordings[index];
        let result = append_chunk(&mut active, recording, segment_index, chunk_index, bytes);
        if result.as_ref().is_err_and(is_storage_failure) {
            fail_active(&mut active, recording);
        } else {
            state.active.insert(recording_id.into(), active);
        }
        result
    }

    pub fn finish_segment(
        &self,
        recording_id: &str,
        segment_index: u64,
        duration_seconds: f64,
    ) -> Result<BrowserRecording, StreamError> {
        self.finish_segment_source(recording_id, segment_index, duration_seconds, None, None)
    }
    pub fn finish_segment_source(
        &self,
        recording_id: &str,
        segment_index: u64,
        duration_seconds: f64,
        source_start_seconds: Option<f64>,
        source_end_seconds: Option<f64>,
    ) -> Result<BrowserRecording, StreamError> {
        validate_id(recording_id)?;
        if !valid_duration(duration_seconds)
            || !valid_source_range(source_start_seconds, source_end_seconds, duration_seconds)
        {
            return Err(invalid());
        }
        let mut state = self.lock()?;
        let index = state
            .recordings
            .iter()
            .position(|entry| entry.id == recording_id)
            .ok_or_else(inactive)?;
        if state.recordings[index].deletion_pending {
            return Err(inactive());
        }
        let recording = &state.recordings[index];
        if segment_index < recording.segment_count {
            let previous = recording
                .segments
                .iter()
                .find(|segment| segment.index == segment_index)
                .ok_or_else(invalid)?;
            if (previous.duration_seconds - duration_seconds).abs() > 0.001
                || previous.source_start_seconds != source_start_seconds
                || previous.source_end_seconds != source_end_seconds
            {
                return Err(invalid());
            }
            return Ok(recording.clone());
        }
        active_index(&state, recording_id)?;
        if segment_index != recording.segment_count || segment_index >= MAX_SEGMENTS {
            return Err(invalid());
        }
        let mut active = state.active.remove(recording_id).ok_or_else(inactive)?;
        let recording = &mut state.recordings[index];
        let result = commit_segment(
            &mut active,
            recording,
            duration_seconds,
            source_start_seconds,
            source_end_seconds,
        );
        let response = match result {
            Ok(()) => Ok(recording.clone()),
            Err(cause) => {
                if is_storage_failure(&cause) {
                    fail_active(&mut active, recording);
                }
                Err(cause)
            }
        };
        if recording.status == BrowserRecordingStatus::Recording {
            state.active.insert(recording_id.into(), active);
        }
        response
    }

    pub fn finish(
        &self,
        recording_id: &str,
        interrupted: bool,
        reason: Option<&str>,
    ) -> Result<BrowserRecording, StreamError> {
        self.finish_details(recording_id, interrupted, reason, None)
    }

    pub fn finish_with_chat(
        &self,
        recording_id: &str,
        interrupted: bool,
        reason: Option<&str>,
        capture_chat: bool,
        chat_status: &str,
        chat_count: u64,
    ) -> Result<BrowserRecording, StreamError> {
        if !valid_chat_summary(Some(capture_chat), Some(chat_status), Some(chat_count)) {
            return Err(invalid());
        }
        self.finish_details(
            recording_id,
            interrupted,
            reason,
            Some((capture_chat, chat_status, chat_count)),
        )
    }

    fn finish_details(
        &self,
        recording_id: &str,
        interrupted: bool,
        reason: Option<&str>,
        chat: Option<(bool, &str, u64)>,
    ) -> Result<BrowserRecording, StreamError> {
        validate_id(recording_id)?;
        let mut state = self.lock()?;
        let index = state
            .recordings
            .iter()
            .position(|entry| entry.id == recording_id)
            .ok_or_else(inactive)?;
        if state.recordings[index].deletion_pending {
            return Err(inactive());
        }
        if let Some((enabled, status, count)) = chat {
            let recording = &mut state.recordings[index];
            recording.capture_chat = Some(enabled);
            recording.chat_status = Some(status.into());
            recording.chat_count = Some(count);
        }
        if !state.active.contains_key(recording_id) {
            // A failed write already closed this recorder. Still persist its
            // chat warning, without ever upgrading Failed to a successful stop.
            if chat.is_some() && save_metadata(&state.recordings[index]).is_err() {
                state.recordings[index].status = BrowserRecordingStatus::Failed;
                state.recordings[index].last_error = Some(
                    "채팅 저장 상태를 기록하지 못했습니다. 기존 영상 파일은 보존됩니다.".into(),
                );
                return Err(storage());
            }
            return Ok(state.recordings[index].clone());
        }
        let mut active = state.active.remove(recording_id).ok_or_else(inactive)?;
        let recording = &mut state.recordings[index];
        if close_partial(&mut active).is_err() {
            fail_active(&mut active, recording);
            return Err(storage());
        }
        recording.status = if interrupted || recording.partial.is_some() {
            BrowserRecordingStatus::Interrupted
        } else {
            BrowserRecordingStatus::Stopped
        };
        recording.updated_at = now_ms();
        recording.last_error =
            (recording.status == BrowserRecordingStatus::Interrupted).then(|| {
                reason
                    .map(|value| bounded_text(value, 512))
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| {
                        "녹화가 중단되었습니다. 확정된 세그먼트와 미완성 파일을 보존했습니다."
                            .into()
                    })
            });
        if save_metadata(recording).is_err() {
            fail_active(&mut active, recording);
            return Err(storage());
        }
        Ok(recording.clone())
    }

    pub fn snapshot(&self) -> Result<Vec<BrowserRecording>, StreamError> {
        Ok(self.lock()?.recordings.iter().rev().cloned().collect())
    }

    pub fn has_active(&self) -> bool {
        self.state
            .lock()
            .map_or(true, |state| !state.active.is_empty())
    }

    /// Called only by the one merge worker. This reserves metadata, not media I/O.
    pub(crate) fn take_merge_job(&self) -> Result<Option<BrowserMergeJob>, StreamError> {
        let mut state = self.lock()?;
        if state.closing || state.merge_job.is_some() {
            return Ok(None);
        }
        // A removed/offline old recording must not starve later valid entries.
        for _ in 0..MAX_RECORDINGS {
            let Some(index) = state.recordings.iter().position(|r| {
                r.status != BrowserRecordingStatus::Recording
                    && !r.deletion_pending
                    && r.segment_count > 0
                    && !state.active.contains_key(&r.id)
                    && r.merge
                        .as_ref()
                        .is_none_or(|m| m.status == BrowserMergeStatus::Queued)
            }) else {
                return Ok(None);
            };
            let recording = &mut state.recordings[index];
            let journal = (|| -> Result<(u64, std::time::SystemTime), StreamError> {
                let root = owned_root(&recording.output_dir, &recording.id)?;
                let metadata = open_regular(&root.join("segments.jsonl"), MAX_JOURNAL)?
                    .metadata()
                    .map_err(|_| storage())?;
                Ok((metadata.len(), metadata.modified().map_err(|_| storage())?))
            })();
            let (journal_len, journal_modified) = match journal {
                Ok(value) => value,
                Err(_) => {
                    let mut merge = BrowserMerge::pending(recording.segment_count);
                    merge.status = BrowserMergeStatus::Blocked;
                    merge.last_error = Some("원본 녹화 목록에 접근할 수 없습니다. 드라이브와 폴더를 확인한 뒤 다시 시도해 주세요.".into());
                    recording.merge = Some(merge);
                    let _ = save_metadata(recording);
                    continue;
                }
            };
            let token = uuid::Uuid::new_v4().simple().to_string();
            let mut merge = BrowserMerge::pending(recording.segment_count);
            merge.status = BrowserMergeStatus::Merging;
            recording.merge = Some(merge);
            if save_metadata(recording).is_err() {
                recording.merge.as_mut().unwrap().status = BrowserMergeStatus::Failed;
                recording.merge.as_mut().unwrap().last_error =
                    Some("병합 예약 상태를 저장하지 못했습니다.".into());
                continue;
            }
            let job = BrowserMergeJob {
                recording: recording.clone(),
                token: token.clone(),
                journal_len,
                journal_modified,
            };
            let id = recording.id.clone();
            state.merge_job = Some((id, token));
            return Ok(Some(job));
        }
        Ok(None)
    }

    /// Retry failed merges or unfinished cleanup, never rebuild a successful merge.
    pub fn retry_merges(&self, id: Option<&str>) -> Result<usize, StreamError> {
        if let Some(id) = id {
            validate_id(id)?;
        }
        let mut state = self.lock()?;
        if state.closing {
            return Err(inactive());
        }
        let mut count = 0;
        let running = state.merge_job.as_ref().map(|(id, _)| id.clone());
        for recording in &mut state.recordings {
            if id.is_some_and(|id| id != recording.id)
                || recording.deletion_pending
                || running.as_deref() == Some(recording.id.as_str())
                || recording.status == BrowserRecordingStatus::Recording
                || recording.segment_count == 0
                || recording
                    .merge
                    .as_ref()
                    .is_some_and(|m| m.status == BrowserMergeStatus::Merging)
            {
                continue;
            }
            if let Some(merge) = recording
                .merge
                .as_mut()
                .filter(|m| m.status == BrowserMergeStatus::Complete)
            {
                if let Some(cleanup) = merge
                    .source_cleanup
                    .as_mut()
                    .filter(|c| c.status == BrowserSourceCleanupStatus::Blocked)
                {
                    cleanup.status = BrowserSourceCleanupStatus::Pending;
                    cleanup.last_error = None;
                    if save_metadata(recording).is_ok() {
                        count += 1;
                    } else {
                        recording
                            .merge
                            .as_mut()
                            .unwrap()
                            .source_cleanup
                            .as_mut()
                            .unwrap()
                            .status = BrowserSourceCleanupStatus::Blocked;
                    }
                }
                continue;
            }
            recording.merge = Some(BrowserMerge::pending(recording.segment_count));
            if save_metadata(recording).is_ok() {
                count += 1;
            } else {
                let merge = recording.merge.as_mut().unwrap();
                merge.status = BrowserMergeStatus::Blocked;
                merge.last_error = Some(
                    "병합 예약 상태를 저장하지 못했습니다. 드라이브와 폴더를 확인해 주세요.".into(),
                );
            }
        }
        Ok(count)
    }

    /// Trusted-main lookup; neither the filename nor the output root is taken
    /// from an IPC caller. Validate a derivative without hashing a whole video.
    pub fn merged_file(&self, id: &str) -> Result<PathBuf, StreamError> {
        validate_id(id)?;
        let state = self.lock()?;
        let recording = state
            .recordings
            .iter()
            .find(|r| r.id == id && !r.deletion_pending)
            .ok_or_else(invalid)?;
        let merge = recording
            .merge
            .as_ref()
            .filter(|m| m.status == BrowserMergeStatus::Complete)
            .ok_or_else(invalid)?;
        if !valid_merge(Some(merge), recording.segment_count, &recording.mime_type) {
            return Err(invalid());
        }
        let root = owned_root(&recording.output_dir, id)?;
        let file = root.join(merge.file.as_ref().ok_or_else(invalid)?);
        if open_regular(&file, u64::MAX)?
            .metadata()
            .map_err(|_| storage())?
            .len()
            != merge.bytes.ok_or_else(invalid)?
            || fs::canonicalize(&file).map_err(|_| storage())? != file
        {
            return Err(storage());
        }
        Ok(file)
    }

    pub(crate) fn replay_source(&self, id: &str) -> Result<BrowserReplaySource, StreamError> {
        validate_id(id)?;
        let state = self.lock()?;
        let recording = state
            .recordings
            .iter()
            .find(|r| r.id == id && !r.deletion_pending)
            .ok_or_else(invalid)?;
        let merge = recording
            .merge
            .as_ref()
            .filter(|m| m.status == BrowserMergeStatus::Complete)
            .ok_or_else(invalid)?;
        if recording.status == BrowserRecordingStatus::Recording
            || state.active.contains_key(id)
            || !valid_merge(Some(merge), recording.segment_count, &recording.mime_type)
        {
            return Err(invalid());
        }
        let root = owned_root(&recording.output_dir, id)?;
        let media_path = root.join(merge.file.as_ref().ok_or_else(invalid)?);
        let timeline_path = root.join(merge.timeline_file.as_ref().ok_or_else(invalid)?);
        for path in [&media_path, &timeline_path] {
            if fs::canonicalize(path).map_err(|_| storage())? != *path {
                return Err(storage());
            }
        }
        let media = open_regular(&media_path, u64::MAX)?;
        if media.metadata().map_err(|_| storage())?.len() != merge.bytes.ok_or_else(invalid)? {
            return Err(storage());
        }
        let timeline = open_regular(&timeline_path, 128 * 1024 * 1024)?;
        let chat_path = root.join("chat.jsonl");
        let chat = match fs::symlink_metadata(&chat_path) {
            Ok(_) => {
                if fs::canonicalize(&chat_path).map_err(|_| storage())? != chat_path {
                    return Err(storage());
                }
                Some(open_regular(&chat_path, u64::MAX)?)
            }
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(storage()),
        };
        Ok(BrowserReplaySource {
            recording: recording.clone(),
            media,
            chat,
            timeline,
        })
    }

    pub(crate) fn complete_merge(
        &self,
        job: &BrowserMergeJob,
        output: BrowserMergedOutput,
    ) -> Result<(), StreamError> {
        let mut state = self.lock()?;
        let index = merge_job_index(&state, job)?;
        let root = validate_merge_generation(job)?;
        if output.file
            != format!(
                "merged-{}.{}",
                job.token,
                extension(&job.recording.mime_type)
            )
            || output.timeline_file != format!("merged-{}.timeline.jsonl", job.token)
            || output.bytes == 0
            || !output.duration_seconds.is_finite()
            || output.duration_seconds <= 0.0
        {
            return Err(invalid());
        }
        if open_regular(&root.join(&output.file), u64::MAX)?
            .metadata()
            .map_err(|_| storage())?
            .len()
            != output.bytes
        {
            return Err(storage());
        }
        open_regular(&root.join(&output.timeline_file), 128 * 1024 * 1024)?;
        let source_cleanup = output
            .cleanup
            .map(|proof| cleanup::pending_proof(&root, &job.token, proof))
            .transpose()?;
        let recording = &mut state.recordings[index];
        recording.merge = Some(BrowserMerge {
            status: BrowserMergeStatus::Complete,
            segment_count: recording.segment_count,
            updated_at: now_ms(),
            file: Some(output.file),
            timeline_file: Some(output.timeline_file),
            bytes: Some(output.bytes),
            duration_seconds: Some(output.duration_seconds),
            last_error: None,
            source_cleanup,
        });
        if let Err(error) = save_metadata(recording) {
            recording.merge.as_mut().unwrap().status = BrowserMergeStatus::Failed;
            recording.merge.as_mut().unwrap().source_cleanup = None;
            recording.merge.as_mut().unwrap().last_error =
                Some("병합 결과 상태를 기록하지 못했습니다. 영상과 원본 조각은 보존됩니다.".into());
            state.merge_job = None;
            return Err(error);
        }
        state.merge_job = None;
        Ok(())
    }

    pub(crate) fn fail_merge(
        &self,
        job: &BrowserMergeJob,
        status: BrowserMergeStatus,
        message: &str,
    ) -> Result<(), StreamError> {
        if !matches!(
            status,
            BrowserMergeStatus::Blocked | BrowserMergeStatus::Failed | BrowserMergeStatus::Queued
        ) {
            return Err(invalid());
        }
        let mut state = self.lock()?;
        let index = merge_job_index(&state, job)?;
        let recording = &mut state.recordings[index];
        let mut merge = BrowserMerge::pending(recording.segment_count);
        merge.status = status;
        merge.last_error = Some(bounded_text(message, 512));
        recording.merge = Some(merge);
        let result = save_metadata(recording);
        state.merge_job = None;
        result
    }

    pub fn shutdown(&self) -> Result<(), StreamError> {
        let ids = {
            let mut state = self.lock()?;
            state.closing = true;
            state.active.keys().cloned().collect::<Vec<_>>()
        };
        let mut failure = None;
        for id in ids {
            if let Err(error) = self.finish(&id, true, Some("app_shutdown")) {
                failure = Some(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    fn lock(&self) -> Result<MutexGuard<'_, State>, StreamError> {
        self.state.lock().map_err(|_| storage())
    }
}

fn merge_job_index(state: &State, job: &BrowserMergeJob) -> Result<usize, StreamError> {
    if state.merge_job.as_ref() != Some(&(job.recording.id.clone(), job.token.clone()))
        || state.active.contains_key(&job.recording.id)
    {
        return Err(invalid());
    }
    state
        .recordings
        .iter()
        .position(|r| {
            r.id == job.recording.id
                && r.status != BrowserRecordingStatus::Recording
                && r.segment_count == job.recording.segment_count
                && r.bytes_written == job.recording.bytes_written
                && r.duration_seconds == job.recording.duration_seconds
        })
        .ok_or_else(invalid)
}

pub(crate) fn validate_merge_generation(job: &BrowserMergeJob) -> Result<PathBuf, StreamError> {
    let root = owned_root(&job.recording.output_dir, &job.recording.id)?;
    let metadata = open_regular(&root.join("segments.jsonl"), MAX_JOURNAL)?
        .metadata()
        .map_err(|_| storage())?;
    if metadata.len() != job.journal_len
        || metadata.modified().map_err(|_| storage())? != job.journal_modified
    {
        return Err(invalid());
    }
    Ok(root)
}

/// A bounded journal scan outside either shared store mutex; never scans media.
pub(crate) fn read_merge_segments(
    job: &BrowserMergeJob,
) -> Result<Vec<BrowserSegment>, StreamError> {
    let root = validate_merge_generation(job)?;
    let mut reader = BufReader::new(open_regular(&root.join("segments.jsonl"), MAX_JOURNAL)?);
    let mut segments = Vec::new();
    let mut bytes = 0u64;
    let mut duration = 0.0;
    while let Some(line) = read_line(&mut reader)? {
        let s: BrowserSegment = serde_json::from_slice(&line).map_err(|_| invalid())?;
        if s.index != segments.len() as u64
            || s.index >= MAX_SEGMENTS
            || s.file != segment_name(s.index, extension(&job.recording.mime_type))
            || s.bytes == 0
            || s.bytes > MAX_SEGMENT
            || !valid_duration(s.duration_seconds)
            || !valid_source_range(
                s.source_start_seconds,
                s.source_end_seconds,
                s.duration_seconds,
            )
        {
            return Err(invalid());
        }
        bytes = bytes.checked_add(s.bytes).ok_or_else(invalid)?;
        duration += s.duration_seconds;
        segments.push(s);
    }
    if segments.len() as u64 != job.recording.segment_count
        || bytes != job.recording.bytes_written
        || (duration - job.recording.duration_seconds).abs() > 0.001
    {
        return Err(invalid());
    }
    validate_merge_generation(job)?;
    Ok(segments)
}

fn active_index(state: &State, id: &str) -> Result<usize, StreamError> {
    if !state.active.contains_key(id) {
        return Err(inactive());
    }
    state
        .recordings
        .iter()
        .position(|entry| entry.id == id)
        .ok_or_else(inactive)
}
fn valid_chat_summary(enabled: Option<bool>, status: Option<&str>, count: Option<u64>) -> bool {
    match (enabled, status, count) {
        (None, None, None) => true,
        (Some(false), Some("disabled"), Some(0)) => true,
        (Some(true), Some(status), Some(count)) => {
            count <= 9_007_199_254_740_991
                && matches!(
                    status,
                    "connecting"
                        | "page_connected"
                        | "observing"
                        | "receiving"
                        | "waiting_socket"
                        | "connected"
                        | "disconnected"
                        | "stopped"
                        | "storage_failed"
                        | "queue_overflow"
                        | "observer_unavailable"
                        | "unsupported_frame"
                        | "frame_too_large"
                        | "invalid_frame"
                        | "message_too_large"
                        | "message_truncated"
                        | "decode_failed"
                        | "observer_overflow"
                        | "connection_gap"
                        | "partial"
                )
        }
        _ => false,
    }
}

fn valid_merge(merge: Option<&BrowserMerge>, count: u64, mime: &str) -> bool {
    let Some(m) = merge else {
        return true;
    };
    if m.segment_count != count
        || m.last_error
            .as_ref()
            .is_some_and(|s| s.chars().count() > 512)
    {
        return false;
    }
    if m.status != BrowserMergeStatus::Complete {
        return m.source_cleanup.is_none();
    }
    let Some(token) = m
        .file
        .as_deref()
        .and_then(|f| f.strip_prefix("merged-"))
        .and_then(|f| f.strip_suffix(&format!(".{}", extension(mime))))
    else {
        return false;
    };
    valid_id(token)
        && cleanup::valid_summary(m.source_cleanup.as_ref(), token, count)
        && m.timeline_file.as_deref() == Some(format!("merged-{token}.timeline.jsonl").as_str())
        && m.bytes.is_some_and(|bytes| bytes > 0)
        && m.duration_seconds.is_some_and(|d| d.is_finite() && d > 0.0)
}
fn valid_source_range(start: Option<f64>, end: Option<f64>, duration: f64) -> bool {
    match (start, end) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.is_finite()
                && b.is_finite()
                && a >= 0.0
                && b > a
                && ((b - a) - duration).abs() <= 0.05
        }
        _ => false,
    }
}

fn append_chunk(
    active: &mut Active,
    recording: &mut BrowserRecording,
    segment_index: u64,
    chunk_index: u64,
    bytes: &[u8],
) -> Result<BrowserAppendAck, StreamError> {
    let root = owned_root(&recording.output_dir, &recording.id)?;
    let duplicate = if segment_index < recording.segment_count {
        let previous = active
            .previous
            .as_ref()
            .filter(|previous| previous.index == segment_index)
            .ok_or_else(invalid)?;
        let receipt = previous
            .chunks
            .get(chunk_index as usize)
            .ok_or_else(invalid)?;
        let mut file = open_regular(&root.join(&previous.file), MAX_SEGMENT)?;
        compare_chunk(&mut file, receipt, bytes)?;
        true
    } else {
        if segment_index != recording.segment_count || segment_index >= MAX_SEGMENTS {
            return Err(invalid());
        }
        if active.current.is_none() {
            if chunk_index != 0 || !valid_header(bytes, extension(&recording.mime_type)) {
                return Err(invalid());
            }
            ensure_space(&root, bytes.len() as u64)?;
            let name = partial_name(segment_index, extension(&recording.mime_type));
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(root.join(&name))
                .map_err(|_| storage())?;
            active.current = Some(OpenSegment {
                file,
                bytes: 0,
                chunks: Vec::new(),
            });
            recording.partial = Some(BrowserPartial {
                index: segment_index,
                file: name,
                bytes: 0,
            });
        }
        let current = active.current.as_mut().ok_or_else(storage)?;
        if let Some(receipt) = current.chunks.get(chunk_index as usize) {
            compare_chunk(&mut current.file, receipt, bytes)?;
            true
        } else {
            if chunk_index != current.chunks.len() as u64
                || !chunk_fits(current.bytes, bytes.len(), current.chunks.len())
            {
                return Err(invalid());
            }
            ensure_space(&root, bytes.len() as u64)?;
            if current.file.metadata().map_err(|_| storage())?.len() != current.bytes {
                return Err(storage());
            }
            current.file.seek(SeekFrom::End(0)).map_err(|_| storage())?;
            current.file.write_all(bytes).map_err(|_| storage())?;
            current.chunks.push(ChunkReceipt {
                offset: current.bytes,
                length: bytes.len(),
            });
            current.bytes += bytes.len() as u64;
            if let Some(partial) = &mut recording.partial {
                partial.bytes = current.bytes;
            }
            recording.updated_at = now_ms();
            false
        }
    };
    Ok(BrowserAppendAck {
        recording_id: recording.id.clone(),
        segment_index,
        chunk_index,
        bytes_accepted: if duplicate { 0 } else { bytes.len() as u64 },
        duplicate,
    })
}

fn chunk_fits(total: u64, length: usize, count: usize) -> bool {
    length > 0
        && length <= MAX_CHUNK
        && count < MAX_CHUNKS
        && total
            .checked_add(length as u64)
            .is_some_and(|next| next <= MAX_SEGMENT)
}

fn compare_chunk(file: &mut File, receipt: &ChunkReceipt, bytes: &[u8]) -> Result<(), StreamError> {
    if receipt.length != bytes.len() {
        return Err(invalid());
    }
    let mut previous = vec![0; receipt.length];
    file.seek(SeekFrom::Start(receipt.offset))
        .map_err(|_| storage())?;
    file.read_exact(&mut previous).map_err(|_| storage())?;
    if previous != bytes {
        return Err(invalid());
    }
    Ok(())
}

fn commit_segment(
    active: &mut Active,
    recording: &mut BrowserRecording,
    duration: f64,
    source_start_seconds: Option<f64>,
    source_end_seconds: Option<f64>,
) -> Result<(), StreamError> {
    let root = owned_root(&recording.output_dir, &recording.id)?;
    let current = active.current.as_ref().ok_or_else(invalid)?;
    if current.bytes == 0 || current.bytes > MAX_SEGMENT {
        return Err(invalid());
    }
    if current.file.metadata().map_err(|_| storage())?.len() != current.bytes {
        return Err(storage());
    }
    current.file.sync_all().map_err(|_| storage())?;
    let current = active.current.take().ok_or_else(invalid)?;
    let bytes = current.bytes;
    let chunks = current.chunks;
    drop(current.file);
    let index = recording.segment_count;
    let name = segment_name(index, extension(&recording.mime_type));
    let partial = partial_name(index, extension(&recording.mime_type));
    rename_closed(&root.join(partial), &root.join(&name), false)?;
    // Until the journal sync succeeds this file is still explicitly unconfirmed.
    recording.partial = Some(BrowserPartial {
        index,
        file: name.clone(),
        bytes,
    });
    let segment = BrowserSegment {
        index,
        file: name.clone(),
        bytes,
        duration_seconds: duration,
        source_start_seconds,
        source_end_seconds,
    };
    let line = json_line(&segment)?;
    let journal_size = active.journal.metadata().map_err(|_| storage())?.len();
    if journal_size.saturating_add(line.len() as u64) > MAX_JOURNAL {
        return Err(storage());
    }
    active
        .journal
        .seek(SeekFrom::End(0))
        .map_err(|_| storage())?;
    active
        .journal
        .write_all(&line)
        .and_then(|()| active.journal.sync_all())
        .map_err(|_| storage())?;
    recording.segment_count += 1;
    recording.bytes_written = recording
        .bytes_written
        .checked_add(bytes)
        .ok_or_else(storage)?;
    recording.duration_seconds += duration;
    recording.updated_at = now_ms();
    recording.partial = None;
    recording.segments.push(segment);
    if recording.segments.len() > MAX_RECENT {
        recording.segments.remove(0);
    }
    active.previous = Some(Replay {
        index,
        file: name,
        chunks,
    });
    save_metadata(recording)
}

fn close_partial(active: &mut Active) -> Result<(), StreamError> {
    if let Some(current) = active.current.take() {
        current.file.sync_all().map_err(|_| storage())?;
    }
    Ok(())
}

fn fail_active(active: &mut Active, recording: &mut BrowserRecording) {
    let _ = close_partial(active);
    recording.status = BrowserRecordingStatus::Failed;
    recording.updated_at = now_ms();
    recording.last_error =
        Some("녹화 파일 저장에 실패했습니다. 확정된 세그먼트와 미완성 파일을 보존했습니다.".into());
    if let Ok(root) = owned_root(&recording.output_dir, &recording.id) {
        if let Ok(partial) = find_partial(
            &root,
            recording.segment_count,
            extension(&recording.mime_type),
        ) {
            recording.partial = partial;
        }
    }
    let _ = save_metadata(recording);
}

fn read_catalog(path: &Path) -> Result<Vec<CatalogEntry>, StreamError> {
    if !path.try_exists().map_err(|_| storage())? {
        return Ok(Vec::new());
    }
    let file = open_regular(path, MAX_CATALOG)?;
    let mut reader = BufReader::new(file);
    let mut entries = Vec::new();
    while let Some(line) = read_line(&mut reader)? {
        let entry: CatalogEntry = serde_json::from_slice(&line).map_err(|_| storage())?;
        if entries.len() >= MAX_RECORDINGS
            || !valid_id(&entry.id)
            || !valid_channel(&entry.channel_id)
            || entry.title.chars().count() > 256
            || normalized_mime(&entry.mime_type).as_deref() != Some(entry.mime_type.as_str())
            || !valid_root_shape(Path::new(&entry.output_dir), &entry.id)
            || entries
                .iter()
                .any(|previous: &CatalogEntry| previous.id == entry.id)
        {
            return Err(storage());
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn recover_recording(entry: &CatalogEntry) -> BrowserRecording {
    let mut recording = entry.recording();
    if entry.deletion_pending {
        // Never rebuild or automatically delete a half-removed recording.
        recording.status = BrowserRecordingStatus::Failed;
        recording.last_error =
            Some("삭제가 완료되지 않았습니다. 선택 삭제로 다시 시도해 주세요.".into());
        return recording;
    }
    let result = recover_into(entry, &mut recording);
    if result.is_err() {
        recording.status = BrowserRecordingStatus::Failed;
        recording.last_error =
            Some("녹화 기록의 일부를 확인하지 못했습니다. 기존 파일은 변경하지 않았습니다.".into());
    } else if recording.status != BrowserRecordingStatus::Failed
        && (recording.status == BrowserRecordingStatus::Recording || recording.partial.is_some())
    {
        recording.status = BrowserRecordingStatus::Interrupted;
        recording.last_error = Some("이전 녹화가 정상적으로 마무리되지 않았습니다. 확정된 세그먼트와 미완성 파일을 보존했습니다.".into());
    }
    recording
}

fn recover_into(entry: &CatalogEntry, recording: &mut BrowserRecording) -> Result<(), StreamError> {
    let root = owned_root(&entry.output_dir, &entry.id)?;
    // Damaged summary metadata must not hide independently journaled segments.
    let metadata = open_regular(&root.join("recording.json"), MAX_METADATA)
        .ok()
        .and_then(|file| serde_json::from_reader::<_, BrowserRecording>(file).ok())
        .filter(|metadata| {
            metadata.id == entry.id
                && metadata.channel_id == entry.channel_id
                && metadata.output_dir == entry.output_dir
                && metadata.mime_type == entry.mime_type
                && metadata.started_at == entry.started_at
                && metadata.title == entry.title
                && metadata.segments.len() <= MAX_RECENT
                && metadata.segment_count <= MAX_SEGMENTS
                && metadata.duration_seconds.is_finite()
                && metadata.duration_seconds >= 0.0
                && valid_chat_summary(
                    metadata.capture_chat,
                    metadata.chat_status.as_deref(),
                    metadata.chat_count,
                )
                && metadata
                    .last_error
                    .as_ref()
                    .is_none_or(|value| value.chars().count() <= 512)
        });
    if let Some(metadata) = &metadata {
        recording.status = metadata.status;
        recording.updated_at = metadata.updated_at;
        recording.last_error = metadata.last_error.clone();
        recording.capture_chat = metadata.capture_chat;
        recording.chat_status = metadata.chat_status.clone();
        recording.chat_count = metadata.chat_count;
        recording.merge = metadata.merge.clone();
        if !valid_merge(
            recording.merge.as_ref(),
            metadata.segment_count,
            &metadata.mime_type,
        ) {
            let mut merge = BrowserMerge::pending(metadata.segment_count);
            merge.status = BrowserMergeStatus::Failed;
            merge.last_error =
                Some("이전 병합 상태를 확인하지 못했습니다. 원본 녹화는 보존됩니다.".into());
            recording.merge = Some(merge);
        }
        if let Some(merge) = &mut recording.merge {
            if merge.status == BrowserMergeStatus::Merging {
                merge.status = BrowserMergeStatus::Queued;
                merge.last_error =
                    Some("이전 병합이 중단되어 다시 시도합니다. 원본 조각은 보존됩니다.".into());
            }
        }
    }
    let file = open_regular(&root.join("segments.jsonl"), MAX_JOURNAL)?;
    let mut reader = BufReader::new(file);
    let mut valid = metadata.is_some();
    loop {
        let line = match read_line(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(_) => {
                valid = false;
                break;
            }
        };
        let segment = match serde_json::from_slice::<BrowserSegment>(&line) {
            Ok(segment) => segment,
            Err(_) => {
                valid = false;
                break;
            }
        };
        if segment.index != recording.segment_count
            || segment.index >= MAX_SEGMENTS
            || segment.file != segment_name(segment.index, extension(&recording.mime_type))
            || segment.bytes == 0
            || segment.bytes > MAX_SEGMENT
            || !valid_duration(segment.duration_seconds)
            || !valid_source_range(
                segment.source_start_seconds,
                segment.source_end_seconds,
                segment.duration_seconds,
            )
        {
            valid = false;
            break;
        }
        recording.segment_count += 1;
        recording.bytes_written = recording
            .bytes_written
            .checked_add(segment.bytes)
            .ok_or_else(storage)?;
        recording.duration_seconds += segment.duration_seconds;
        recording.segments.push(segment);
        if recording.segments.len() > MAX_RECENT {
            recording.segments.remove(0);
        }
    }
    // After durable verified publication, source files are no longer playback
    // inputs. They may be absent or Windows delete-pending behind a live reader.
    // Remaining sources are hash/no-follow checked by the cleanup worker itself.
    let cleaned_sources = recording.merge.as_ref().is_some_and(|merge| {
        merge.status == BrowserMergeStatus::Complete
            && merge.source_cleanup.is_some()
            && valid_merge(Some(merge), recording.segment_count, &recording.mime_type)
            && cleanup::merged_files_exist(&root, merge)
    });
    // Avoid reading or hashing historical media. Only bounded recent file metadata
    // and their first 16 bytes are checked; the durable journal contains all totals.
    for segment in &recording.segments {
        if cleaned_sources {
            break;
        }
        let checked = (|| -> Result<bool, StreamError> {
            let mut file = open_regular(&root.join(&segment.file), MAX_SEGMENT)?;
            if file.metadata().map_err(|_| storage())?.len() != segment.bytes {
                return Ok(false);
            }
            let mut header = [0; 16];
            let length = file.read(&mut header).map_err(|_| storage())?;
            Ok(valid_header(
                &header[..length],
                extension(&recording.mime_type),
            ))
        })();
        if !matches!(checked, Ok(true)) {
            valid = false;
            break;
        }
    }
    recording.partial = find_partial(
        &root,
        recording.segment_count,
        extension(&recording.mime_type),
    )?;
    if metadata.as_ref().is_some_and(|metadata| {
        metadata.segment_count > recording.segment_count
            || metadata.bytes_written > recording.bytes_written
            || metadata.duration_seconds > recording.duration_seconds + 0.001
    }) || recording
        .partial
        .as_ref()
        .is_some_and(|partial| partial.bytes > MAX_SEGMENT)
    {
        valid = false;
    }
    if valid {
        // A missing derivative never turns valid original segments into a failed
        // recording. Recheck metadata only, not a full media hash, at startup.
        if let Some(merge) = &mut recording.merge {
            if merge.status == BrowserMergeStatus::Complete
                && (merge.segment_count != recording.segment_count
                    || merge.file.as_ref().is_none_or(|file| {
                        open_regular(&root.join(file), u64::MAX)
                            .and_then(|file| file.metadata().map_err(|_| storage()))
                            .map_or(true, |m| Some(m.len()) != merge.bytes)
                    })
                    || merge.timeline_file.as_ref().is_none_or(|file| {
                        open_regular(&root.join(file), 128 * 1024 * 1024).is_err()
                    }))
            {
                merge.status = BrowserMergeStatus::Failed;
                merge.last_error =
                    Some("이전 병합 파일을 확인하지 못했습니다. 원본 조각은 보존됩니다.".into());
            }
        }
        Ok(())
    } else {
        Err(storage())
    }
}

fn find_partial(root: &Path, index: u64, ext: &str) -> Result<Option<BrowserPartial>, StreamError> {
    for file in [partial_name(index, ext), segment_name(index, ext)] {
        match fs::symlink_metadata(root.join(&file)) {
            Ok(metadata) if safe_regular(&metadata) => {
                return Ok(Some(BrowserPartial {
                    index,
                    file,
                    bytes: metadata.len(),
                }))
            }
            Ok(_) => return Err(storage()),
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(storage()),
        }
    }
    Ok(None)
}

fn read_line(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, StreamError> {
    let mut line = Vec::new();
    reader
        .take((MAX_LINE + 1) as u64)
        .read_until(b'\n', &mut line)
        .map_err(|_| storage())?;
    if line.len() > MAX_LINE {
        return Err(storage());
    }
    // A crash-truncated trailing record is never considered committed.
    if line.last() != Some(&b'\n') {
        return Ok(None);
    }
    Ok(Some(line))
}

fn save_metadata(recording: &BrowserRecording) -> Result<(), StreamError> {
    let root = owned_root(&recording.output_dir, &recording.id)?;
    let bytes = serde_json::to_vec(recording).map_err(|_| storage())?;
    if bytes.len() as u64 > MAX_METADATA {
        return Err(storage());
    }
    atomic_write(&root, "recording.json", &bytes)
}

fn save_catalog(root: &Path, entries: &[CatalogEntry]) -> Result<(), StreamError> {
    let mut bytes = Vec::new();
    for entry in entries {
        bytes.extend(json_line(entry)?);
    }
    if bytes.len() as u64 > MAX_CATALOG {
        return Err(storage());
    }
    atomic_write(root, "catalog.jsonl", &bytes)
}

fn json_line(value: &impl Serialize) -> Result<Vec<u8>, StreamError> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| storage())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_LINE {
        return Err(storage());
    }
    Ok(bytes)
}

pub(super) fn atomic_write(root: &Path, name: &str, bytes: &[u8]) -> Result<(), StreamError> {
    let root = checked_directory(root)?;
    let temporary = root.join(format!(".{name}.{}.partial", uuid::Uuid::new_v4().simple()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| storage())?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| storage())?;
    drop(file);
    rename_closed(&temporary, &root.join(name), true)
}

fn rename_closed(source: &Path, target: &Path, replace: bool) -> Result<(), StreamError> {
    if let Ok(metadata) = fs::symlink_metadata(target) {
        if !replace || !safe_regular(&metadata) {
            return Err(storage());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            },
        };
        let source = source
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let target = target
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let flags = if replace {
            MOVEFILE_WRITE_THROUGH | MOVEFILE_REPLACE_EXISTING
        } else {
            MOVEFILE_WRITE_THROUGH
        };
        unsafe { MoveFileExW(PCWSTR(source.as_ptr()), PCWSTR(target.as_ptr()), flags) }
            .map_err(|_| storage())?;
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, target).map_err(|_| storage())?;
        if let Some(parent) = target.parent() {
            File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(|_| storage())?;
        }
    }
    Ok(())
}

fn checked_directory(path: &Path) -> Result<PathBuf, StreamError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(invalid());
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| storage())?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(storage());
    }
    fs::canonicalize(path).map_err(|_| storage())
}

fn child_directory(parent: &Path, name: &str) -> Result<PathBuf, StreamError> {
    let child = parent.join(name);
    match fs::create_dir(&child) {
        Ok(()) => {}
        Err(cause) if cause.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(storage()),
    }
    checked_directory(&child)
}

fn valid_root_shape(path: &Path, id: &str) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        && path.file_name().and_then(|value| value.to_str()) == Some(id)
        && path
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            == Some("BrowserCapture")
        && path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            == Some("CHZZK")
}

fn owned_root(path: &str, id: &str) -> Result<PathBuf, StreamError> {
    validate_id(id)?;
    let path = Path::new(path);
    if !valid_root_shape(path, id) {
        return Err(storage());
    }
    for ancestor in path.ancestors().take(3) {
        checked_directory(ancestor)?;
    }
    let root = checked_directory(path)?;
    if root != path {
        return Err(storage());
    }
    Ok(root)
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn safe_regular(metadata: &Metadata) -> bool {
    metadata.is_file() && !is_link(metadata)
}

fn open_regular(path: &Path, limit: u64) -> Result<File, StreamError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| storage())?;
    if !safe_regular(&metadata) || metadata.len() > limit {
        return Err(storage());
    }
    let file = File::open(path).map_err(|_| storage())?;
    if !safe_regular(&file.metadata().map_err(|_| storage())?) {
        return Err(storage());
    }
    Ok(file)
}

fn ensure_space(root: &Path, additional: u64) -> Result<(), StreamError> {
    if fs2::available_space(root).map_err(|_| storage())?
        < FREE_SPACE_RESERVE.saturating_add(additional)
    {
        return Err(error(
            "BROWSER_CAPTURE_DISK_FULL",
            "녹화 디스크의 여유 공간이 부족합니다.",
            false,
        ));
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        && uuid::Uuid::parse_str(id).is_ok()
}
fn validate_id(id: &str) -> Result<(), StreamError> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(invalid())
    }
}
fn valid_channel(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn valid_duration(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= MAX_DURATION
}
fn segment_name(index: u64, ext: &str) -> String {
    format!("segment-{index:012}.{ext}")
}
fn partial_name(index: u64, ext: &str) -> String {
    format!("{}.partial", segment_name(index, ext))
}
fn extension(mime: &str) -> &'static str {
    if mime.starts_with("video/webm") {
        "webm"
    } else {
        "mp4"
    }
}

fn normalized_mime(value: &str) -> Option<String> {
    if value.len() > 128 || !value.is_ascii() || value.bytes().any(|byte| byte.is_ascii_control()) {
        return None;
    }
    let value = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != '"')
        .collect::<String>()
        .to_ascii_lowercase();
    if matches!(
        value.as_str(),
        "video/webm" | "video/webm;codecs=vp8,opus" | "video/webm;codecs=vp9,opus" | "video/mp4"
    ) {
        return Some(value);
    }
    let codecs = value.strip_prefix("video/mp4;codecs=")?;
    let (video, audio) = codecs.split_once(',')?;
    if audio != "mp4a.40.2"
        || !(video == "avc1"
            || video.strip_prefix("avc1.").is_some_and(|profile| {
                profile.len() == 6 && profile.bytes().all(|byte| byte.is_ascii_hexdigit())
            }))
    {
        return None;
    }
    Some(value)
}

fn valid_header(bytes: &[u8], ext: &str) -> bool {
    match ext {
        "webm" => bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
        "mp4" => {
            bytes.len() >= 12
                && &bytes[4..8] == b"ftyp"
                && u32::from_be_bytes(bytes[..4].try_into().unwrap()) >= 8
        }
        _ => false,
    }
}

fn bounded_text(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(limit)
        .collect()
}
fn error(code: &str, message: &str, retryable: bool) -> StreamError {
    StreamError::new(code, message, retryable)
}
fn invalid() -> StreamError {
    error(
        "BROWSER_CAPTURE_INVALID",
        "브라우저 녹화 데이터 또는 순서가 올바르지 않습니다.",
        false,
    )
}
fn inactive() -> StreamError {
    error(
        "BROWSER_CAPTURE_INACTIVE",
        "진행 중인 브라우저 녹화를 찾지 못했습니다.",
        false,
    )
}
fn storage() -> StreamError {
    error(
        "BROWSER_CAPTURE_STORAGE",
        "브라우저 녹화 파일을 저장하거나 확인하지 못했습니다.",
        false,
    )
}
fn is_storage_failure(cause: &StreamError) -> bool {
    matches!(
        cause.code.as_str(),
        "BROWSER_CAPTURE_STORAGE" | "BROWSER_CAPTURE_DISK_FULL"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "0123456789abcdef0123456789abcdef";
    const WEBM: &[u8] = &[
        0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x86, 0x81, 1, 0, 0, 0, 0, 0, 0, 0,
    ];

    fn fixture() -> (tempfile::TempDir, BrowserCaptureStore, BrowserRecording) {
        let directory = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(directory.path()).unwrap();
        let recording = store
            .begin(
                directory.path(),
                CHANNEL,
                "capture",
                "video/webm;codecs=vp8,opus",
            )
            .unwrap();
        (directory, store, recording)
    }

    fn fake_merged(job: &BrowserMergeJob) -> BrowserMergedOutput {
        let root = Path::new(&job.recording.output_dir);
        let file = format!("merged-{}.webm", job.token);
        let timeline_file = format!("merged-{}.timeline.jsonl", job.token);
        fs::write(root.join(&file), WEBM).unwrap();
        fs::write(root.join(&timeline_file), b"{}\n").unwrap();
        BrowserMergedOutput {
            file,
            timeline_file,
            bytes: WEBM.len() as u64,
            duration_seconds: 15.0,
            cleanup: None,
        }
    }

    #[test]
    fn merge_only_claims_terminal_records_and_retries_are_token_fenced() {
        let (_directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        store.finish_segment(&recording.id, 0, 15.0).unwrap();
        assert!(store.take_merge_job().unwrap().is_none());
        assert_eq!(store.retry_merges(Some(&recording.id)).unwrap(), 0);
        store.finish(&recording.id, false, None).unwrap();
        let first = store.take_merge_job().unwrap().unwrap();
        assert!(store.take_merge_job().unwrap().is_none());
        assert_eq!(store.retry_merges(None).unwrap(), 0);
        store
            .fail_merge(&first, BrowserMergeStatus::Blocked, "tool missing")
            .unwrap();
        assert!(store.take_merge_job().unwrap().is_none());
        assert_eq!(store.retry_merges(None).unwrap(), 1);
        let second = store.take_merge_job().unwrap().unwrap();
        assert_ne!(first.token, second.token);
        assert!(store.complete_merge(&first, fake_merged(&first)).is_err());
        store
            .fail_merge(&second, BrowserMergeStatus::Queued, "cancelled")
            .unwrap();
        assert_eq!(
            store.snapshot().unwrap()[0].status,
            BrowserRecordingStatus::Stopped
        );
        assert_eq!(
            fs::read(Path::new(&recording.output_dir).join("segment-000000000000.webm")).unwrap(),
            WEBM
        );
    }

    #[test]
    fn completed_merge_reopens_without_changing_source_or_chat_and_checks_missing_output() {
        let (directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        store.finish_segment(&recording.id, 0, 15.0).unwrap();
        store
            .finish_with_chat(&recording.id, false, None, true, "partial", 7)
            .unwrap();
        let root = Path::new(&recording.output_dir);
        fs::write(root.join("chat.jsonl"), b"preserved chat").unwrap();
        let original_journal = fs::read(root.join("segments.jsonl")).unwrap();
        let job = store.take_merge_job().unwrap().unwrap();
        assert_eq!(read_merge_segments(&job).unwrap().len(), 1);
        store.complete_merge(&job, fake_merged(&job)).unwrap();
        let merged_path = store.merged_file(&recording.id).unwrap();
        assert_eq!(store.retry_merges(None).unwrap(), 0);
        assert!(store.merged_file("../escape").is_err());
        drop(store);
        let reopened = BrowserCaptureStore::new(directory.path()).unwrap();
        let saved = reopened.snapshot().unwrap().remove(0);
        assert_eq!(saved.status, BrowserRecordingStatus::Stopped);
        assert_eq!(saved.merge.unwrap().status, BrowserMergeStatus::Complete);
        assert_eq!(saved.chat_status.as_deref(), Some("partial"));
        assert_eq!(saved.chat_count, Some(7));
        assert_eq!(saved.segment_count, 1);
        assert_eq!(
            fs::read(root.join("segments.jsonl")).unwrap(),
            original_journal
        );
        assert_eq!(
            fs::read(root.join("chat.jsonl")).unwrap(),
            b"preserved chat"
        );
        fs::remove_file(merged_path).unwrap();
        assert!(reopened.merged_file(&recording.id).is_err());
        let recovered = BrowserCaptureStore::new(directory.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .remove(0);
        assert_eq!(recovered.status, BrowserRecordingStatus::Stopped);
        assert_eq!(recovered.merge.unwrap().status, BrowserMergeStatus::Failed);
    }

    #[test]
    fn interrupted_merge_recovers_queue_and_changed_journal_rejects_publication() {
        let (directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        store.finish_segment(&recording.id, 0, 15.0).unwrap();
        store.append(&recording.id, 1, 0, WEBM).unwrap();
        store
            .finish(&recording.id, true, Some("synthetic stop"))
            .unwrap();
        let first = store.take_merge_job().unwrap().unwrap();
        drop(store);
        let reopened = BrowserCaptureStore::new(directory.path()).unwrap();
        let recovered = reopened.snapshot().unwrap().remove(0);
        assert_eq!(recovered.status, BrowserRecordingStatus::Interrupted);
        assert_eq!(recovered.merge.unwrap().status, BrowserMergeStatus::Queued);
        assert!(recovered.partial.is_some());
        let job = reopened.take_merge_job().unwrap().unwrap();
        assert_ne!(first.token, job.token);
        let journal = Path::new(&recording.output_dir).join("segments.jsonl");
        OpenOptions::new()
            .append(true)
            .open(journal)
            .unwrap()
            .write_all(b"changed\n")
            .unwrap();
        assert!(read_merge_segments(&job).is_err());
        assert!(reopened.complete_merge(&job, fake_merged(&job)).is_err());
        reopened
            .fail_merge(&job, BrowserMergeStatus::Failed, "journal changed")
            .unwrap();
        assert_eq!(
            reopened.snapshot().unwrap()[0].status,
            BrowserRecordingStatus::Interrupted
        );
        assert!(Path::new(&recording.output_dir)
            .join("segment-000000000001.webm.partial")
            .exists());
    }

    #[test]
    fn merge_publish_metadata_failure_never_claims_success_or_deletes_derivatives() {
        let (_directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        store.finish_segment(&recording.id, 0, 15.0).unwrap();
        store.finish(&recording.id, false, None).unwrap();
        let job = store.take_merge_job().unwrap().unwrap();
        let output = fake_merged(&job);
        let root = Path::new(&recording.output_dir);
        let path = root.join(&output.file);
        fs::remove_file(root.join("recording.json")).unwrap();
        fs::create_dir(root.join("recording.json")).unwrap();
        assert!(store.complete_merge(&job, output).is_err());
        let snapshot = store.snapshot().unwrap().remove(0);
        assert_eq!(snapshot.status, BrowserRecordingStatus::Stopped);
        assert_eq!(snapshot.merge.unwrap().status, BrowserMergeStatus::Failed);
        assert!(path.exists());
        assert_eq!(
            fs::read(root.join("segment-000000000000.webm")).unwrap(),
            WEBM
        );
    }

    #[test]
    fn unavailable_historical_journal_does_not_starve_later_completed_recordings() {
        let (directory, store, first) = fixture();
        let second = store
            .begin(directory.path(), CHANNEL, "second", "video/webm")
            .unwrap();
        for id in [&first.id, &second.id] {
            store.append(id, 0, 0, WEBM).unwrap();
            store.finish_segment(id, 0, 15.0).unwrap();
            store.finish(id, false, None).unwrap();
        }
        fs::remove_file(Path::new(&first.output_dir).join("segments.jsonl")).unwrap();
        let job = store.take_merge_job().unwrap().unwrap();
        assert_eq!(job.recording.id, second.id);
        let first = store
            .snapshot()
            .unwrap()
            .into_iter()
            .find(|r| r.id == first.id)
            .unwrap();
        assert_eq!(first.status, BrowserRecordingStatus::Stopped);
        assert_eq!(first.merge.unwrap().status, BrowserMergeStatus::Blocked);
    }

    #[test]
    fn invalid_derivative_metadata_cannot_expose_a_path_or_upgrade_capture_status() {
        let (directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        store.finish_segment(&recording.id, 0, 15.0).unwrap();
        store
            .finish(&recording.id, true, Some("synthetic interruption"))
            .unwrap();
        let job = store.take_merge_job().unwrap().unwrap();
        store.complete_merge(&job, fake_merged(&job)).unwrap();
        let mut metadata = store.snapshot().unwrap().remove(0);
        metadata.merge.as_mut().unwrap().file = Some("../outside.webm".into());
        fs::write(
            Path::new(&recording.output_dir).join("recording.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        let reopened = BrowserCaptureStore::new(directory.path()).unwrap();
        let recovered = reopened.snapshot().unwrap().remove(0);
        assert_eq!(recovered.status, BrowserRecordingStatus::Interrupted);
        assert_eq!(recovered.merge.unwrap().status, BrowserMergeStatus::Failed);
        assert!(reopened.merged_file(&recording.id).is_err());
    }

    #[test]
    fn interleaved_recordings_keep_independent_indices_files_and_source_ranges() {
        let (dir, store, first) = fixture();
        let second = store
            .begin(
                dir.path(),
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "second",
                "video/webm",
            )
            .unwrap();
        store.append(&first.id, 0, 0, WEBM).unwrap();
        store.append(&second.id, 0, 0, WEBM).unwrap();
        store.append(&first.id, 0, 1, b"first").unwrap();
        store.append(&second.id, 0, 1, b"second").unwrap();
        assert!(store
            .finish_segment_source(&first.id, 0, 15.0, Some(10.0), Some(24.0))
            .is_err());
        let committed = store
            .finish_segment_source(&first.id, 0, 15.0, Some(10.0), Some(25.0))
            .unwrap();
        assert_eq!(committed.segments[0].source_start_seconds, Some(10.0));
        assert!(store.finish_segment(&first.id, 0, 15.0).is_err());
        store.finish(&first.id, false, None).unwrap();
        assert!(store.has_active());
        store.finish_segment(&second.id, 0, 15.0).unwrap();
        store.finish(&second.id, false, None).unwrap();
        drop(store);
        let restored = BrowserCaptureStore::new(dir.path())
            .unwrap()
            .snapshot()
            .unwrap();
        assert_eq!(restored.len(), 2);
        let a = restored.iter().find(|r| r.id == first.id).unwrap();
        let b = restored.iter().find(|r| r.id == second.id).unwrap();
        assert_ne!(a.output_dir, b.output_dir);
        assert_eq!(a.segments[0].source_end_seconds, Some(25.0));
        assert_eq!(b.segments[0].source_start_seconds, None);
        assert_eq!(
            fs::read(Path::new(&a.output_dir).join(&a.segments[0].file)).unwrap(),
            [WEBM, b"first"].concat()
        );
        assert_eq!(
            fs::read(Path::new(&b.output_dir).join(&b.segments[0].file)).unwrap(),
            [WEBM, b"second"].concat()
        );
    }

    #[test]
    fn chat_gap_is_durable_without_marking_valid_video_as_failed_and_legacy_is_unknown() {
        let (dir, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        store.finish_segment(&recording.id, 0, 15.0).unwrap();
        let done = store
            .finish_with_chat(&recording.id, false, None, true, "storage_failed", 17)
            .unwrap();
        assert_eq!(done.status, BrowserRecordingStatus::Stopped);
        assert_eq!(done.chat_status.as_deref(), Some("storage_failed"));
        let old = store
            .begin(dir.path(), CHANNEL, "old metadata", "video/webm")
            .unwrap();
        store.finish(&old.id, true, None).unwrap();
        drop(store);
        let recovered = BrowserCaptureStore::new(dir.path())
            .unwrap()
            .snapshot()
            .unwrap();
        let saved = recovered.iter().find(|r| r.id == recording.id).unwrap();
        assert_eq!(saved.capture_chat, Some(true));
        assert_eq!(saved.chat_count, Some(17));
        assert_eq!(saved.chat_status.as_deref(), Some("storage_failed"));
        let legacy = recovered.iter().find(|r| r.id == old.id).unwrap();
        assert_eq!(legacy.capture_chat, None);
        assert_eq!(legacy.chat_status, None);
        assert_eq!(legacy.chat_count, None);
    }
    #[test]
    fn invalid_chat_summary_and_unwritable_warning_never_report_success_or_remove_video() {
        let (_dir, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        let committed = store.finish_segment(&recording.id, 0, 15.0).unwrap();
        assert!(store
            .finish_with_chat(&recording.id, false, None, true, "not-a-status", 0)
            .is_err());
        assert!(store
            .finish_with_chat(&recording.id, false, None, true, "partial", u64::MAX)
            .is_err());
        assert!(store
            .finish_with_chat(&recording.id, false, None, false, "disabled", 1)
            .is_err());
        let root = Path::new(&recording.output_dir);
        let media = root.join(&committed.segments[0].file);
        let before = fs::read(&media).unwrap();
        fs::remove_file(root.join("recording.json")).unwrap();
        fs::create_dir(root.join("recording.json")).unwrap();
        assert!(store
            .finish_with_chat(&recording.id, false, None, true, "partial", 7)
            .is_err());
        assert_eq!(fs::read(media).unwrap(), before);
        let failed = store.snapshot().unwrap().remove(0);
        assert_eq!(failed.status, BrowserRecordingStatus::Failed);
        assert_eq!(failed.chat_status.as_deref(), Some("partial"));
    }

    #[test]
    fn committed_segments_survive_reopen_and_duplicate_bytes_are_not_appended() {
        let (directory, store, recording) = fixture();
        let first = store.append(&recording.id, 0, 0, WEBM).unwrap();
        assert!(!first.duplicate);
        let retry = store.append(&recording.id, 0, 0, WEBM).unwrap();
        assert!(retry.duplicate);
        assert_eq!(retry.bytes_accepted, 0);
        assert!(store.append(&recording.id, 0, 0, &[0; 16]).is_err());
        assert!(store.append(&recording.id, 0, 2, b"later").is_err());
        store.append(&recording.id, 0, 1, b"tail").unwrap();
        let committed = store.finish_segment(&recording.id, 0, 15.0).unwrap();
        assert_eq!(committed.bytes_written, WEBM.len() as u64 + 4);
        assert!(committed.partial.is_none());
        assert!(
            store
                .append(&recording.id, 0, 1, b"tail")
                .unwrap()
                .duplicate
        );
        assert!(store.append(&recording.id, 0, 1, b"fail").is_err());
        assert_eq!(
            store.finish_segment(&recording.id, 0, 15.0).unwrap(),
            committed
        );
        assert!(store.finish_segment(&recording.id, 0, 16.0).is_err());
        store.finish(&recording.id, false, None).unwrap();
        drop(store);
        let restored = BrowserCaptureStore::new(directory.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .remove(0);
        assert_eq!(restored.status, BrowserRecordingStatus::Stopped);
        assert_eq!(restored.segment_count, 1);
        assert_eq!(restored.bytes_written, WEBM.len() as u64 + 4);
        assert_eq!(
            fs::read(Path::new(&restored.output_dir).join(&restored.segments[0].file)).unwrap(),
            [WEBM, b"tail"].concat()
        );
    }

    #[test]
    fn crash_keeps_partial_and_valid_journal_prefix_without_rewriting_files() {
        let (directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        store.finish_segment(&recording.id, 0, 15.0).unwrap();
        store.append(&recording.id, 1, 0, WEBM).unwrap();
        drop(store);
        let root = Path::new(&recording.output_dir);
        let journal = root.join("segments.jsonl");
        OpenOptions::new()
            .append(true)
            .open(&journal)
            .unwrap()
            .write_all(b"{\"index\":1")
            .unwrap();
        let before = fs::read(&journal).unwrap();
        let restored = BrowserCaptureStore::new(directory.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .remove(0);
        assert_eq!(restored.status, BrowserRecordingStatus::Interrupted);
        assert_eq!(restored.segment_count, 1);
        assert_eq!(restored.partial.unwrap().index, 1);
        assert_eq!(fs::read(journal).unwrap(), before);
        assert!(root.join(partial_name(1, "webm")).is_file());
    }

    #[test]
    fn unjournaled_renamed_file_is_reported_as_unconfirmed() {
        let (directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        drop(store);
        let root = Path::new(&recording.output_dir);
        fs::rename(
            root.join(partial_name(0, "webm")),
            root.join(segment_name(0, "webm")),
        )
        .unwrap();
        let restored = BrowserCaptureStore::new(directory.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .remove(0);
        assert_eq!(restored.status, BrowserRecordingStatus::Interrupted);
        assert_eq!(restored.segment_count, 0);
        assert_eq!(restored.partial.unwrap().file, segment_name(0, "webm"));
    }

    #[test]
    fn failed_commit_preserves_existing_destination_and_never_reports_stopped() {
        let (directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        let root = Path::new(&recording.output_dir);
        let final_file = root.join(segment_name(0, "webm"));
        fs::write(&final_file, b"existing").unwrap();
        assert!(store.finish_segment(&recording.id, 0, 15.0).is_err());
        assert!(!store.has_active());
        assert_eq!(fs::read(final_file).unwrap(), b"existing");
        assert!(root.join(partial_name(0, "webm")).is_file());
        assert_eq!(
            store.finish(&recording.id, false, None).unwrap().status,
            BrowserRecordingStatus::Failed
        );
        drop(store);
        assert_eq!(
            BrowserCaptureStore::new(directory.path())
                .unwrap()
                .snapshot()
                .unwrap()[0]
                .status,
            BrowserRecordingStatus::Failed
        );
    }

    #[test]
    fn four_active_captures_share_one_catalog_and_shutdown_preserves_each_partial() {
        let (directory, store, recording) = fixture();
        let mut ids = vec![recording.id.clone()];
        for n in 1..4 {
            ids.push(
                store
                    .begin(
                        directory.path(),
                        CHANNEL,
                        &format!("stream {n}"),
                        "video/webm",
                    )
                    .unwrap()
                    .id,
            );
        }
        assert!(store
            .begin(directory.path(), CHANNEL, "fifth", "video/mp4")
            .is_err());
        for id in &ids {
            store.append(id, 0, 0, WEBM).unwrap();
        }
        store.shutdown().unwrap();
        assert!(!store.has_active());
        for stopped in store.snapshot().unwrap() {
            assert_eq!(stopped.status, BrowserRecordingStatus::Interrupted);
            assert!(stopped.partial.is_some());
        }
        assert!(store
            .begin(directory.path(), CHANNEL, "late", "video/mp4")
            .is_err());
        assert_eq!(
            store.finish(&recording.id, false, None).unwrap().status,
            BrowserRecordingStatus::Interrupted
        );
    }

    #[test]
    fn rejects_traversal_mime_header_and_chunk_or_duration_limits() {
        let (directory, store, recording) = fixture();
        for id in [
            "../recording",
            "01234567-89ab-cdef-0123-456789abcdef",
            "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
        ] {
            assert!(store.append(id, 0, 0, WEBM).is_err());
        }
        assert!(store.append(&recording.id, 1, 0, WEBM).is_err());
        assert!(store.append(&recording.id, 0, 1, WEBM).is_err());
        assert!(store
            .append(&recording.id, 0, 0, b"<html>bad</html>")
            .is_err());
        assert!(store
            .append(&recording.id, 0, 0, &vec![0; MAX_CHUNK + 1])
            .is_err());
        assert!(!chunk_fits(MAX_SEGMENT, 1, 1));
        assert!(!chunk_fits(0, 1, MAX_CHUNKS));
        assert!(chunk_fits(MAX_SEGMENT - MAX_CHUNK as u64, MAX_CHUNK, 63));
        for duration in [0.0, -1.0, f64::NAN, f64::INFINITY, MAX_DURATION + 1.0] {
            assert!(store.finish_segment(&recording.id, 0, duration).is_err());
        }
        for mime in [
            "text/html",
            "video/webm;codecs=av1",
            "video/mp4;codecs=evil,mp4a.40.2",
            "video/mp4\n",
        ] {
            assert!(normalized_mime(mime).is_none());
        }
        assert!(checked_directory(Path::new("relative")).is_err());
        assert!(checked_directory(&directory.path().join("..")).is_err());
        assert!(normalized_mime("video/mp4; codecs=\"avc1.42E01E,mp4a.40.2\"").is_some());
        assert!(normalized_mime("video/mp4;codecs=avc1,mp4a.40.2").is_some());
    }

    #[test]
    fn mp4_header_and_independent_segment_index_are_checked() {
        let directory = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(directory.path()).unwrap();
        let recording = store
            .begin(directory.path(), CHANNEL, "mp4", "video/mp4")
            .unwrap();
        let bytes = b"\0\0\0\x18ftypisom\0\0\0\0isommp42";
        store.append(&recording.id, 0, 0, bytes).unwrap();
        let finished = store.finish_segment(&recording.id, 0, 14.5).unwrap();
        assert!(finished.segments[0].file.ends_with(".mp4"));
        assert!(store
            .append(
                &recording.id,
                1,
                0,
                b"\0\0\0\x18moofnot-an-independent-segment"
            )
            .is_err());
        store.append(&recording.id, 1, 0, bytes).unwrap();
        assert_eq!(
            store
                .finish_segment(&recording.id, 1, 1.5)
                .unwrap()
                .duration_seconds,
            16.0
        );
    }

    #[test]
    fn malformed_metadata_counts_do_not_override_the_durable_journal() {
        let (directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        let mut metadata = store.finish_segment(&recording.id, 0, 15.0).unwrap();
        drop(store);
        metadata.segment_count = 9;
        metadata.bytes_written = 99_999;
        metadata.status = BrowserRecordingStatus::Stopped;
        fs::write(
            Path::new(&recording.output_dir).join("recording.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        let restored = BrowserCaptureStore::new(directory.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .remove(0);
        assert_eq!(restored.status, BrowserRecordingStatus::Failed);
        assert_eq!(restored.segment_count, 1);
        assert_eq!(restored.bytes_written, WEBM.len() as u64);
    }

    #[test]
    fn damaged_or_stale_summary_still_recovers_committed_segments_and_partial() {
        let (directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        store.finish_segment(&recording.id, 0, 15.0).unwrap();
        store.append(&recording.id, 1, 0, WEBM).unwrap();
        drop(store);
        let path = Path::new(&recording.output_dir).join("recording.json");
        // Crash after journal sync but before summary replacement.
        fs::write(&path, serde_json::to_vec(&recording).unwrap()).unwrap();
        let recovered = BrowserCaptureStore::new(directory.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .remove(0);
        assert_eq!(recovered.status, BrowserRecordingStatus::Interrupted);
        assert_eq!(recovered.segment_count, 1);
        assert_eq!(recovered.partial.unwrap().index, 1);
        fs::write(&path, b"{damaged").unwrap();
        let recovered = BrowserCaptureStore::new(directory.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .remove(0);
        assert_eq!(recovered.status, BrowserRecordingStatus::Failed);
        assert_eq!(recovered.segment_count, 1);
        assert_eq!(recovered.bytes_written, WEBM.len() as u64);
        assert!(recovered.partial.is_some());
    }

    #[test]
    fn recovered_memory_is_bounded_while_full_journal_totals_are_retained() {
        let (directory, store, recording) = fixture();
        drop(store);
        let root = Path::new(&recording.output_dir);
        let mut journal = OpenOptions::new()
            .append(true)
            .open(root.join("segments.jsonl"))
            .unwrap();
        for index in 0..(MAX_RECENT as u64 + 2) {
            let segment = BrowserSegment {
                index,
                file: segment_name(index, "webm"),
                bytes: WEBM.len() as u64,
                duration_seconds: 15.0,
                source_start_seconds: None,
                source_end_seconds: None,
            };
            fs::write(root.join(&segment.file), WEBM).unwrap();
            journal.write_all(&json_line(&segment).unwrap()).unwrap();
        }
        drop(journal);
        let recovered = BrowserCaptureStore::new(directory.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .remove(0);
        assert_eq!(recovered.segment_count, MAX_RECENT as u64 + 2);
        assert_eq!(recovered.segments.len(), MAX_RECENT);
        assert_eq!(recovered.segments[0].index, 2);
        assert_eq!(
            recovered.bytes_written,
            (MAX_RECENT as u64 + 2) * WEBM.len() as u64
        );
        assert_eq!(recovered.duration_seconds, (MAX_RECENT + 2) as f64 * 15.0);
    }

    #[test]
    fn catalog_record_count_is_bounded_without_reading_media() {
        let (_directory, store, recording) = fixture();
        let original = store.lock().unwrap().catalog[0].clone();
        let parent = Path::new(&recording.output_dir).parent().unwrap();
        let mut bytes = Vec::new();
        for index in 0..=MAX_RECORDINGS {
            let id = format!("{index:032x}");
            let entry = CatalogEntry {
                id: id.clone(),
                output_dir: parent.join(id).to_string_lossy().into_owned(),
                ..original.clone()
            };
            bytes.extend(json_line(&entry).unwrap());
        }
        let catalog = store.catalog_root.join("catalog.jsonl");
        fs::write(&catalog, bytes).unwrap();
        assert!(read_catalog(&catalog).is_err());
    }

    #[test]
    fn journal_corruption_and_missing_final_file_never_report_success() {
        let (directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        store.finish_segment(&recording.id, 0, 15.0).unwrap();
        store.finish(&recording.id, false, None).unwrap();
        drop(store);
        let root = Path::new(&recording.output_dir);
        OpenOptions::new()
            .append(true)
            .open(root.join("segments.jsonl"))
            .unwrap()
            .write_all(b"{bad}\n")
            .unwrap();
        let restored = BrowserCaptureStore::new(directory.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .remove(0);
        assert_eq!(restored.status, BrowserRecordingStatus::Failed);
        assert_eq!(restored.segment_count, 1);
        fs::remove_file(root.join(segment_name(0, "webm"))).unwrap();
        assert_eq!(
            BrowserCaptureStore::new(directory.path())
                .unwrap()
                .snapshot()
                .unwrap()[0]
                .status,
            BrowserRecordingStatus::Failed
        );
    }

    #[test]
    fn oversized_lines_and_catalogs_are_rejected_and_truncated_tail_ignored() {
        let mut reader = BufReader::new(std::io::Cursor::new(vec![b'x'; MAX_LINE + 1]));
        assert!(read_line(&mut reader).is_err());
        let mut reader = BufReader::new(std::io::Cursor::new(b"{partial"));
        assert!(read_line(&mut reader).unwrap().is_none());
        let directory = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(directory.path()).unwrap();
        let path = store.catalog_root.join("catalog.jsonl");
        File::create(&path)
            .unwrap()
            .set_len(MAX_CATALOG + 1)
            .unwrap();
        assert!(BrowserCaptureStore::new(directory.path()).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn metadata_permission_failure_keeps_committed_file_and_failed_state() {
        let (directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        let path = Path::new(&recording.output_dir).join("recording.json");
        let original_permissions = fs::metadata(&path).unwrap().permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions.clone()).unwrap();
        assert!(store.finish_segment(&recording.id, 0, 15.0).is_err());
        let snapshot = store.snapshot().unwrap().remove(0);
        assert_eq!(snapshot.status, BrowserRecordingStatus::Failed);
        assert_eq!(snapshot.segment_count, 1);
        assert!(Path::new(&recording.output_dir)
            .join(&snapshot.segments[0].file)
            .is_file());
        fs::set_permissions(path, original_permissions).unwrap();
        drop(store);
        assert_ne!(
            BrowserCaptureStore::new(directory.path())
                .unwrap()
                .snapshot()
                .unwrap()[0]
                .status,
            BrowserRecordingStatus::Stopped
        );
    }

    #[test]
    fn dto_exposes_partial_separately_from_committed_totals() {
        let (_directory, store, recording) = fixture();
        store.append(&recording.id, 0, 0, WEBM).unwrap();
        let value = serde_json::to_value(store.snapshot().unwrap().remove(0)).unwrap();
        assert_eq!(value["status"], "recording");
        assert_eq!(value["bytesWritten"], 0);
        assert_eq!(value["partial"]["bytes"], WEBM.len() as u64);
        assert_eq!(value["segmentCount"], 0);
    }
}
