//! Read-only CHZZK sidecars authorized by an already verified cloud receipt.
//! Admission never opens media, recovers a recording, or changes its catalog.
use super::*;
use crate::cloud_recording::replay::CloudReplayAuthorization;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_CHAT: u64 = 512 * 1024 * 1024;

fn unavailable() -> StreamError {
    error("REPLAY_SIDECAR_UNAVAILABLE", "이 보관 영상과 연결된 채팅·동기화 기록을 확인하지 못했습니다. 원본 기록은 그대로 유지됩니다.", false)
}

struct Held {
    file: File,
    path: PathBuf,
    len: u64,
    modified: std::time::SystemTime,
    created: Option<std::time::SystemTime>,
}
impl Held {
    fn open(path: &Path, limit: u64) -> Result<Self, StreamError> {
        let file = cleanup::verification_guard(path)?;
        let meta = file.metadata().map_err(|_| storage())?;
        if meta.len() > limit { return Err(unavailable()); }
        Ok(Self { file, path: path.to_owned(), len: meta.len(), modified: meta.modified().map_err(|_| storage())?, created: meta.created().ok() })
    }
    fn valid(&self) -> bool {
        let Ok(meta) = self.file.metadata() else { return false; };
        if !safe_regular(&meta) || meta.len() != self.len || meta.modified().ok() != Some(self.modified) || meta.created().ok() != self.created { return false; }
        // Windows denies write/delete sharing and pins every containing directory.
        #[cfg(not(windows))]
        if fs::canonicalize(&self.path).ok().as_ref() != Some(&self.path) { return false; }
        true
    }
    fn bytes(&mut self, cancel: &AtomicBool) -> Result<Vec<u8>, StreamError> {
        self.file.seek(SeekFrom::Start(0)).map_err(|_| storage())?;
        let mut bytes = Vec::new();
        let mut block = [0; 64 * 1024];
        loop {
            if cancel.load(Ordering::Acquire) { return Err(inactive()); }
            let n = self.file.read(&mut block).map_err(|_| storage())?;
            if n == 0 { break; }
            if bytes.len() as u64 + n as u64 > self.len { return Err(unavailable()); }
            bytes.extend_from_slice(&block[..n]);
        }
        if bytes.len() as u64 != self.len || !self.valid() { return Err(unavailable()); }
        Ok(bytes)
    }
}

