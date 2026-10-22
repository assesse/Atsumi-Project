//! YouTube-only owner of unchanged, durably acknowledged VP9/WebM + AAC bytes.
//! Metadata admission is not media verification; only the existing selected
//! finalizer/publication capability may produce a library entry.
use super::*;
use crate::platform_media::youtube_spool as spool;
use std::time::Instant;

const FATAL_FILE: &str = "youtube-raw-fatal.json";
const FATAL: &str = "원본 녹화 입력의 무결성이 변경되었습니다. 자동 복구하지 않고 원본을 보존했습니다.";
const MAX_UNITS: u64 = 100_000;

/// Fixed native diagnostic latch, not a path/recording/publication capability.
/// Preserve it when a separate raw-fatal marker could not be durably written.
pub(in crate::platform_media) fn integrity_failed(job: &MediaJob) -> bool {
    job.reference.provider == "youtube" && job.reference.kind == "live" && job.error.as_deref() == Some(FATAL)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RawInitDecision { Accepted, Changed }

#[derive(Clone, PartialEq, Eq)]
struct InitReceipt { unit: u64, start: u64, bytes: Vec<u8> }
struct TrackState {
    role: spool::Track,
    configuration: [u8; 32],
    timebase: u64,
    length: u64,
    next_unit: u64,
    initialization: Option<InitReceipt>,
    matched: usize,
    payload: bool,
    configuration_changed: bool,
}
impl TrackState {
    fn inspect(&self, bytes: &[u8], cancel: &AtomicBool) -> Result<spool::RawInitialization, String> {
        match self.role {
            spool::Track::Video => spool::inspect_vp9_initialization(bytes, cancel),
            spool::Track::Audio => spool::inspect_aac_initialization(bytes, cancel),
        }.map_err(|_| FATAL.into())
    }
    fn observe(&mut self, unit: u64, start: u64, bytes: &[u8], cancel: &AtomicBool) -> Result<RawInitDecision, String> {
        if self.configuration_changed { return Ok(RawInitDecision::Changed); }
        if self.initialization.as_ref().is_some_and(|last| last.unit == unit && last.start == start && last.bytes == bytes) {
            return Ok(RawInitDecision::Accepted);
        }
        if unit != self.next_unit || unit >= MAX_UNITS || start != self.length
            || self.initialization.as_ref().is_some_and(|last| self.matched != last.bytes.len() || self.length <= last.start + last.bytes.len() as u64)
        { return Err(FATAL.into()); }
        let facts = self.inspect(bytes, cancel)?;
        if facts.configuration_sha256() != self.configuration || facts.timebase() != self.timebase {
            self.configuration_changed = true;
            return Ok(RawInitDecision::Changed);
        }
        self.initialization = Some(InitReceipt { unit, start, bytes: bytes.to_vec() });
        self.matched = 0;
        self.next_unit += 1;
        Ok(RawInitDecision::Accepted)
    }
    fn check_bytes(&self, bytes: &[u8]) -> Result<usize, String> {
        if self.configuration_changed { return Err(FATAL.into()); }
        let init = self.initialization.as_ref().ok_or(FATAL)?;
        let count = bytes.len().min(init.bytes.len().saturating_sub(self.matched));
        if bytes[..count] != init.bytes[self.matched..self.matched + count] { return Err(FATAL.into()); }
        Ok(count)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FatalProof { version: u8, identity: spool::SpoolIdentity, reason: FatalReason }
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FatalReason { InputIntegrity }

struct RawWriter {
    lease: OfficialReceiverLease,
    _ownership: lifecycle::JobOwnership,
    job: MediaJob,
    directory: PathBuf,
    identity: spool::SpoolIdentity,
    cancel: Arc<AtomicBool>,
    spool: Option<spool::SpoolWriter>,
    tracks: BTreeMap<u32, TrackState>,
    previous: Option<(Receipt, Option<u64>, EncodedAck)>,
    begin: Instant,
    boundary_attempted: bool,
    boundary_accepted: bool,
    poisoned: bool,
    interrupted: bool,
}
pub(crate) struct YouTubeRawArchive { writer: Option<RawWriter> }

fn track_inputs(tracks: &[EncodedTrack], cancel: &AtomicBool) -> Result<BTreeMap<u32, TrackState>, String> {
    if tracks.len() != 2 { return Err(FATAL.into()); }
    let mut states = BTreeMap::new();
    for track in tracks {
        if track.track_index > 1 || track.mime_type.len() > 120 { return Err(FATAL.into()); }
        let mime: String = track.mime_type.to_ascii_lowercase().chars().filter(|value| !value.is_ascii_whitespace() && *value != '"').collect();
        let video_hint = mime == "video/webm;codecs=vp9" || mime.strip_prefix("video/webm;codecs=vp09.").is_some_and(|value| {
            let parts: Vec<_> = value.split('.').collect();
            (3..=8).contains(&parts.len()) && parts.iter().all(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_digit()))
        });
        let facts = if video_hint {
            spool::inspect_vp9_initialization(&track.init, cancel)
        } else if mime == "audio/mp4;codecs=mp4a.40.2" {
            spool::inspect_aac_initialization(&track.init, cancel)
        } else { return Err(FATAL.into()); }.map_err(|_| FATAL)?;
        if states.values().any(|value: &TrackState| value.role == facts.track()) { return Err(FATAL.into()); }
        if states.insert(track.track_index, TrackState {
            role: facts.track(), configuration: facts.configuration_sha256(), timebase: facts.timebase(),
            length: 0, next_unit: 0, initialization: None, matched: 0, payload: false, configuration_changed: false,
        }).is_some() { return Err(FATAL.into()); }
    }
    Ok(states)
}

impl OfficialReceiverLease {
    pub(crate) fn begin_youtube_raw(
        &self, source_id: String, document_id: String, document_epoch: u64,
        source_generation: u64, request_id: String, tracks: Vec<EncodedTrack>,
        title: String, channel_name: String, owner: live::RecordingOwner,
    ) -> Result<YouTubeRawArchive, String> {
        if self.receiver.reference.provider != "youtube" || self.receiver.reference.kind != "live"
            || self.is_closed() || !uuid(&source_id) || !uuid(&document_id) || !uuid(&request_id)
            || document_epoch == 0 || source_generation == 0 { return Err(FATAL.into()); }
        let channel = {
            let binding = self.receiver.youtube_chat_binding.lock().map_err(|_| ERROR)?;
            binding.as_ref().ok_or("YouTube 원본 플레이어의 채팅 보존 경로를 아직 확인하지 못했습니다.")?.channel_id.clone()
        };
        if let live::RecordingOwner::Scheduled { schedule_id, .. } = &owner {
            if schedule_id != &channel { return Err(FATAL.into()); }
        }
        validate_owner(&owner, &self.receiver.reference, &self.receiver.broadcast_id)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let tracks = track_inputs(&tracks, &cancel)?;
        let timebase = |role| tracks.values().find(|track| track.role == role).map(|track| track.timebase).ok_or(FATAL);
        let clock = spool::ClockDescriptor {
            video_timestamp_scale_ns: timebase(spool::Track::Video)?, audio_timescale: timebase(spool::Track::Audio)?,
            mode: spool::Mode::Segments, append_window: spool::Window::Default,
            video_timestamp_offset: 0, audio_timestamp_offset: 0,
        };
        let mut recording = self.receiver.recording.lock().map_err(|_| ERROR)?;
        if recording.is_some() { return Err("이미 녹화 중이거나 마무리 중입니다.".into()); }
        let mut state = self.service.inner.state.lock().map_err(|_| ERROR)?;
        if state.shutting_down || self.is_closed() { return Err("앱 또는 플레이어를 종료하는 중입니다.".into()); }
        if state.jobs.values().any(|job| job.request_id == request_id) { return Err("이미 처리한 녹화 요청입니다.".into()); }
        if state.jobs.values().filter(|job| job.active()).count() >= queries::ACTIVE_JOB_LIMIT { return Err("진행 중인 작업이 많습니다.".into()); }
        let job = MediaJob {
            broadcast_id: Some(self.receiver.broadcast_id.clone()), job_id: uuid::Uuid::new_v4().to_string(), request_id,
            reference: self.receiver.reference.clone(), revision: 1, state: "recording".into(),
            title: clean(Some(&title), 500), channel_name: clean(Some(&channel_name), 200), created_at: now(),
            received_bytes: 0, total_bytes: None, error: None, library: None, download_root: self.receiver.download_root.clone(),
        };
        let identity = spool::SpoolIdentity {
            job_id: job.job_id.clone(), request_id: job.request_id.clone(), receiver_id: self.receiver.id.clone(),
            source_id: source_id.clone(), document_id, document_epoch, source_generation,
            video_id: self.receiver.broadcast_id.clone(), channel_id: channel, owner: owner.clone().into(),
        };
        let directory = storage::job_dir(&job.download_root, "youtube", &job.job_id)?;
        let parent = directory.parent().ok_or(ERROR)?;
        storage::safe_path(parent)?;
        fs::create_dir_all(parent).map_err(|_| ERROR)?;
        // The spool exclusively creates the job directory. Do not precreate it.
        let spool = spool::SpoolWriter::create_new(&job.download_root, identity.clone(), clock, cancel.clone()).map_err(|_| ERROR)?;
        write_new_json(&directory.join(PROOF), &Proof {
            version: 1, reference: job.reference.clone(), broadcast_id: self.receiver.broadcast_id.clone(),
            receiver_id: self.receiver.id.clone(), source_id, generation: document_epoch,
            request_id: job.request_id.clone(), job_id: job.job_id.clone(), owner: owner.clone(),
        })?;
        write_new_json(&directory.join("recording-owner.json"), &owner)?;
        self.receiver.chat.attach_recording_archive(&directory, "youtube", &job.job_id)?;
        if let Err(error) = storage::save_job(&self.service.inner.journal, &job) {
            let _ = self.receiver.chat.detach_youtube_raw_archive(&job.job_id);
            return Err(error);
        }
        state.cancel.insert(job.job_id.clone(), cancel.clone());
        state.jobs.insert(job.job_id.clone(), job.clone());
        *recording = Some(job.job_id.clone());
        drop(state);
        drop(recording);
        self.service.schedule_profile(&job, &cancel, Some(ProfileReceiver::new(self, owner)));
        Ok(YouTubeRawArchive { writer: Some(RawWriter {
            lease: self.clone(), _ownership: lifecycle::JobOwnership::new(self.service.clone(), job.job_id.clone(), cancel.clone()),
            job, directory, identity, cancel, spool: Some(spool), tracks, previous: None,
            begin: Instant::now(), boundary_attempted: false, boundary_accepted: false, poisoned: false, interrupted: false,
        }) })
    }
}

impl RawWriter {
    fn poison(&mut self) {
        if self.poisoned { return; }
        self.poisoned = true;
        let stored = write_new_json(&self.directory.join(FATAL_FILE), &FatalProof {
            version: 1, identity: self.identity.clone(), reason: FatalReason::InputIntegrity,
        });
        let job_stored = self.lease.service.update(&self.job.job_id, |job| job.error = Some(FATAL.into()));
        if stored.is_err() || job_stored.is_err() {
            let mut state = self.lease.service.inner.state.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(job) = state.jobs.get_mut(&self.job.job_id) { job.error = Some(FATAL.into()); }
            lifecycle::remember_persistence_error(&mut state, &self.job.job_id, lifecycle::persistence_message(FATAL));
        }
    }
    fn append(&mut self, packet: EncodedAppend, boundary: Option<u64>) -> Result<EncodedAck, String> {
        if self.poisoned || self.interrupted || self.cancel.load(Ordering::Acquire) || self.lease.is_closed() { return Err(ERROR.into()); }
        if packet.source_id != self.identity.source_id || packet.generation != self.identity.document_epoch
            || packet.bytes.is_empty() || packet.bytes.len() > MAX_CHUNK { self.poison(); return Err(FATAL.into()); }
        let receipt = Receipt { track: packet.track_index, index: packet.index, chunk: packet.chunk,
            final_chunk: packet.final_chunk, digest: Sha256::digest(&packet.bytes).into() };
        let duplicate = self.previous.as_ref().is_some_and(|(last, hint, _)| last == &receipt && *hint == boundary);
        let track = match self.tracks.get(&packet.track_index) { Some(track) => track, None => { self.poison(); return Err(FATAL.into()); } };
        let matched = if duplicate { 0 } else { match track.check_bytes(&packet.bytes) {
            Ok(count) => count, Err(error) => { self.poison(); return Err(error); }
        } };
        let start = track.length;
        let role = track.role;
        let end = start.checked_add(packet.bytes.len() as u64).ok_or(FATAL)?;
        if !duplicate && boundary.is_some_and(|value| role != spool::Track::Video || self.boundary_attempted
            || value == 0 || value > MAX_APPEND as u64 || value > end || value.saturating_add(16) < start
            || self.begin.elapsed().as_secs_f64() > 15.0) { self.poison(); return Err(FATAL.into()); }
        let chunk = spool::Chunk { track: role, append_index: packet.index, chunk_index: packet.chunk,
            final_chunk: packet.final_chunk, bytes: packet.bytes };
        let observation = if !duplicate { boundary.and_then(|value| {
            self.boundary_attempted = true;
            spool::BoundaryObservation::capture(self.begin, value, &chunk).ok()
        }) } else { None };
        let ack = match self.spool.as_mut().ok_or(ERROR)?.append(&self.identity, chunk) {
            Ok(ack) => ack,
            Err(error) => {
                if matches!(error, spool::Error::Identity | spool::Error::Proof | spool::Error::Order | spool::Error::Bounds | spool::Error::Corrupt) { self.poison(); }
                else { self.interrupted = true; }
                return Err(ERROR.into());
            }
        };
        if ack.duplicate != duplicate { self.poison(); return Err(FATAL.into()); }
        if !ack.duplicate {
            let track = self.tracks.get_mut(&packet.track_index).ok_or(ERROR)?;
            if ack.committed_end != end { self.poison(); return Err(FATAL.into()); }
            track.matched += matched;
            track.length = ack.committed_end;
            if let Some(init) = &track.initialization { track.payload |= track.length > init.start + init.bytes.len() as u64; }
            if boundary.is_some() { self.boundary_accepted = true; }
            if let Some(observation) = observation {
                // Missing timing cannot turn durable media bytes into a false NACK.
                let _ = observation.persist(&self.directory, &self.identity, &ack, &self.cancel);
            }
            if self.lease.service.update(&self.job.job_id, |job| job.received_bytes = job.received_bytes.saturating_add(ack.committed_end - start)).is_err() {
                self.interrupted = true;
                return Err("원본 바이트는 저장했지만 녹화 상태를 저장하지 못했습니다.".into());
            }
        }
        let answer = EncodedAck { track_index: packet.track_index, index: packet.index, chunk: packet.chunk, duplicate: ack.duplicate };
        self.previous = Some((receipt, boundary, answer.clone()));
        Ok(answer)
    }
}

impl YouTubeRawArchive {
    pub(crate) fn snapshot(&self) -> Result<MediaJob, String> {
        let writer = self.writer.as_ref().ok_or(ERROR)?;
        writer.lease.service.inner.state.lock().map_err(|_| ERROR)?.jobs.get(&writer.job.job_id).cloned().ok_or_else(|| ERROR.into())
    }
    pub(crate) fn is_cancelled(&self) -> bool { self.writer.as_ref().is_none_or(|writer| writer.cancel.load(Ordering::Acquire)) }
    pub(crate) fn startup_ready(&self) -> bool {
        self.writer.as_ref().is_some_and(|writer| !writer.poisoned && !writer.interrupted && writer.boundary_accepted && writer.tracks.values().all(|track| track.payload))
    }
    pub(crate) fn observe_init(&mut self, index: u32, unit: u64, start: u64, bytes: &[u8]) -> Result<RawInitDecision, String> {
        let writer = self.writer.as_mut().ok_or(ERROR)?;
        if writer.poisoned || writer.interrupted || writer.cancel.load(Ordering::Acquire) || writer.lease.is_closed() { return Err(ERROR.into()); }
        let result = writer.tracks.get_mut(&index).ok_or_else(|| FATAL.to_owned()).and_then(|track| track.observe(unit, start, bytes, &writer.cancel));
        if result.is_err() { writer.poison(); }
        result
    }
    pub(crate) fn append(&mut self, packet: EncodedAppend, boundary: Option<u64>) -> Result<EncodedAck, String> {
        self.writer.as_mut().ok_or(ERROR)?.append(packet, boundary)
    }
    pub(crate) fn finish(mut self, interrupted: bool) -> Result<MediaJob, String> {
        dispatch_raw(self.writer.take().ok_or(ERROR)?, interrupted)
    }
}
impl Drop for YouTubeRawArchive {
    fn drop(&mut self) { if let Some(writer) = self.writer.take() { let _ = dispatch_raw(writer, true); } }
}

fn dispatch_raw(mut writer: RawWriter, interrupted: bool) -> Result<MediaJob, String> {
    let service = writer.lease.service.clone();
    let id = writer.job.job_id.clone();
    let chat = writer.lease.receiver.chat.detach_youtube_raw_archive(&id);
    if let Ok(mut active) = writer.lease.receiver.recording.lock() { if active.as_ref() == Some(&id) { *active = None; } }
    if let Some(mut spool) = writer.spool.take() {
        if !writer.poisoned {
            let cause = if interrupted || writer.interrupted { spool::SealCause::Interrupted } else { spool::SealCause::Stopped };
            if spool.seal(&writer.identity, cause).is_err() { writer.interrupted = true; }
        }
        drop(spool); // Release write holdings before any guarded local read.
    }
    let transition_error = service.update(&id, |job| job.state = "finalizing".into()).err();
    let snapshot = service.inner.state.lock().unwrap_or_else(|error| error.into_inner()).jobs.get(&id).cloned().unwrap_or_else(|| writer.job.clone());
    let finalizing = snapshot.clone();
    // Spawn failure must leave a caller-owned task to close/persist; moving the
    // sole detached sink directly into a failed spawn would silently drop it.
    let task = Arc::new(Mutex::new(Some((writer, chat, finalizing, transition_error.clone()))));
    let worker_task = task.clone();
    let worker_service = service.clone();
    let spawn = std::thread::Builder::new().name("youtube-raw-finalize".into()).spawn(move || {
        let Some((writer, chat, finalizing, transition_error)) = worker_task.lock().unwrap_or_else(|error| error.into_inner()).take() else { return; };
        let result = (|| {
            let chat = chat?;
            if writer.poisoned || transition_error.is_some() {
                if let Some(chat) = chat {
                    chat.persist_cancellable(&writer.directory, "youtube", &id, None, &writer.cancel)?;
                }
                return Err(if writer.poisoned { FATAL.into() } else { transition_error.unwrap_or_else(|| ERROR.into()) });
            }
            worker_service.finalize_raw_youtube(finalizing, writer.cancel.clone(), writer.identity.clone(), chat)
        })();
        if let Err(error) = result { worker_service.chat_aware_terminal_error(&id, &writer.cancel, &error); }
        // `writer` retains the receiver and JobOwnership until here.
    });
    if spawn.is_err() {
        if let Some((writer, chat, _, _)) = task.lock().unwrap_or_else(|error| error.into_inner()).take() {
            if let Ok(Some(chat)) = chat {
                let _ = chat.persist_cancellable(&writer.directory, "youtube", &writer.job.job_id, None, &writer.cancel);
            }
            service.chat_aware_terminal_error(&writer.job.job_id, &writer.cancel, "녹화 마무리 작업을 시작하지 못했습니다. 원본은 보존했습니다.");
        }
        return Err("녹화 마무리 작업을 시작하지 못했습니다. 원본은 보존했습니다.".into());
    }
    if let Some(error) = transition_error { return Err(error); }
    Ok(snapshot)
}

pub(in crate::platform_media) fn recover_identity(job: &MediaJob, directory: &Path, cancel: &AtomicBool) -> Result<spool::SpoolIdentity, String> {
    if job.reference.provider != "youtube" || job.reference.kind != "live" || integrity_failed(job)
        || directory != storage::job_dir(&job.download_root, "youtube", &job.job_id)? { return Err(FATAL.into()); }
    match fs::symlink_metadata(directory.join(FATAL_FILE)) {
        Ok(_) => return Err(FATAL.into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(_) => return Err(FATAL.into()),
    }
    let identity = spool::recovery_identity(&job.download_root, &job.job_id, cancel).map_err(|_| FATAL)?;
    let proof = read_proof(job, directory)?;
    let owner_path = directory.join("recording-owner.json");
    if storage::regular(&owner_path)?.len() > 8192 { return Err(FATAL.into()); }
    let owner: live::RecordingOwner = serde_json::from_slice(&fs::read(owner_path).map_err(|_| FATAL)?).map_err(|_| FATAL)?;
    if identity.job_id != job.job_id || identity.request_id != job.request_id
        || identity.video_id != job.reference.external_id || Some(&identity.video_id) != job.broadcast_id.as_ref()
        || identity.receiver_id != proof.receiver_id || identity.source_id != proof.source_id
        || identity.document_epoch != proof.generation || identity.owner != proof.owner.clone().into()
        || owner != proof.owner || cancel.load(Ordering::Acquire) { return Err(FATAL.into()); }
    Ok(identity)
}

#[cfg(test)]
mod tests;