pub(crate) struct ChatSourceGuard {
    store: BrowserCaptureStore,
    entry: CatalogEntry,
    _lease: Arc<()>,
    _directories: Vec<File>,
    pinned_roots: HashSet<PathBuf>,
    held: Vec<Held>,
}
impl ChatSourceGuard {
    pub(crate) fn valid(&self) -> bool {
        let Ok(state) = self.store.lock() else { return false; };
        let current = state.catalog.iter().find(|e| e.id == self.entry.id);
        let same = current.is_some_and(|e| !e.deletion_pending && e.output_dir == self.entry.output_dir
            && e.channel_id == self.entry.channel_id && e.started_at == self.entry.started_at
            && e.mime_type == self.entry.mime_type && e.media_removed_at == self.entry.media_removed_at);
        if state.closing || !same { return false; }
        // Held Windows handles deny writes/deletion and directory replacement.
        // Do not stat thousands of immutable part proofs for each returned row.
        #[cfg(windows)]
        { true }
        #[cfg(not(windows))]
        { self.held.iter().all(Held::valid) }
    }
    pub(crate) fn fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(self.entry.output_dir.as_bytes());
        for h in &self.held {
            hash.update(h.path.to_string_lossy().as_bytes());
            hash.update(format!("{}|{:?}|{:?}", h.len, h.modified, h.created).as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
    fn hold(&mut self, path: &Path, limit: u64) -> Result<usize, StreamError> {
        #[cfg(windows)]
        {
            let parent = path.parent().ok_or_else(storage)?;
            if self.pinned_roots.insert(parent.to_owned()) {
                self._directories.extend(cleanup::pin_directories(parent)?);
            }
        }
        self.held.push(Held::open(path, limit)?);
        if self.held.iter().map(|h| h.len).try_fold(0u64, |a, b| a.checked_add(b)).is_none_or(|n| n > 768 * 1024 * 1024) { return Err(unavailable()); }
        Ok(self.held.len() - 1)
    }
    fn optional(&mut self, path: &Path, limit: u64) -> Result<Option<File>, StreamError> {
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(storage()),
            Ok(_) => { let i = self.hold(path, limit)?; Ok(Some(self.held[i].file.try_clone().map_err(|_| storage())?)) }
        }
    }
}

pub(crate) struct BrowserChatReplaySource {
    pub guard: ChatSourceGuard,
    pub recording: BrowserRecording,
    pub chat: Option<File>,
    pub viewers: Option<File>,
    pub timeline: Vec<u8>,
    /// Absolute source clock range. Legacy single-part receipts never imply a whole archive.
    pub start: f64,
    pub end: f64,
    pub first_segment: u64,
    pub cohort: String,
    pub warnings: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Proof {
    version: u32,
    recording_id: String,
    media_sha256: String,
    timeline_sha256: String,
    journal_sha256: String,
    segments: Vec<ProofSegment>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProofSegment { index: u64, bytes: u64, sha256: String }

fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn close(a: f64, b: f64) -> bool { a.is_finite() && b.is_finite() && (a - b).abs() <= 0.001 }

impl BrowserCaptureStore {
    pub(crate) fn cloud_chat_source(&self, auth: &CloudReplayAuthorization, cancel: &AtomicBool) -> Result<BrowserChatReplaySource, StreamError> {
        let id = &auth.source.recording_id;
        validate_id(id)?;
        if auth.source.provider != "chzzk" || auth.original_inputs.is_empty() || auth.original_inputs.len() > 4096 || cancel.load(Ordering::Acquire) { return Err(unavailable()); }
        let mut state = self.lock()?;
        if state.closing || state.active.contains_key(id) || state.range_evictions.contains(id)
            || state.deleting.contains_key(id) || state.merge_job.as_ref().is_some_and(|(r, _)| r == id) { return Err(unavailable()); }
        let entry = state.catalog.iter().find(|e| &e.id == id && !e.deletion_pending).cloned().ok_or_else(unavailable)?;
        let lease = state.replay_leases.entry(id.clone()).or_default().clone();
        drop(state);
        let root = owned_root(&entry.output_dir, id)?;
        let mut guard = ChatSourceGuard { store: self.clone(), entry, _lease: lease, _directories: vec![], pinned_roots: HashSet::new(), held: vec![] };
        let metadata = guard.hold(&root.join("recording.json"), MAX_METADATA)?;
        let mut record: BrowserRecording = serde_json::from_slice(&guard.held[metadata].bytes(cancel)?).map_err(|_| unavailable())?;
        let e = &guard.entry;
        if record.id != *id || record.channel_id != e.channel_id || record.output_dir != e.output_dir
            || record.started_at != e.started_at || record.mime_type != e.mime_type || record.deletion_pending
            || record.status == BrowserRecordingStatus::Recording || record.segment_count == 0 || record.segment_count > MAX_SEGMENTS
            || record.segments.len() > MAX_RECENT { return Err(unavailable()); }
        record.media_removed_at = e.media_removed_at;
        if !auth.matches_recording_generation(&format!("{}:{}:{:.9}", record.segment_count, record.bytes_written, record.duration_seconds)) { return Err(unavailable()); }
        let journal_idx = guard.hold(&root.join("segments.jsonl"), MAX_JOURNAL)?;
        let journal = guard.held[journal_idx].bytes(cancel)?;
        let mut hash = Sha256::new();
        let mut journal_hashes = HashMap::new();
        let parts_mode = auth.original_inputs[0].part_index.is_some();
        let all_parts = if parts_mode {
            let i = guard.hold(&root.join("progressive.json"), 4 * 1024 * 1024)?;
            let _ = guard.held[i].bytes(cancel)?;
            parts::load(&record)?
        } else { vec![] };
        let mut needed_ends = HashSet::new();
        for input in &auth.original_inputs {
            needed_ends.insert(if let Some(i) = input.part_index { all_parts.get(i as usize).ok_or_else(unavailable)?.end } else { record.segment_count });
        }
        let mut segments = Vec::new();
        let journal_end = *needed_ends.iter().max().ok_or_else(unavailable)?;
        for line in journal.split_inclusive(|b| *b == b'\n') {
            // A legacy receipt authorizes its committed prefix only. An
            // unrelated unfinalized tail cannot expand or invalidate that range.
            if segments.len() as u64 == journal_end { break; }
            if cancel.load(Ordering::Acquire) { return Err(inactive()); }
            if line.len() > MAX_LINE || line.last() != Some(&b'\n') || segments.len() as u64 >= record.segment_count { return Err(unavailable()); }
            let segment: BrowserSegment = serde_json::from_slice(line).map_err(|_| unavailable())?;
            if segment.index != segments.len() as u64 || segment.bytes == 0 || segment.bytes > MAX_SEGMENT
                || segment.file != segment_name(segment.index, extension(&record.mime_type)) || !valid_duration(segment.duration_seconds)
                || !valid_source_range(segment.source_start_seconds, segment.source_end_seconds, segment.duration_seconds) { return Err(unavailable()); }
            hash.update(line);
            segments.push(segment);
            if needed_ends.contains(&(segments.len() as u64)) { journal_hashes.insert(segments.len() as u64, format!("{:x}", hash.clone().finalize())); }
        }
        if segments.len() as u64 != journal_end { return Err(unavailable()); }
        let resolver = archive::offload::Resolver::new(&record)?;
        let mut timeline = Vec::new();
        let start = auth.original_inputs[0].media_offset_seconds;
        let mut end = start;
        let mut first_segment = 0;
        for (ordinal, input) in auth.original_inputs.iter().enumerate() {
            if cancel.load(Ordering::Acquire) { return Err(inactive()); }
            if input.part_index.is_some() != parts_mode || !cleanup::valid_hash(&input.sha256)
                || input.bytes == 0 || input.duration_seconds <= 0.0 || !close(input.media_offset_seconds, end) { return Err(unavailable()); }
            let (first, stop, proof_name, proof_hash, timeline_name, part) = if let Some(index) = input.part_index {
                let p = all_parts.get(index as usize).ok_or_else(unavailable)?;
                if p.bytes != input.bytes || !close(p.duration, input.duration_seconds) || !close(p.offset, input.media_offset_seconds) { return Err(unavailable()); }
                (p.first, p.end, p.proof_file.clone(), p.proof_sha256.clone(), p.timeline_file.clone(), Some(p))
            } else {
                if auth.original_inputs.len() != 1 || !close(start, 0.0) { return Err(unavailable()); }
                let merge = record.merge.as_ref().filter(|m| m.status == BrowserMergeStatus::Complete).ok_or_else(unavailable)?;
                if !valid_merge(Some(merge), record.segment_count, &record.mime_type) || merge.bytes != Some(input.bytes)
                    || !merge.duration_seconds.is_some_and(|v| close(v, input.duration_seconds)) { return Err(unavailable()); }
                let proof = merge.source_cleanup.as_ref().ok_or_else(unavailable)?;
                (0, record.segment_count, proof.proof_file.clone(), proof.proof_sha256.clone(), merge.timeline_file.clone().ok_or_else(unavailable)?, None)
            };
            if ordinal == 0 { first_segment = first; }
            let pi = guard.hold(&resolver.resolve(&proof_name)?, 64 * 1024 * 1024)?;
            let proof_bytes = guard.held[pi].bytes(cancel)?;
            if digest(&proof_bytes) != proof_hash { return Err(unavailable()); }
            let proof: Proof = serde_json::from_slice(&proof_bytes).map_err(|_| unavailable())?;
            if proof.version != 1 || proof.recording_id != record.id || proof.media_sha256 != input.sha256
                || Some(&proof.journal_sha256) != journal_hashes.get(&stop) || proof.segments.len() as u64 != stop - first { return Err(unavailable()); }
            for (s, p) in segments[first as usize..stop as usize].iter().zip(&proof.segments) {
                if s.index != p.index || s.bytes != p.bytes || !cleanup::valid_hash(&p.sha256) { return Err(unavailable()); }
            }
            let path = resolver.resolve(&timeline_name)?;
            let ti = guard.hold(&path, MAX_JOURNAL)?;
            let bytes = guard.held[ti].bytes(cancel)?;
            if digest(&bytes) != proof.timeline_sha256 { return Err(unavailable()); }
            let rows = if let Some(p) = part { parts::timeline_rows(path.parent().ok_or_else(unavailable)?, p, p.offset, p.duration)? } else { bytes };
            if timeline.len() as u64 + rows.len() as u64 > MAX_JOURNAL { return Err(unavailable()); }
            timeline.extend(rows);
            end += input.duration_seconds;
        }
        let chat = guard.optional(&root.join("chat.jsonl"), MAX_CHAT)?;
        if record.capture_chat == Some(true) && chat.is_none() { return Err(unavailable()); }
        let viewers = guard.optional(&root.join(super::super::viewer_metrics::FILE_NAME), super::super::viewer_metrics::MAX_BYTES)?;
        if !guard.valid() { return Err(unavailable()); }
        let warnings = if auth.description.coverage.state != "complete" { vec!["업로드가 확인된 구간의 채팅만 연결됩니다. 전체 녹화의 완료를 뜻하지 않습니다.".into()] } else { vec![] };
        Ok(BrowserChatReplaySource { guard, recording: record, chat, viewers, timeline, start, end, first_segment, cohort: auth.cohort_id.clone(), warnings })
    }
}
