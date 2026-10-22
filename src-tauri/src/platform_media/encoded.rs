//! Archives only clear bytes already accepted by a provider's official player.
//! No media URL, extractor, account cookies or CHZZK session state enters here.
use super::*;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Write, path::Path, sync::atomic::AtomicUsize};

// Compile the immutable, provider-neutral parser separately. Its only shared
// dependency is the error DTO; there is no CHZZK browser/capture instance.
#[path = "../streaming/browser_fmp4.rs"]
mod fmp4;
pub use fmp4::EncodedTrackInput as EncodedTrack;
#[path = "youtube_aac.rs"]
mod youtube_aac;
#[path = "youtube_fragment.rs"]
mod youtube_fragment;
mod youtube_raw_archive;
pub(super) mod soop_finalize;
pub(crate) use youtube_raw_archive::{YouTubeRawArchive, RawInitDecision};
pub(in crate::platform_media) use youtube_raw_archive::recover_identity as recover_youtube_raw_identity;
pub(in crate::platform_media) use youtube_raw_archive::integrity_failed as raw_youtube_integrity_failed;

pub(super) const PROOF: &str = "encoded-owner.json";
const MANIFEST: &str = "encoded-segments.jsonl";
const MAX_CHUNK: usize = 128 * 1024;
const MAX_APPEND: usize = 32 * 1024 * 1024;
const MAX_SEGMENTS: usize = 100_000;
const ERROR: &str = "원본 플레이어 녹화 데이터가 올바르지 않습니다. 받은 파일은 보존했습니다.";
pub(super) fn same_reference(left: &MediaRef, right: &MediaRef) -> bool {
    left.provider == right.provider
        && left.kind == right.kind
        && if left.provider == "soop" {
            left.external_id.eq_ignore_ascii_case(&right.external_id)
        } else {
            left.external_id == right.external_id
        }
}

struct YouTubeChatBinding {
    channel_id: String,
    producer_started: bool,
}
pub(super) struct OfficialReceiver {
    pub(super) id: String,
    pub(super) reference: MediaRef,
    broadcast_id: String,
    download_root: PathBuf,
    directory: PathBuf,
    leases: AtomicUsize,
    closed: AtomicBool,
    chat: Arc<live_chat::SharedChat>,
    recording: Mutex<Option<String>>,
    profile_commit: Mutex<()>,
    youtube_chat_binding: Mutex<Option<YouTubeChatBinding>>,
    // Never exposed to the page or accepted as a live/source handle.
    screenshot_token: String,
    screenshot_serial: Mutex<()>,
}
impl OfficialReceiver {
    pub(super) fn is_active(&self) -> bool {
        self.leases.load(Ordering::Acquire) > 0 || !self.chat.is_done()
    }
    pub(super) fn close(&self) {
        // Match YouTube admission ordering: close cannot win and then leave a
        // new producer starting behind it. SOOP has no YouTube binding gate.
        let _binding = (self.reference.provider == "youtube").then(|| {
            self.youtube_chat_binding.lock().unwrap_or_else(|error| error.into_inner())
        });
        let _profile = self.profile_commit.lock().unwrap_or_else(|error| error.into_inner());
        self.closed.store(true, Ordering::Release);
        self.chat.stop();
    }
}
/// The host must retain this lease until its native receiver actually closes.
/// Recording holds another lease until finalization has relinquished ownership.
pub struct OfficialReceiverLease {
    service: PlatformMediaService,
    receiver: Arc<OfficialReceiver>,
}
impl Clone for OfficialReceiverLease {
    fn clone(&self) -> Self {
        self.receiver.leases.fetch_add(1, Ordering::AcqRel);
        Self {
            service: self.service.clone(),
            receiver: self.receiver.clone(),
        }
    }
}
impl Drop for OfficialReceiverLease {
    fn drop(&mut self) {
        if self.receiver.leases.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.receiver.close();
        }
    }
}
impl PlatformMediaService {
    /// Read the real finalizer job even after the native view was disposed.
    /// An immutable owner proof fences stale/cross-reservation polling.
    pub fn encoded_scheduled_state(
        &self,
        capture: &live::ScheduledCapture,
    ) -> Result<live::ScheduledState, String> {
        if !uuid(&capture.session_id)
            || !uuid(&capture.job_id)
            || !matches!(capture.owner, live::RecordingOwner::Scheduled { .. })
        {
            return Err(ERROR.into());
        }
        let (job, active) = {
            let state = self.inner.state.lock().map_err(|_| ERROR)?;
            (
                state
                    .jobs
                    .get(&capture.job_id)
                    .cloned()
                    .ok_or("녹화 작업을 찾을 수 없습니다.")?,
                state.cancel.contains_key(&capture.job_id),
            )
        };
        let directory = storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id)?;
        if !capture.owner.valid_for(&job.reference.provider) { return Err(ERROR.into()); }
        let proof = read_proof(&job, &directory)?;
        if proof.owner != capture.owner || proof.receiver_id != capture.session_id {
            return Err(ERROR.into());
        }
        Ok(live::ScheduledState {
            state: job.state,
            error: job.error,
            active,
        })
    }
    /// Call only after the native host has verified the official page's current
    /// broadcast identity. This reserves admission before navigation/video load.
    pub fn reserve_official_receiver(
        &self,
        reference: MediaRef,
        broadcast_id: String,
        receiver_id: String,
        download_root: PathBuf,
    ) -> Result<OfficialReceiverLease, String> {
        reference.canonical_url()?;
        if !broadcast::valid(&reference, &broadcast_id) || !uuid(&receiver_id) {
            return Err(ERROR.into());
        }
        storage::safe_path(&download_root)?;
        let mut registry = self.inner.live.lock().map_err(|_| ERROR)?;
        if self.inner.state.lock().map_err(|_| ERROR)?.shutting_down {
            return Err("앱을 종료하는 중입니다.".into());
        }
        registry.official.retain(|_, r| r.is_active());
        if let Some(receiver) = registry.official.get(&receiver_id) {
            if !same_reference(&receiver.reference, &reference)
                || receiver.broadcast_id != broadcast_id
                || receiver.download_root != download_root
                || receiver.closed.load(Ordering::Acquire)
            {
                return Err(ERROR.into());
            }
            receiver.leases.fetch_add(1, Ordering::AcqRel);
            return Ok(OfficialReceiverLease {
                service: self.clone(),
                receiver: receiver.clone(),
            });
        }
        if registry.legacy_conflict(&reference)
            || registry
                .official
                .values()
                .any(|r| same_reference(&r.reference, &reference))
        {
            return Err("이미 수신 중인 방송입니다. 기존 플레이어를 사용해 주세요.".into());
        }
        if registry.legacy_count() + registry.official.len() >= 4 {
            return Err("라이브는 동시에 네 개까지 재생할 수 있습니다.".into());
        }
        let directory = download_root
            .join(if reference.provider == "soop" {
                "SOOP"
            } else {
                "YouTube"
            })
            .join(".official")
            .join(&receiver_id);
        storage::safe_path(&directory)?;
        fs::create_dir_all(&directory).map_err(|_| "라이브 기록 폴더를 만들지 못했습니다.")?;
        let receiver = Arc::new(OfficialReceiver {
            id: receiver_id.clone(),
            reference: reference.clone(),
            broadcast_id,
            download_root,
            directory: directory.clone(),
            leases: AtomicUsize::new(1),
            closed: AtomicBool::new(false),
            chat: live_chat::SharedChat::new(),
            recording: Mutex::new(None),
            profile_commit: Mutex::new(()),
            youtube_chat_binding: Mutex::new(None),
            screenshot_token: uuid::Uuid::new_v4().to_string(),
            screenshot_serial: Mutex::new(()),
        });
        registry.official.insert(receiver_id, receiver.clone());
        // Chat is receive-only and does not open a second video source. YouTube
        // needs a verified continuation descriptor; until supplied, recording
        // fails closed instead of declaring an empty archive complete.
        #[cfg(test)]
        let synthetic = self.inner.synthetic_live_receiver.load(Ordering::Acquire);
        #[cfg(not(test))]
        let synthetic = false;
        if !synthetic && receiver.reference.provider == "soop" {
            receiver.chat.start_bound(
                self.clone(),
                reference,
                directory,
                Err("원본 플레이어의 채팅 연결 정보를 확인하지 못했습니다.".into()),
                (receiver.reference.provider == "soop").then(|| receiver.broadcast_id.clone()),
            );
        }
        Ok(OfficialReceiverLease {
            service: self.clone(),
            receiver,
        })
    }
}
fn uuid(value: &str) -> bool {
    value.len() == 36 && uuid::Uuid::parse_str(value).is_ok()
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Proof {
    version: u8,
    reference: MediaRef,
    broadcast_id: String,
    receiver_id: String,
    source_id: String,
    generation: u64,
    request_id: String,
    job_id: String,
    owner: live::RecordingOwner,
}
/// Optional portrait authority; never keeps a receiver/viewer lease alive.
pub(in crate::platform_media) struct ProfileReceiver {
    receiver: std::sync::Weak<OfficialReceiver>,
    owner: live::RecordingOwner,
}
impl ProfileReceiver {
    fn new(lease: &OfficialReceiverLease, owner: live::RecordingOwner) -> Self {
        Self { receiver: Arc::downgrade(&lease.receiver), owner }
    }
    pub(in crate::platform_media) fn with_current<R>(&self, job: &MediaJob, operation: impl FnOnce() -> Option<R>) -> Option<R> {
        let receiver = self.receiver.upgrade()?;
        let _commit = receiver.profile_commit.lock().ok()?;
        let recording = receiver.recording.lock().ok()?;
        if receiver.closed.load(Ordering::Acquire) || recording.as_ref() != Some(&job.job_id)
            || receiver.reference != job.reference || Some(&receiver.broadcast_id) != job.broadcast_id.as_ref()
            || receiver.download_root != job.download_root { return None; }
        let directory = storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id).ok()?;
        let proof = read_proof(job, &directory).ok()?;
        if proof.receiver_id != receiver.id || proof.owner != self.owner { return None; }
        operation()
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SegmentProof {
    name: String,
    duration: f64,
    source_start: f64,
    source_end: f64,
    bytes: u64,
    sha256: String,
}
fn write_new_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    storage::safe_path(path)?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| ERROR)?;
    serde_json::to_writer(&mut file, value).map_err(|_| ERROR)?;
    file.sync_all().map_err(|_| ERROR.into())
}
pub(super) fn validate_proof(job: &MediaJob, directory: &Path) -> Result<(), String> {
    read_proof(job, directory).map(|_| ())
}
fn read_proof(job: &MediaJob, directory: &Path) -> Result<Proof, String> {
    let path = directory.join(PROOF);
    if storage::regular(&path)?.len() > 8192 {
        return Err(ERROR.into());
    }
    let proof: Proof =
        serde_json::from_slice(&fs::read(path).map_err(|_| ERROR)?).map_err(|_| ERROR)?;
    if proof.version != 1
        || proof.reference != job.reference
        || Some(&proof.broadcast_id) != job.broadcast_id.as_ref()
        || proof.job_id != job.job_id
        || proof.request_id != job.request_id
        || !uuid(&proof.receiver_id)
        || !uuid(&proof.source_id)
        || proof.generation == 0
        || !proof.owner.valid_for(&proof.reference.provider)
        || !broadcast::valid(&proof.reference, &proof.broadcast_id)
    {
        return Err(ERROR.into());
    }
    validate_owner(&proof.owner, &proof.reference, &proof.broadcast_id)?;
    Ok(proof)
}
fn validate_owner(
    owner: &live::RecordingOwner,
    reference: &MediaRef,
    broadcast: &str,
) -> Result<(), String> {
    if !owner.valid_for(&reference.provider) {
        return Err(ERROR.into());
    }
    if let live::RecordingOwner::Scheduled {
        schedule_id,
        broadcast_key,
        ..
    } = owner
    {
        let matches_source = match reference.provider.as_str() {
            "soop" => schedule_id == &reference.external_id,
            // Canonical channel ownership is proven by the native YouTube host
            // before arming; media identity remains the exact live video ID.
            "youtube" => reference.kind == "live" && reference.external_id == broadcast,
            _ => false,
        };
        if !matches_source || broadcast_key != broadcast {
            return Err(ERROR.into());
        }
    }
    Ok(())
}
pub(super) fn segment_sequence(name: &str) -> Option<u64> {
    let digits = name.strip_prefix('e')?.strip_suffix(".mp4")?;
    (digits.len() == 8 && digits.bytes().all(|v| v.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}
pub(super) fn recovery_segments(
    job: &MediaJob,
    directory: &Path,
) -> Result<Vec<(String, f64)>, String> {
    recovery_segments_cancellable(job, directory, &AtomicBool::new(false))
}
fn recovery_segments_cancellable(
    job: &MediaJob,
    directory: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<(String, f64)>, String> {
    validate_proof(job, directory)?;
    let path = directory.join(MANIFEST);
    if storage::regular(&path)?.len() > 32 * 1024 * 1024 {
        return Err(ERROR.into());
    }
    let text = fs::read_to_string(path).map_err(|_| ERROR)?;
    let mut segments = Vec::new();
    for line in text.lines() {
        if cancel.load(Ordering::Acquire) { return Err("cancelled".into()); }
        if segments.len() >= MAX_SEGMENTS {
            return Err(ERROR.into());
        }
        let part: SegmentProof = serde_json::from_str(line).map_err(|_| ERROR)?;
        if segment_sequence(&part.name) != Some(segments.len() as u64)
            || !part.duration.is_finite()
            || part.duration <= 0.0
            || part.duration > 120.0
            || !part.source_start.is_finite()
            || !part.source_end.is_finite()
            || part.source_end <= part.source_start
            || part.bytes == 0
            || part.bytes > MAX_APPEND as u64
        {
            return Err(ERROR.into());
        }
        let path = directory.join(&part.name);
        // Once the final file was published, exact owned pieces may already be
        // cleaned. The verifier must revalidate media.mp4, not recapture video.
        if path.exists() || !directory.join("media.mp4").exists() {
            if storage::regular(&path)?.len() != part.bytes {
                return Err(ERROR.into());
            }
            if storage::hash(&path, cancel)? != part.sha256 {
                return Err(ERROR.into());
            }
        }
        segments.push((part.name, part.duration));
    }
    if segments.is_empty() {
        return Err("복구할 녹화 조각이 없습니다.".into());
    }
    if cancel.load(Ordering::Acquire) { return Err("cancelled".into()); }
    Ok(segments)
}

pub struct EncodedAppend {
    pub source_id: String,
    pub generation: u64,
    pub track_index: u32,
    pub index: u64,
    pub chunk: u32,
    pub final_chunk: bool,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EncodedAck {
    pub track_index: u32,
    pub index: u64,
    pub chunk: u32,
    pub duplicate: bool,
}
#[derive(PartialEq, Eq)]
struct Receipt {
    track: u32,
    index: u64,
    chunk: u32,
    final_chunk: bool,
    digest: [u8; 32],
}
struct PartialAppend {
    track: u32,
    index: u64,
    next_chunk: u32,
    bytes: Vec<u8>,
}
struct Writer {
    lease: OfficialReceiverLease,
    job: MediaJob,
    directory: PathBuf,
    cancel: Arc<AtomicBool>,
    _ownership: lifecycle::JobOwnership,
    source_id: String,
    generation: u64,
    muxer: fmp4::EncodedMuxer,
    youtube_fragments: Option<youtube_fragment::Adapter>,
    next: BTreeMap<u32, u64>,
    partial: Option<PartialAppend>,
    previous: Option<Receipt>,
    segments: Vec<(String, f64)>,
    poisoned: bool,
}
pub struct EncodedArchive {
    writer: Option<Writer>,
}
impl OfficialReceiverLease {
    /// Only the exact-document native capture host may supply these bytes.
    /// Provider, output root and live identity always come from this lease.
    pub fn save_screenshot(
        &self,
        request_id: &str,
        png: &[u8],
        title: &str,
        channel_name: &str,
    ) -> Result<screenshots::Item, String> {
        let _work = self.service.screenshot_work()?;
        let _serial = self.receiver.screenshot_serial.lock().map_err(|_| ERROR)?;
        self.service.inner.screenshots.save_native(
            &self.receiver.screenshot_token,
            request_id,
            screenshots::CaptureContext {
                download_root: self.receiver.download_root.clone(),
                reference: self.receiver.reference.clone(),
                title: title.into(),
                channel_name: channel_name.into(),
            },
            png,
            || {
                if self.is_closed() {
                    Err("종료된 방송 화면은 캡처할 수 없습니다.".into())
                } else {
                    Ok(())
                }
            },
        )
    }
    /// Saved-preview undo remains valid after playback closes, but only for
    /// this lease's private owner and the Store's bounded undo interval.
    pub fn undo_screenshot(&self, id: &str) -> Result<(), String> {
        let _work = self.service.screenshot_work()?;
        let _serial = self.receiver.screenshot_serial.lock().map_err(|_| ERROR)?;
        self.service
            .screenshot_undo(&self.receiver.screenshot_token, id)
    }
    /// Called by the exact-origin native host, never by untrusted IPC. These
    /// public identity scalars are not account credentials or arbitrary URLs.
    pub fn bind_youtube_chat(
        &self,
        video_id: &str,
        channel_id: &str,
        is_live: bool,
        chat_available: Option<bool>,
    ) -> Result<(), String> {
        let reference = &self.receiver.reference;
        if self.is_closed()
            || reference.provider != "youtube"
            || video_id != reference.external_id
            || !is_live
            || channel_id.len() != 24
            || !channel_id.starts_with("UC")
            || !channel_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(ERROR.into());
        }
        let mut binding = self
            .receiver
            .youtube_chat_binding
            .lock()
            .map_err(|_| ERROR)?;
        let state = self.service.inner.state.lock().map_err(|_| ERROR)?;
        if state.shutting_down || self.is_closed() {
            return Err("앱 또는 플레이어를 종료하는 중입니다.".into());
        }
        if let Some(existing) = binding.as_ref() {
            if existing.channel_id != channel_id { return Err(ERROR.into()); }
            // A real attempted producer, including a failed one, is never
            // reset by an account reload or later negative/unknown tick.
            if existing.producer_started { return Ok(()); }
        }
        #[cfg(test)]
        let synthetic = self
            .service
            .inner
            .synthetic_live_receiver
            .load(Ordering::Acquire);
        #[cfg(not(test))]
        let synthetic = false;
        // Mark coverage before publishing a canonical binding eligible for
        // video archive admission. Synthetic-positive test fixtures retain
        // their existing no-chat-worker behavior, never used in production.
        if !synthetic || chat_available != Some(true) {
            self.receiver.chat.observe_youtube_availability(chat_available)?;
        }
        let binding = binding.get_or_insert_with(|| YouTubeChatBinding {
            channel_id: channel_id.into(), producer_started: false,
        });
        if chat_available != Some(true) { return Ok(()); }
        let descriptor = youtube_chat::Descriptor::from_metadata(
            reference,
            &serde_json::json!({
                "id":video_id,"is_live":true,"channel_id":channel_id,
                "subtitles":{"live_chat":[{"video_id":video_id,"protocol":"youtube_live_chat","ext":"json","url":format!("https://www.youtube.com/watch?v={video_id}")}]} 
            }),
        )?.ok_or(ERROR)?;
        binding.producer_started = true;
        if !synthetic {
            self.receiver.chat.start_official_youtube(
                self.service.clone(),
                reference.clone(),
                self.receiver.directory.clone(),
                Ok(Some(descriptor)),
            );
        }
        Ok(())
    }
    pub fn close(&self) {
        self.receiver.close();
    }
    pub fn is_closed(&self) -> bool {
        self.receiver.closed.load(Ordering::Acquire)
    }
    pub fn chat_status(&self) -> String {
        self.receiver.chat.status()
    }
    pub fn youtube_chat_preparing(&self) -> bool {
        self.receiver.reference.provider == "youtube" && self.receiver.chat.is_preparing()
    }
    pub fn receiver_id(&self) -> &str {
        &self.receiver.id
    }
    pub fn begin_encoded(
        &self,
        source_id: String,
        generation: u64,
        request_id: String,
        mut tracks: Vec<EncodedTrack>,
        title: String,
        channel_name: String,
        owner: live::RecordingOwner,
    ) -> Result<EncodedArchive, String> {
        if self.is_closed()
            || !uuid(&source_id)
            || generation == 0
            || !uuid(&request_id)
            || tracks.is_empty()
            || tracks.len() > 2
            || tracks.iter().any(|t| {
                t.track_index > 1
                    || t.mime_type.len() > 120
                    || t.init.is_empty()
                    || t.init.len() > 65536
            })
        {
            return Err(ERROR.into());
        }
        if self.receiver.reference.provider == "youtube" {
            let binding = self.receiver.youtube_chat_binding.lock().map_err(|_| ERROR)?;
            let channel = &binding.as_ref().ok_or("YouTube 원본 플레이어의 채팅 보존 경로를 아직 확인하지 못했습니다. 녹화를 시작하지 않았습니다.")?.channel_id;
            if let live::RecordingOwner::Scheduled { schedule_id, .. } = &owner {
                if schedule_id != channel { return Err("예약한 채널과 실제 방송 채널이 다릅니다. 녹화를 시작하지 않았습니다.".into()); }
            }
        }
        validate_owner(
            &owner,
            &self.receiver.reference,
            &self.receiver.broadcast_id,
        )?;
        let next: BTreeMap<_, _> = tracks.iter().map(|t| (t.track_index, 0)).collect();
        if next.len() != tracks.len() {
            return Err(ERROR.into());
        }
        youtube_aac::normalize(&self.receiver.reference.provider, &mut tracks);
        let muxer = fmp4::EncodedMuxer::new(tracks.clone()).map_err(|e| e.message)?;
        // Admission above proves the single stsd/trex description index used
        // by the provider-local header adapter. Other providers bypass it.
        let youtube_fragments = youtube_fragment::Adapter::for_validated_inputs(
            &self.receiver.reference.provider,
            &tracks,
        );
        let mut recording = self.receiver.recording.lock().map_err(|_| ERROR)?;
        if recording.is_some() {
            return Err("이미 녹화 중이거나 마무리 중입니다.".into());
        }
        let mut state = self.service.inner.state.lock().map_err(|_| ERROR)?;
        if state.shutting_down || self.is_closed() {
            return Err("앱 또는 플레이어를 종료하는 중입니다.".into());
        }
        if state.jobs.values().any(|job| job.request_id == request_id) {
            return Err("이미 처리한 녹화 요청입니다.".into());
        }
        if state.jobs.values().filter(|job| job.active()).count() >= queries::ACTIVE_JOB_LIMIT {
            return Err("진행 중인 작업이 많습니다.".into());
        }
        let job = MediaJob {
            broadcast_id: Some(self.receiver.broadcast_id.clone()),
            job_id: uuid::Uuid::new_v4().to_string(),
            request_id,
            reference: self.receiver.reference.clone(),
            revision: 1,
            state: "recording".into(),
            title: clean(Some(&title), 500),
            channel_name: clean(Some(&channel_name), 200),
            created_at: now(),
            received_bytes: 0,
            total_bytes: None,
            error: None,
            library: None,
            download_root: self.receiver.download_root.clone(),
        };
        let directory = storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id)?;
        fs::create_dir_all(&directory).map_err(|_| ERROR)?;
        write_new_json(
            &directory.join(PROOF),
            &Proof {
                version: 1,
                reference: job.reference.clone(),
                broadcast_id: self.receiver.broadcast_id.clone(),
                receiver_id: self.receiver.id.clone(),
                source_id: source_id.clone(),
                generation,
                request_id: job.request_id.clone(),
                job_id: job.job_id.clone(),
                owner: owner.clone(),
            },
        )?;
        write_new_json(&directory.join("recording-owner.json"), &owner)?;
        self.receiver.chat.attach_recording_archive(
            &directory,
            &job.reference.provider,
            &job.job_id,
        )?;
        if let Err(error) = storage::save_job(&self.service.inner.journal, &job) {
            let _ = self.receiver.chat.detach_archive();
            return Err(error);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state.cancel.insert(job.job_id.clone(), cancel.clone());
        state.jobs.insert(job.job_id.clone(), job.clone());
        *recording = Some(job.job_id.clone());
        drop(state);
        drop(recording);
        self.service.schedule_profile(&job, &cancel, Some(ProfileReceiver::new(self, owner)));
        Ok(EncodedArchive {
            writer: Some(Writer {
                lease: self.clone(),
                _ownership: lifecycle::JobOwnership::new(
                    self.service.clone(),
                    job.job_id.clone(),
                    cancel.clone(),
                ),
                job,
                directory,
                cancel,
                source_id,
                generation,
                muxer,
                youtube_fragments,
                next,
                partial: None,
                previous: None,
                segments: Vec::new(),
                poisoned: false,
            }),
        })
    }
}
impl Writer {
    fn commit(&mut self, segments: Vec<fmp4::EncodedSegment>) -> Result<(), String> {
        for segment in segments {
            if self.cancel.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            if self.segments.len() >= MAX_SEGMENTS
                || segment.bytes.is_empty()
                || segment.bytes.len() > MAX_APPEND
                || !segment.duration_seconds.is_finite()
                || segment.duration_seconds <= 0.0
                || segment.duration_seconds > 120.0
            {
                return Err(ERROR.into());
            }
            if fs2::available_space(&self.directory).map_err(|_| ERROR)? < 1024 * 1024 * 1024 {
                return Err("저장 공간이 부족합니다. 녹화 조각은 보존했습니다.".into());
            }
            let name = format!("e{:08}.mp4", self.segments.len());
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(self.directory.join(&name))
                .map_err(|_| ERROR)?;
            file.write_all(&segment.bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| ERROR)?;
            let part = SegmentProof {
                name: name.clone(),
                duration: segment.duration_seconds,
                source_start: segment.source_start_seconds,
                source_end: segment.source_end_seconds,
                bytes: segment.bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&segment.bytes)),
            };
            let path = self.directory.join(MANIFEST);
            if path.exists() {
                storage::regular(&path)?;
            }
            let mut manifest = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|_| ERROR)?;
            serde_json::to_writer(&mut manifest, &part).map_err(|_| ERROR)?;
            manifest
                .write_all(b"\n")
                .and_then(|_| manifest.sync_all())
                .map_err(|_| ERROR)?;
            if self.segments.is_empty() {
                let committed_at_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
                let duration_ms = (segment.duration_seconds * 1000.0).round() as u64;
                write_new_json(
                    &self.directory.join("chat-timeline.json"),
                    &serde_json::json!({"version":1,"provider":self.job.reference.provider,"firstSegmentCommittedAtMs":committed_at_ms,"firstSegmentDurationMs":duration_ms,"recordingRequestedAtMs":self.job.created_at*1000,"timelineQuality":"receive-time-approximate"}),
                )?;
                if self.job.reference.provider == "youtube" {
                    self.lease.receiver.chat.resolve_youtube_archive_origin(
                        &self.job.job_id,
                        u64::try_from(committed_at_ms).ok()
                            .filter(|time| (1_000_000_000_000..=9_000_000_000_000).contains(time)
                                && (1..=120_000).contains(&duration_ms))
                            .and_then(|time| time.checked_sub(duration_ms)),
                    )?;
                }
            }
            self.segments.push((name, segment.duration_seconds));
            self.lease.service.update(&self.job.job_id, |job| {
                job.received_bytes = job
                    .received_bytes
                    .saturating_add(segment.bytes.len() as u64)
            })?;
        }
        Ok(())
    }
    fn append(&mut self, packet: EncodedAppend) -> Result<EncodedAck, String> {
        if self.poisoned
            || self.cancel.load(Ordering::Acquire)
            || self.lease.is_closed()
            || packet.source_id != self.source_id
            || packet.generation != self.generation
            || packet.bytes.is_empty()
            || packet.bytes.len() > MAX_CHUNK
            || packet.index >= 1_000_000
        {
            return Err(ERROR.into());
        }
        let receipt = Receipt {
            track: packet.track_index,
            index: packet.index,
            chunk: packet.chunk,
            final_chunk: packet.final_chunk,
            digest: Sha256::digest(&packet.bytes).into(),
        };
        let ack = |duplicate| EncodedAck {
            track_index: packet.track_index,
            index: packet.index,
            chunk: packet.chunk,
            duplicate,
        };
        if self.previous.as_ref() == Some(&receipt) {
            return Ok(ack(true));
        }
        if self.next.get(&packet.track_index) != Some(&packet.index) {
            return Err(ERROR.into());
        }
        if self.partial.is_none() {
            if packet.chunk != 0 {
                return Err(ERROR.into());
            }
            self.partial = Some(PartialAppend {
                track: packet.track_index,
                index: packet.index,
                next_chunk: 0,
                bytes: Vec::new(),
            });
        }
        let partial = self.partial.as_mut().ok_or(ERROR)?;
        if partial.track != packet.track_index
            || partial.index != packet.index
            || partial.next_chunk != packet.chunk
            || partial.bytes.len().saturating_add(packet.bytes.len()) > MAX_APPEND
        {
            return Err(ERROR.into());
        }
        partial.bytes.extend_from_slice(&packet.bytes);
        partial.next_chunk = partial.next_chunk.checked_add(1).ok_or(ERROR)?;
        if packet.final_chunk {
            let partial = self.partial.take().ok_or(ERROR)?;
            let segments = if let Some(adapter) = self.youtube_fragments.as_mut() {
                let muxer = &mut self.muxer;
                let mut segments = Vec::new();
                adapter.push(packet.track_index, &partial.bytes, |bytes| {
                    segments.extend(
                        muxer
                            .push(packet.track_index, bytes)
                            .map_err(|e| e.message)?,
                    );
                    Ok(())
                })?;
                segments
            } else {
                self.muxer
                    .push(packet.track_index, &partial.bytes)
                    .map_err(|e| e.message)?
            };
            self.commit(segments)?;
            self.next.insert(packet.track_index, packet.index + 1);
        }
        self.previous = Some(receipt);
        Ok(ack(false))
    }
}
impl EncodedArchive {
    pub fn is_cancelled(&self) -> bool {
        self.writer
            .as_ref()
            .is_none_or(|writer| writer.cancel.load(Ordering::Acquire))
    }
    pub fn snapshot(&self) -> Result<MediaJob, String> {
        let writer = self.writer.as_ref().ok_or(ERROR)?;
        writer
            .lease
            .service
            .inner
            .state
            .lock()
            .map_err(|_| ERROR)?
            .jobs
            .get(&writer.job.job_id)
            .cloned()
            .ok_or_else(|| ERROR.into())
    }
    pub fn append(&mut self, packet: EncodedAppend) -> Result<EncodedAck, String> {
        let writer = self.writer.as_mut().ok_or(ERROR)?;
        let result = writer.append(packet);
        if result.is_err() {
            writer.poisoned = true;
        }
        result
    }
    /// Transfer the existing job and cancel owner into a local-only finalizer.
    /// Returned state is finalizing, not an assertion that a file is complete.
    pub fn finish(mut self, incoming_error: Option<String>) -> Result<MediaJob, String> {
        let mut writer = self.writer.take().ok_or(ERROR)?;
        if writer.job.reference.provider == "soop" && writer.job.reference.kind == "live" {
            let error = soop_finalize::finish(&mut writer, incoming_error.is_some()).err();
            // Always detach and persist this job's chat, including proof/commit
            // failures. Only a durable stop proof permits salvaged publication.
            return dispatch(writer, error);
        }
        let mut error = incoming_error
            .map(|_| "원본 플레이어의 수신이 중단됐습니다. 녹화 조각은 보존했습니다.".to_string());
        if writer.poisoned
            || writer.partial.is_some()
            || writer
                .youtube_fragments
                .as_ref()
                .is_some_and(|adapter| adapter.incomplete())
        {
            error.get_or_insert(ERROR.into());
        }
        if !writer.poisoned {
            let tail = if error.is_some() {
                writer.muxer.finish_discontinuity()
            } else {
                writer.muxer.finish()
            };
            match tail
                .map_err(|e| e.message)
                .and_then(|parts| writer.commit(parts))
            {
                Ok(()) => (),
                Err(e) => {
                    error.get_or_insert(e);
                }
            }
        }
        dispatch(writer, error)
    }
    /// Close a confirmed YouTube format boundary without mixing decoder configs.
    /// The caller must revoke/replace the exact recording arm under its owner gate.
    /// `true` means durable complete-segment progress, NOT a verified final file.
    /// Existing ordinary/error finish paths intentionally keep their stricter rules.
    pub(crate) fn finish_format_change(mut self) -> Result<(MediaJob, bool), String> {
        let mut writer = self.writer.take().ok_or(ERROR)?;
        let mut fatal = writer.job.reference.provider != "youtube"
            || writer.poisoned
            || writer.partial.is_some()
            || writer.cancel.load(Ordering::Acquire);
        let mut error = fatal.then(|| ERROR.to_owned());
        if writer.job.reference.provider == "youtube" && !writer.poisoned {
            // A format boundary can abandon an MSE box. The adapter never sent
            // its unparsed moof/header onward; the muxer drops only its own
            // incomplete fragment and flushes previously validated A/V samples.
            match writer.muxer.finish_discontinuity() {
                Ok(parts) => if let Err(cause) = writer.commit(parts) {
                    fatal = true;
                    error.get_or_insert(cause);
                },
                // This exact parser branch means no valid A/V start existed,
                // not malformed input. Any changed/unknown parser error stays
                // fatal. A regression test binds this narrow classification.
                Err(cause) if !fatal && writer.segments.is_empty()
                    && cause.code == "ENCODED_UNSUPPORTED"
                    && cause.message == "완전한 영상·음성 시작 구간을 받지 못했습니다." => {
                    error = Some(cause.message);
                }
                Err(cause) => {
                    fatal = true;
                    error.get_or_insert(cause.message);
                }
            }
        }
        if writer.cancel.load(Ordering::Acquire) {
            fatal = true;
            error.get_or_insert_with(|| ERROR.to_owned());
        }
        if writer.segments.is_empty() {
            error.get_or_insert_with(|| "형식 변경 전에 완전한 영상·음성 구간을 받지 못했습니다. 원본은 보존했습니다.".into());
        }
        let progressed = !fatal && error.is_none() && !writer.segments.is_empty();
        // Normal final-media and chat verification still owns publication and
        // cleanup. A failed tool/proof never becomes success at this boundary.
        let fatal_message = fatal.then(|| error.clone().unwrap_or_else(|| ERROR.to_owned()));
        let job = dispatch(writer, error)?;
        match fatal_message {
            Some(message) => Err(message),
            None => Ok((job, progressed)),
        }
    }
    /// Carries the old attempt's cancellation fence into a pending successor.
    /// It grants no new ownership and is never shared across provider sessions.
    pub(crate) fn format_change_cancel_guard(&self) -> Option<Arc<AtomicBool>> {
        let writer = self.writer.as_ref()?;
        (writer.job.reference.provider == "youtube" && !writer.poisoned
            && writer.partial.is_none() && !writer.cancel.load(Ordering::Acquire))
            .then(|| writer.cancel.clone())
    }
}
impl Drop for EncodedArchive {
    fn drop(&mut self) {
        if let Some(writer) = self.writer.take() {
            let _ = dispatch(
                writer,
                Some("녹화가 중단됐습니다. 원본 조각은 보존했습니다.".into()),
            );
        }
    }
}
fn dispatch(writer: Writer, mut incoming_error: Option<String>) -> Result<MediaJob, String> {
    let service = writer.lease.service.clone();
    let id = writer.job.job_id.clone();
    // Detach under the chat lock before allowing the next archive. A late chat
    // callback cannot cross into the following recording's files.
    let chat = writer.lease.receiver.chat.detach_archive();
    if let Ok(mut active) = writer.lease.receiver.recording.lock() {
        if active.as_ref() == Some(&id) {
            *active = None;
        }
    }
    let snapshot = if writer.job.reference.provider == "soop" {
        match soop_finalize::start(&service, &writer.job, &writer.cancel) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                incoming_error.get_or_insert(error);
                // Keep detachment/persistence ownership even after revocation,
                // but never demote a completed or replacement job's state.
                service.inner.state.lock().map_err(|_| ERROR)?.jobs.get(&id)
                    .cloned().unwrap_or_else(|| writer.job.clone())
            }
        }
    } else {
      service.update(&id, |job| job.state = "finalizing".into())?;
      service
        .inner
        .state
        .lock()
        .map_err(|_| ERROR)?
        .jobs
        .get(&id)
        .cloned()
        .ok_or(ERROR)?
    };
    std::thread::Builder::new()
        .name("platform-encoded-finalize".into())
        .spawn(move || {
            // Move the whole Writer, not individual used fields: its RAII
            // ownership must stay alive until this worker actually returns.
            let writer = writer;
            let result = (|| {
                let detached =
                    chat?.ok_or("채팅 기록을 마무리하지 못했습니다. 원본은 보존했습니다.")?;
                let _status = detached.persist_cancellable(
                    &writer.directory,
                    &writer.job.reference.provider,
                    &id,
                    &writer.cancel,
                )?;
                if let Some(error) = incoming_error {
                    return Err(error);
                }
                // A partial chat proof must not hide a valid playable video.
                // Publication retains that explicit status and all originals;
                // it never upgrades partial/unknown chat to complete.
                service.finalize_encoded_recording(
                    &writer.job,
                    writer.directory.clone(),
                    writer.cancel.clone(),
                    writer.segments.clone(),
                )
            })();
            if let Err(error) = result {
                service.chat_aware_terminal_error(&id, &writer.cancel, &error);
            }
            // Writer's RAII owner releases only this attempt's cancel token.
        })
        .map_err(|_| "녹화 마무리 작업을 시작하지 못했습니다. 원본은 보존했습니다.")?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        service: PlatformMediaService,
        root: tempfile::TempDir,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let missing = root.path().join("no-network-or-media-tool.exe");
            let service = PlatformMediaService::new(
                root.path().join("data"),
                PlatformMediaTools {
                    yt_dlp: missing.clone(),
                    deno: missing.clone(),
                    ffmpeg: missing.clone(),
                    ffprobe: missing,
                },
                None,
            )
            .unwrap();
            service
                .inner
                .synthetic_live_receiver
                .store(true, Ordering::Release);
            Self { service, root }
        }
        fn media(&self) -> PathBuf {
            self.root.path().join("media")
        }
        fn lease(&self, channel: &str) -> OfficialReceiverLease {
            self.service
                .reserve_official_receiver(
                    reference(channel),
                    "123".into(),
                    uuid::Uuid::new_v4().to_string(),
                    self.media(),
                )
                .unwrap()
        }
        fn settled(&self, id: &str) -> MediaJob {
            let start = std::time::Instant::now();
            loop {
                let state = self.service.inner.state.lock().unwrap();
                let job = state.jobs[id].clone();
                if !job.active() && !state.cancel.contains_key(id) {
                    return job;
                }
                drop(state);
                assert!(
                    start.elapsed() < Duration::from_secs(5),
                    "isolated finalizer did not drain"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.service.shutdown();
        }
    }
    fn reference(channel: &str) -> MediaRef {
        MediaRef {
            provider: "soop".into(),
            kind: "live".into(),
            external_id: channel.into(),
        }
    }
    fn archive(lease: &OfficialReceiverLease) -> EncodedArchive {
        lease
            .begin_encoded(
                uuid::Uuid::new_v4().to_string(),
                1,
                uuid::Uuid::new_v4().to_string(),
                fmp4::fixtures::inputs(),
                "fixture".into(),
                "fixture".into(),
                live::RecordingOwner::Manual,
            )
            .unwrap()
    }
    fn packet(archive: &EncodedArchive, track: u32, index: u64, bytes: Vec<u8>) -> EncodedAppend {
        EncodedAppend {
            source_id: archive.writer.as_ref().unwrap().source_id.clone(),
            generation: 1,
            track_index: track,
            index,
            chunk: 0,
            final_chunk: true,
            bytes,
        }
    }
    #[test]
    fn profile_receiver_is_weak_and_rejects_closed_or_replaced_recording() {
        let fixture = Fixture::new();
        let lease = fixture.lease("portrait");
        let record = archive(&lease);
        let job = record.writer.as_ref().unwrap().job.clone();
        let leases = lease.receiver.leases.load(Ordering::Acquire);
        let capability = ProfileReceiver::new(&lease, live::RecordingOwner::Manual);
        assert_eq!(lease.receiver.leases.load(Ordering::Acquire), leases);
        assert_eq!(capability.with_current(&job, || Some(7)), Some(7));
        *lease.receiver.recording.lock().unwrap() = Some(uuid::Uuid::new_v4().to_string());
        assert_eq!(capability.with_current(&job, || Some(7)), None);
        *lease.receiver.recording.lock().unwrap() = Some(job.job_id.clone());
        lease.close();
        assert_eq!(capability.with_current(&job, || Some(7)), None);
    }
    #[test]
    fn profile_receiver_rechecks_immutable_native_owner() {
        let fixture = Fixture::new();
        let lease = fixture.lease("portrait");
        let record = archive(&lease);
        let job = record.writer.as_ref().unwrap().job.clone();
        let capability = ProfileReceiver::new(&lease, live::RecordingOwner::Manual);
        let directory = storage::job_dir(&job.download_root, "soop", &job.job_id).unwrap();
        let mut proof = read_proof(&job, &directory).unwrap();
        proof.owner = live::RecordingOwner::Scheduled {
            schedule_id: "portrait".into(), broadcast_key: "123".into(),
            run_id: uuid::Uuid::new_v4().to_string(), generation: 1,
        };
        std::fs::write(directory.join(PROOF), serde_json::to_vec(&proof).unwrap()).unwrap();
        assert_eq!(capability.with_current(&job, || Some(7)), None);
    }
    #[test]
    fn profile_commit_and_receiver_close_share_one_linearization_gate() {
        let fixture = Fixture::new();
        let lease = fixture.lease("portrait");
        let record = archive(&lease);
        let job = record.writer.as_ref().unwrap().job.clone();
        let capability = ProfileReceiver::new(&lease, live::RecordingOwner::Manual);
        std::thread::scope(|scope| {
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let capability = &capability;
            let job = &job;
            let commit = scope.spawn(move || capability.with_current(job, || {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                Some(())
            }));
            entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            let close = scope.spawn(|| lease.close());
            assert!(!lease.is_closed());
            release_tx.send(()).unwrap();
            assert_eq!(commit.join().unwrap(), Some(()));
            close.join().unwrap();
        });
        assert!(lease.is_closed());
        assert_eq!(capability.with_current(&job, || Some(())), None);
    }
    #[test]
    fn admission_binds_provider_receiver_broadcast_and_combined_capacity() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        let same = fixture
            .service
            .reserve_official_receiver(
                reference("one"),
                "123".into(),
                lease.receiver_id().into(),
                fixture.media(),
            )
            .unwrap();
        assert_eq!(lease.receiver_id(), same.receiver_id());
        for (provider, channel, broadcast) in [
            ("chzzk", "one", "123"),
            ("soop", "one", "0123"),
            ("youtube", "abcdefghijk", "lmnopqrstuv"),
        ] {
            assert!(fixture
                .service
                .reserve_official_receiver(
                    MediaRef {
                        provider: provider.into(),
                        kind: "live".into(),
                        external_id: channel.into()
                    },
                    broadcast.into(),
                    uuid::Uuid::new_v4().to_string(),
                    fixture.media()
                )
                .is_err());
        }
        assert!(fixture
            .service
            .reserve_official_receiver(
                reference("one"),
                "124".into(),
                lease.receiver_id().into(),
                fixture.media()
            )
            .is_err());
        assert!(fixture
            .service
            .reserve_official_receiver(
                reference("other"),
                "123".into(),
                lease.receiver_id().into(),
                fixture.media()
            )
            .is_err());
        assert!(fixture
            .service
            .live_open(
                reference("one"),
                uuid::Uuid::new_v4().to_string(),
                fixture.media()
            )
            .is_err());
        assert!(fixture
            .service
            .scheduled_start(
                live::RecordingOwner::Scheduled {
                    schedule_id: "one".into(),
                    run_id: uuid::Uuid::new_v4().to_string(),
                    broadcast_key: "123".into(),
                    generation: 1
                },
                fixture.media()
            )
            .is_err());
        let other: Vec<_> = ["two", "three", "four"]
            .iter()
            .map(|channel| fixture.lease(channel))
            .collect();
        assert!(fixture
            .service
            .reserve_official_receiver(
                reference("five"),
                "123".into(),
                uuid::Uuid::new_v4().to_string(),
                fixture.media()
            )
            .is_err());
        assert!(fixture
            .service
            .live_open(
                reference("five"),
                uuid::Uuid::new_v4().to_string(),
                fixture.media()
            )
            .is_err());
        assert_eq!(fixture.service.active_ids().len(), 4);
        drop(other);
        drop(same);
        drop(lease);
        assert!(fixture.service.active_ids().is_empty());
    }
    #[test]
    fn native_screenshot_uses_private_owner_png_dimensions_and_existing_library() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        let other = fixture.lease("two");
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(3, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let png = bytes.into_inner();
        let request = uuid::Uuid::new_v4().to_string();
        let item = lease
            .save_screenshot(&request, &png, "방송", "채널")
            .unwrap();
        assert_eq!((item.width, item.height), (3, 2));
        assert_eq!(item.reference, reference("one"));
        assert_eq!(item.provider, "soop");
        assert_eq!(
            fixture
                .service
                .screenshot_list("soop", None, 24)
                .unwrap()
                .items
                .len(),
            1
        );
        assert!(fixture
            .service
            .screenshot_list("youtube", None, 24)
            .unwrap()
            .items
            .is_empty());
        assert_eq!(
            lease
                .save_screenshot(&request, &png, "방송", "채널")
                .unwrap()
                .id,
            item.id
        );
        assert!(other.undo_screenshot(&item.id).is_err());
        assert!(fixture
            .service
            .screenshot_undo(lease.receiver_id(), &item.id)
            .is_err());
        assert_ne!(lease.receiver.screenshot_token, lease.receiver.id);
        assert!(lease
            .save_screenshot(&uuid::Uuid::new_v4().to_string(), b"not png", "", "")
            .is_err());
        lease.close();
        assert!(lease
            .save_screenshot(&uuid::Uuid::new_v4().to_string(), &png, "", "")
            .is_err());
        lease.undo_screenshot(&item.id).unwrap();
        assert!(fixture
            .service
            .screenshot_list("soop", None, 24)
            .unwrap()
            .items
            .is_empty());
    }
    #[test]
    fn legacy_receiver_blocks_official_admission_without_second_extractor() {
        let fixture = Fixture::new();
        let live = fixture
            .service
            .live_open(
                reference("one"),
                uuid::Uuid::new_v4().to_string(),
                fixture.media(),
            )
            .unwrap();
        assert!(fixture
            .service
            .reserve_official_receiver(
                reference("one"),
                "123".into(),
                uuid::Uuid::new_v4().to_string(),
                fixture.media()
            )
            .is_err());
        fixture.service.live_close(&live.token).unwrap();
    }
    #[test]
    fn youtube_unavailable_does_not_consume_the_once_only_chat_producer() {
        let fixture = Fixture::new();
        // The fixture has no executable media/network tools. This observation
        // must not start any worker at all, rather than start one that fails.
        fixture.service.inner.synthetic_live_receiver.store(false, Ordering::Release);
        let lease = fixture.service.reserve_official_receiver(
            MediaRef { provider: "youtube".into(), kind: "live".into(), external_id: "abcdefghijk".into() },
            "abcdefghijk".into(), uuid::Uuid::new_v4().to_string(), fixture.media(),
        ).unwrap();
        lease.bind_youtube_chat("abcdefghijk", "UCabcdefghijklmnopqrstuv", true, Some(false)).unwrap();
        assert!(!lease.receiver.chat.producer_started_for_test(), "unavailable is an observation, not a terminal producer attempt");
    }
    fn observation_lease(fixture: &Fixture) -> OfficialReceiverLease {
        fixture.service.inner.synthetic_live_receiver.store(false, Ordering::Release);
        fixture.service.reserve_official_receiver(
            MediaRef { provider: "youtube".into(), kind: "live".into(), external_id: "abcdefghijk".into() },
            "abcdefghijk".into(), uuid::Uuid::new_v4().to_string(), fixture.media(),
        ).unwrap()
    }
    #[test]
    fn youtube_unknown_and_disabled_upgrade_once_without_retrying_a_failed_producer() {
        for initial in [None, Some(false)] {
            let fixture = Fixture::new(); let lease = observation_lease(&fixture);
            let channel = "UCabcdefghijklmnopqrstuv";
            lease.bind_youtube_chat("abcdefghijk", channel, true, initial).unwrap();
            assert!(!lease.receiver.chat.producer_started_for_test());
            assert!(!lease.receiver.youtube_chat_binding.lock().unwrap().as_ref().unwrap().producer_started);
            std::thread::scope(|scope| {
                for _ in 0..8 { scope.spawn(|| lease.bind_youtube_chat("abcdefghijk", channel, true, Some(true)).unwrap()); }
            });
            assert!(lease.receiver.chat.producer_started_for_test());
            assert!(lease.receiver.youtube_chat_binding.lock().unwrap().as_ref().unwrap().producer_started);
            // The nonexistent fixture executable fails locally, without any
            // network request. Later observations cannot reset this attempt.
            let until = std::time::Instant::now() + Duration::from_secs(3);
            while !lease.receiver.chat.is_done() {
                assert!(std::time::Instant::now() < until); std::thread::sleep(Duration::from_millis(5));
            }
            let failed_status = lease.chat_status();
            for value in [None, Some(false), Some(true)] {
                lease.bind_youtube_chat("abcdefghijk", channel, true, value).unwrap();
                assert_eq!(lease.chat_status(), failed_status);
                assert!(lease.receiver.chat.is_done());
            }
        }
    }
    #[test]
    fn youtube_close_and_shutdown_cannot_publish_a_late_binding() {
        for shutdown in [false, true] {
            let fixture = Fixture::new(); let lease = observation_lease(&fixture);
            if shutdown { fixture.service.inner.state.lock().unwrap().shutting_down = true; }
            else { lease.close(); }
            for value in [None, Some(false), Some(true)] {
                assert!(lease.bind_youtube_chat("abcdefghijk", "UCabcdefghijklmnopqrstuv", true, value).is_err());
            }
            assert!(lease.receiver.youtube_chat_binding.lock().unwrap().is_none());
            assert!(!lease.receiver.chat.producer_started_for_test());
        }
    }
    #[test]
    fn youtube_close_serializes_with_unknown_binding_without_reopening_receiver() {
        for _ in 0..8 {
            let fixture = Fixture::new(); let lease = observation_lease(&fixture);
            let barrier = std::sync::Barrier::new(2);
            std::thread::scope(|scope| {
                scope.spawn(|| { barrier.wait(); lease.close(); });
                barrier.wait();
                let _ = lease.bind_youtube_chat("abcdefghijk", "UCabcdefghijklmnopqrstuv", true, None);
            });
            assert!(lease.is_closed());
            assert!(lease.bind_youtube_chat("abcdefghijk", "UCabcdefghijklmnopqrstuv", true, Some(true)).is_err());
            assert!(!lease.receiver.chat.producer_started_for_test());
        }
    }
    #[test]
    fn youtube_public_chat_binding_is_exact_idempotent_and_not_account_input() {
        let fixture = Fixture::new();
        let reference = MediaRef {
            provider: "youtube".into(),
            kind: "live".into(),
            external_id: "abcdefghijk".into(),
        };
        let lease = fixture
            .service
            .reserve_official_receiver(
                reference,
                "abcdefghijk".into(),
                uuid::Uuid::new_v4().to_string(),
                fixture.media(),
            )
            .unwrap();
        let begin = || {
            lease.begin_encoded(
                uuid::Uuid::new_v4().to_string(),
                1,
                uuid::Uuid::new_v4().to_string(),
                fmp4::fixtures::inputs(),
                String::new(),
                String::new(),
                live::RecordingOwner::Manual,
            )
        };
        assert!(begin().is_err());
        let channel = "UCabcdefghijklmnopqrstuv";
        assert!(lease
            .bind_youtube_chat("wrongwrongw", channel, true, Some(true))
            .is_err());
        assert!(lease
            .bind_youtube_chat("abcdefghijk", channel, false, Some(true))
            .is_err());
        assert!(lease
            .bind_youtube_chat("abcdefghijk", "https://evil.invalid", true, Some(true))
            .is_err());
        lease
            .bind_youtube_chat("abcdefghijk", channel, true, Some(true))
            .unwrap();
        lease
            .bind_youtube_chat("abcdefghijk", channel, true, Some(true))
            .unwrap();
        assert!(lease
            .bind_youtube_chat("abcdefghijk", "UC0000000000000000000000", true, Some(true))
            .is_err());
        let record = begin().unwrap();
        let id = record.snapshot().unwrap().job_id;
        drop(record);
        fixture.settled(&id);
    }
    #[test]
    fn youtube_scheduled_archive_rejects_another_bound_channel_before_job_creation() {
        let fixture = Fixture::new();
        let reference = MediaRef { provider: "youtube".into(), kind: "live".into(), external_id: "abcdefghijk".into() };
        let lease = fixture.service.reserve_official_receiver(reference, "abcdefghijk".into(), uuid::Uuid::new_v4().to_string(), fixture.media()).unwrap();
        let channel = "UCabcdefghijklmnopqrstuv";
        lease.bind_youtube_chat("abcdefghijk", channel, true, Some(true)).unwrap();
        let begin = |schedule_id: &str| lease.begin_encoded(
            uuid::Uuid::new_v4().to_string(), 1, uuid::Uuid::new_v4().to_string(),
            fmp4::fixtures::inputs(), String::new(), String::new(), live::RecordingOwner::Scheduled {
                schedule_id: schedule_id.into(), run_id: uuid::Uuid::new_v4().to_string(), broadcast_key: "abcdefghijk".into(), generation: 1,
            });
        assert!(begin("UC0000000000000000000000").is_err());
        assert!(fixture.service.jobs(Some("youtube")).is_empty());
        let record = begin(channel).unwrap();
        let id = record.snapshot().unwrap().job_id;
        drop(record);
        fixture.settled(&id);
    }
    fn youtube_archive(fixture: &Fixture) -> EncodedArchive {
        let reference = MediaRef {
            provider: "youtube".into(),
            kind: "live".into(),
            external_id: "abcdefghijk".into(),
        };
        let lease = fixture
            .service
            .reserve_official_receiver(
                reference,
                "abcdefghijk".into(),
                uuid::Uuid::new_v4().to_string(),
                fixture.media(),
            )
            .unwrap();
        lease
            .bind_youtube_chat("abcdefghijk", "UCabcdefghijklmnopqrstuv", true, Some(true))
            .unwrap();
        archive(&lease)
    }
    #[test]
    fn youtube_adapter_keeps_transport_duplicates_out_of_stream_state() {
        let fixture = Fixture::new();
        let mut record = youtube_archive(&fixture);
        assert!(record.writer.as_ref().unwrap().youtube_fragments.is_some());
        for track in 0..2 {
            let bytes = youtube_fragment::explicit_description(&fmp4::fixtures::fragment(
                track, 0, 4, true,
            ));
            // A logical MSE append ends inside a box header. Retrying its exact
            // transport chunk must not feed those same three bytes twice.
            let first = packet(&record, track, 0, bytes[..3].to_vec());
            record.append(first).unwrap();
            assert!(
                record
                    .append(packet(&record, track, 0, bytes[..3].to_vec()))
                    .unwrap()
                    .duplicate
            );
            let mut next = packet(&record, track, 1, bytes[3..9].to_vec());
            next.final_chunk = false;
            record.append(next).unwrap();
            let mut last = packet(&record, track, 1, bytes[9..].to_vec());
            last.chunk = 1;
            record.append(last).unwrap();
        }
        let writer = record.writer.as_mut().unwrap();
        assert!(!writer.youtube_fragments.as_ref().unwrap().incomplete());
        let tail = writer.muxer.finish().unwrap();
        writer.commit(tail).unwrap();
        assert_eq!(writer.segments.len(), 1);
        let id = record.snapshot().unwrap().job_id;
        assert!(record.snapshot().unwrap().received_bytes > 0);
        drop(record);
        fixture.settled(&id);
        let soop = fixture.lease("one");
        let archive = archive(&soop);
        assert!(archive.writer.as_ref().unwrap().youtube_fragments.is_none());
        let id = archive.snapshot().unwrap().job_id;
        drop(archive);
        fixture.settled(&id);
    }
    #[test]
    fn youtube_withheld_header_or_moof_marks_finalizer_partial_and_preserves_valid_tail() {
        for trailing in [3usize, 13] {
            let fixture = Fixture::new();
            let mut record = youtube_archive(&fixture);
            for track in 0..2 {
                let bytes = youtube_fragment::explicit_description(&fmp4::fixtures::fragment(
                    track, 0, 4, true,
                ));
                record.append(packet(&record, track, 0, bytes)).unwrap();
            }
            let next = youtube_fragment::explicit_description(&fmp4::fixtures::fragment(
                0, 180_000, 4, true,
            ));
            record
                .append(packet(&record, 0, 1, next[..trailing].to_vec()))
                .unwrap();
            let id = record.snapshot().unwrap().job_id;
            let directory = record.writer.as_ref().unwrap().directory.clone();
            record.finish(None).unwrap();
            let job = fixture.settled(&id);
            assert_eq!(job.state, "failed");
            assert_eq!(job.error.as_deref(), Some(ERROR));
            assert_eq!(recovery_segments(&job, &directory).unwrap().len(), 1);
            assert!(job.library.is_none());
            assert!(directory.join("e00000000.mp4").is_file());
        }
    }
    #[test]
    fn youtube_format_boundary_flushes_valid_tail_but_still_requires_final_verification() {
        for trailing in [0usize, 3, 13] {
            let fixture = Fixture::new();
            let mut record = youtube_archive(&fixture);
            for track in 0..2 {
                let bytes = youtube_fragment::explicit_description(&fmp4::fixtures::fragment(track, 0, 4, true));
                record.append(packet(&record, track, 0, bytes)).unwrap();
            }
            if trailing > 0 {
                let bytes = youtube_fragment::explicit_description(&fmp4::fixtures::fragment(0, 180_000, 4, true));
                record.append(packet(&record, 0, 1, bytes[..trailing].to_vec())).unwrap();
            }
            let id = record.snapshot().unwrap().job_id;
            let directory = record.writer.as_ref().unwrap().directory.clone();
            let merge = fixture.service.inner.merge.lock().unwrap();
            let (snapshot, progressed) = record.finish_format_change().unwrap();
            assert!(progressed);
            assert_eq!(snapshot.state, "finalizing");
            assert!(snapshot.library.is_none());
            assert_eq!(recovery_segments(&snapshot, &directory).unwrap().len(), 1);
            assert!(fixture.service.inner.state.lock().unwrap().cancel.contains_key(&id));
            drop(merge);
            let job = fixture.settled(&id);
            assert_eq!(job.state, "failed"); // Fixture has no media verifier/tools.
            assert_ne!(job.error.as_deref(), Some(ERROR));
            assert!(job.library.is_none());
            assert!(directory.join("e00000000.mp4").is_file());
            assert!(directory.join("chat-status.final.json").is_file());
        }
    }
    #[test]
    fn youtube_format_boundary_never_promotes_empty_poisoned_cancelled_or_partial_transport() {
        for mode in 0..4 {
            let fixture = Fixture::new();
            let mut record = youtube_archive(&fixture);
            let id = record.snapshot().unwrap().job_id;
            match mode {
                0 => (),
                1 => { assert!(record.append(packet(&record, 9, 0, vec![0; 8])).is_err()); }
                2 => record.writer.as_ref().unwrap().cancel.store(true, Ordering::Release),
                _ => {
                    let mut value = packet(&record, 0, 0, vec![0; 8]);
                    value.final_chunk = false;
                    record.append(value).unwrap();
                }
            }
            assert_eq!(record.format_change_cancel_guard().is_some(), mode == 0);
            let outcome = record.finish_format_change();
            if mode == 0 { assert!(!outcome.unwrap().1); }
            else { assert!(outcome.is_err()); }
            let job = fixture.settled(&id);
            assert_ne!(job.state, "completed");
            assert!(job.library.is_none());
        }
        let fixture = Fixture::new();
        let record = archive(&fixture.lease("soop-only"));
        let id = record.snapshot().unwrap().job_id;
        assert!(record.format_change_cancel_guard().is_none());
        assert!(record.finish_format_change().is_err());
        assert_ne!(fixture.settled(&id).state, "completed");
    }
    #[test]
    fn youtube_format_boundary_does_not_retry_tail_parser_or_commit_failure() {
        for commit_failure in [false, true] {
            let fixture = Fixture::new();
            let mut record = youtube_archive(&fixture);
            let id = record.snapshot().unwrap().job_id;
            let directory = record.writer.as_ref().unwrap().directory.clone();
            for track in 0..2 {
                // Short audio at the same start is a real tail validation error,
                // not the explicitly supported no-A/V-start case.
                let count = if !commit_failure && track == 1 { 2 } else { 4 };
                let bytes = youtube_fragment::explicit_description(&fmp4::fixtures::fragment(track, 0, count, true));
                record.append(packet(&record, track, 0, bytes)).unwrap();
            }
            if commit_failure { fs::create_dir(directory.join("e00000000.mp4")).unwrap(); }
            assert!(record.finish_format_change().is_err());
            let job = fixture.settled(&id);
            assert_eq!(job.state, "failed");
            assert!(job.library.is_none());
        }
    }
    #[test]
    fn scheduled_owner_validation_is_provider_specific() {
        let owner = live::RecordingOwner::Scheduled {
            schedule_id: "UCrNicqyCSAUuxSzUi6VChvw".into(),
            run_id: uuid::Uuid::new_v4().to_string(),
            broadcast_key: "cswFQYam8Ic".into(), generation: 1,
        };
        let reference = MediaRef { provider: "youtube".into(), kind: "live".into(), external_id: "cswFQYam8Ic".into() };
        assert!(validate_owner(&owner, &reference, "cswFQYam8Ic").is_ok());
        assert!(validate_owner(&owner, &reference, "another_vid").is_err());
        assert!(!owner.valid_for("soop"));
        assert!(!owner.valid_for("chzzk"));
        let channel = MediaRef { external_id: "UCrNicqyCSAUuxSzUi6VChvw".into(), ..reference.clone() };
        assert!(validate_owner(&owner, &channel, "cswFQYam8Ic").is_err());
        let mut wrong = owner.clone();
        if let live::RecordingOwner::Scheduled { schedule_id, .. } = &mut wrong { *schedule_id = "somehandle".into(); }
        assert!(!wrong.valid_for("youtube"));
    }
    #[test]
    fn scheduled_query_follows_actual_job_and_rejects_stale_owner_after_view_close() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        let owner = live::RecordingOwner::Scheduled {
            schedule_id: "one".into(),
            run_id: uuid::Uuid::new_v4().to_string(),
            broadcast_key: "123".into(),
            generation: 1,
        };
        let record = lease
            .begin_encoded(
                uuid::Uuid::new_v4().to_string(),
                1,
                uuid::Uuid::new_v4().to_string(),
                fmp4::fixtures::inputs(),
                String::new(),
                String::new(),
                owner.clone(),
            )
            .unwrap();
        let capture = live::ScheduledCapture {
            session_id: lease.receiver_id().into(),
            job_id: record.snapshot().unwrap().job_id,
            owner,
        };
        assert!(
            fixture
                .service
                .encoded_scheduled_state(&capture)
                .unwrap()
                .active
        );
        let wrong = live::ScheduledCapture {
            session_id: uuid::Uuid::new_v4().to_string(),
            ..capture.clone()
        };
        assert!(fixture.service.encoded_scheduled_state(&wrong).is_err());
        drop(record);
        fixture.settled(&capture.job_id);
        drop(lease);
        let status = fixture.service.encoded_scheduled_state(&capture).unwrap();
        assert_eq!(status.state, "failed");
        assert!(!status.active);
    }
    #[test]
    fn begin_checks_identity_tracks_scheduled_owner_and_never_creates_job_on_rejection() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        for invalid in [0, 1, 2, 3] {
            let mut tracks = fmp4::fixtures::inputs();
            match invalid {
                0 => tracks[1].track_index = 0,
                1 => tracks[0].init = vec![0; 65537],
                2 => tracks[0].mime_type = "x".repeat(121),
                _ => tracks[0].track_index = 2,
            }
            assert!(lease
                .begin_encoded(
                    uuid::Uuid::new_v4().to_string(),
                    1,
                    uuid::Uuid::new_v4().to_string(),
                    tracks,
                    String::new(),
                    String::new(),
                    live::RecordingOwner::Manual
                )
                .is_err());
        }
        assert!(lease
            .begin_encoded(
                uuid::Uuid::new_v4().to_string(),
                1,
                uuid::Uuid::new_v4().to_string(),
                fmp4::fixtures::inputs(),
                String::new(),
                String::new(),
                live::RecordingOwner::Scheduled {
                    schedule_id: "other".into(),
                    run_id: uuid::Uuid::new_v4().to_string(),
                    broadcast_key: "123".into(),
                    generation: 1
                }
            )
            .is_err());
        assert!(fixture.service.jobs(None).is_empty());
        let first = archive(&lease);
        assert!(lease
            .begin_encoded(
                uuid::Uuid::new_v4().to_string(),
                1,
                uuid::Uuid::new_v4().to_string(),
                fmp4::fixtures::inputs(),
                String::new(),
                String::new(),
                live::RecordingOwner::Manual
            )
            .is_err());
        let id = first.snapshot().unwrap().job_id;
        drop(first);
        assert_ne!(fixture.settled(&id).state, "completed");
    }
    #[test]
    fn exact_chunk_duplicate_is_acknowledged_changed_or_reordered_chunk_poisons_writer() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        let mut record = archive(&lease);
        let bytes = fmp4::fixtures::fragment(0, 0, 2, true);
        let first = record.append(packet(&record, 0, 0, bytes.clone())).unwrap();
        assert!(!first.duplicate);
        assert!(
            record
                .append(packet(&record, 0, 0, bytes.clone()))
                .unwrap()
                .duplicate
        );
        let mut changed = bytes;
        *changed.last_mut().unwrap() ^= 1;
        assert!(record.append(packet(&record, 0, 0, changed)).is_err());
        assert!(record
            .append(packet(
                &record,
                1,
                0,
                fmp4::fixtures::fragment(1, 0, 2, true)
            ))
            .is_err());
        let id = record.snapshot().unwrap().job_id;
        let finished = record.finish(None).unwrap();
        assert_eq!(finished.state, "finalizing");
        assert_ne!(fixture.settled(&id).state, "completed");
    }
    #[test]
    fn source_generation_size_order_and_partial_tracks_are_fail_closed() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        for mutation in 0..6 {
            let mut record = archive(&lease);
            let mut value = packet(&record, 0, 0, fmp4::fixtures::fragment(0, 0, 2, true));
            match mutation {
                0 => value.source_id = uuid::Uuid::new_v4().to_string(),
                1 => value.generation = 2,
                2 => value.bytes = vec![0; MAX_CHUNK + 1],
                3 => value.chunk = 1,
                4 => value.index = 1,
                _ => value.track_index = 3,
            }
            assert!(record.append(value).is_err());
            let id = record.snapshot().unwrap().job_id;
            drop(record);
            fixture.settled(&id);
        }
        let mut record = archive(&lease);
        let mut first = packet(&record, 0, 0, vec![0; MAX_CHUNK]);
        first.final_chunk = false;
        record.append(first).unwrap();
        let different = packet(&record, 1, 0, vec![1; 1]);
        assert!(record.append(different).is_err());
        let id = record.snapshot().unwrap().job_id;
        drop(record);
        fixture.settled(&id);
    }
    #[test]
    fn segments_have_distinct_durable_identity_hashes_and_local_only_recovery() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        let mut record = archive(&lease);
        for track in [0, 1] {
            record
                .append(packet(
                    &record,
                    track,
                    0,
                    fmp4::fixtures::fragment(track, 0, 4, true),
                ))
                .unwrap();
        }
        let writer = record.writer.as_mut().unwrap();
        let parts = writer.muxer.finish().unwrap();
        writer.commit(parts).unwrap();
        let job = record.snapshot().unwrap();
        let directory = record.writer.as_ref().unwrap().directory.clone();
        assert_eq!(recovery_segments(&job, &directory).unwrap().len(), 1);
        assert!(directory.join("e00000000.mp4").is_file());
        assert!(!directory.join("segments.jsonl").exists());
        assert!(validate_proof(
            &MediaJob {
                reference: reference("wrong"),
                ..job.clone()
            },
            &directory
        )
        .is_err());
        let data = fs::read(directory.join("e00000000.mp4")).unwrap();
        fs::write(directory.join("e00000000.mp4"), vec![0; data.len()]).unwrap();
        assert!(recovery_segments(&job, &directory).is_err());
        fs::write(directory.join("e00000000.mp4"), &data).unwrap();
        drop(record);
        fixture.settled(&job.job_id);
        let error = fixture
            .service
            .resume_live_recording(&job, &Arc::new(AtomicBool::new(false)))
            .unwrap_err();
        assert!(!error.is_empty()); // absent media tools, never a network fallback
        assert!(directory.join("e00000000.mp4").is_file());
        assert!(directory.join("chat-status.final.json").is_file());
        assert!(!directory.join("media.mp4").exists());
    }
    #[test]
    fn shutdown_blocks_append_retains_active_owner_until_archive_drain() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        let mut record = archive(&lease);
        let job = record.snapshot().unwrap();
        fixture.service.shutdown();
        assert!(lease.is_closed());
        assert!(record.append(packet(&record, 0, 0, vec![0; 8])).is_err());
        assert!(fixture
            .service
            .active_ids()
            .iter()
            .any(|id| id == &format!("platform-media:{}", job.job_id)));
        drop(record);
        assert_eq!(fixture.settled(&job.job_id).state, "interrupted");
        drop(lease);
        assert!(fixture.service.active_ids().is_empty());
    }
    #[test]
    fn explicit_interruption_saves_parseable_tail_without_reporting_completion() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        let mut record = archive(&lease);
        for track in [0, 1] {
            record
                .append(packet(
                    &record,
                    track,
                    0,
                    fmp4::fixtures::fragment(track, 0, 4, true),
                ))
                .unwrap();
        }
        let job = record.snapshot().unwrap();
        let directory = record.writer.as_ref().unwrap().directory.clone();
        assert!(!directory.join(MANIFEST).exists());
        record.finish(Some("fixture source closed".into())).unwrap();
        assert_eq!(fixture.settled(&job.job_id).state, "failed");
        assert_eq!(recovery_segments(&job, &directory).unwrap().len(), 1);
        assert!(directory.join("e00000000.mp4").is_file());
        assert!(!directory.join("media.mp4").exists());
        let outcome = soop_finalize::outcome(&job, &directory, &AtomicBool::new(false)).unwrap().unwrap();
        assert_eq!(serde_json::to_value(outcome).unwrap()["state"], "interrupted");
        assert!(directory.join("chat-status.final.json").is_file());
    }
    #[test]
    fn soop_incomplete_box_and_ipc_tail_salvage_only_validated_prefix() {
        for ipc in [false, true] {
            let fixture = Fixture::new();
            let lease = fixture.lease("one");
            let mut record = archive(&lease);
            for track in [0, 1] {
                record.append(packet(&record, track, 0, fmp4::fixtures::fragment(track, 0, 4, true))).unwrap();
            }
            let mut tail = packet(&record, 0, 1, vec![0, 0, 0]);
            tail.final_chunk = !ipc;
            record.append(tail).unwrap();
            let job = record.snapshot().unwrap();
            let directory = record.writer.as_ref().unwrap().directory.clone();
            record.finish(None).unwrap();
            assert_eq!(fixture.settled(&job.job_id).state, "failed"); // absent tools, never a false completed file
            assert_eq!(recovery_segments(&job, &directory).unwrap().len(), 1);
            assert!(soop_finalize::outcome(&job, &directory, &AtomicBool::new(false)).unwrap().is_some());
            assert!(!directory.join("media.mp4").exists());
        }
    }
    #[test]
    fn soop_stop_proof_is_exact_bounded_and_does_not_hide_unknown_or_poisoned_state() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        let mut record = archive(&lease);
        for track in [0, 1] {
            record.append(packet(&record, track, 0, fmp4::fixtures::fragment(track, 0, 4, true))).unwrap();
        }
        let job = record.snapshot().unwrap();
        let directory = record.writer.as_ref().unwrap().directory.clone();
        assert!(soop_finalize::outcome(&job, &directory, &AtomicBool::new(false)).unwrap().is_none());
        record.finish(Some("source stopped".into())).unwrap();
        fixture.settled(&job.job_id);
        let path = directory.join("encoded-stop.json");
        let original = fs::read(&path).unwrap();
        for (field, value) in [("version", serde_json::json!(2)), ("job_id", serde_json::json!(uuid::Uuid::new_v4().to_string())),
            ("owner_sha256", serde_json::json!("00")), ("manifest_sha256", serde_json::json!("00")), ("extra", serde_json::json!(true))] {
            let mut proof: serde_json::Value = serde_json::from_slice(&original).unwrap();
            proof[field] = value;
            fs::write(&path, serde_json::to_vec(&proof).unwrap()).unwrap();
            assert!(soop_finalize::outcome(&job, &directory, &AtomicBool::new(false)).is_err());
        }
        fs::write(&path, vec![b' '; 2049]).unwrap();
        assert!(soop_finalize::outcome(&job, &directory, &AtomicBool::new(false)).is_err());
        fs::write(&path, &original).unwrap();
        assert!(soop_finalize::outcome(&job, &directory, &AtomicBool::new(true)).is_err());
        fs::OpenOptions::new().append(true).open(directory.join(MANIFEST)).unwrap().write_all(b"\n").unwrap();
        assert!(soop_finalize::outcome(&job, &directory, &AtomicBool::new(false)).is_err());
        let other = fixture.lease("two");
        let mut poisoned = archive(&other);
        let bad_job = poisoned.snapshot().unwrap();
        let bad_dir = poisoned.writer.as_ref().unwrap().directory.clone();
        assert!(poisoned.append(packet(&poisoned, 0, 1, vec![0; 8])).is_err());
        poisoned.finish(Some("stopped".into())).unwrap();
        fixture.settled(&bad_job.job_id);
        assert!(!bad_dir.join("encoded-stop.json").exists());
    }
    #[test]
    fn soop_retry_owner_rejects_cancel_shutdown_replaced_token_and_completed_job() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        let record = archive(&lease);
        let writer = record.writer.as_ref().unwrap();
        let job = writer.job.clone();
        let cancel = writer.cancel.clone();
        assert!(soop_finalize::owner(&fixture.service, &job, &cancel).is_ok());
        cancel.store(true, Ordering::Release);
        assert!(soop_finalize::owner(&fixture.service, &job, &cancel).is_err());
        cancel.store(false, Ordering::Release);
        {
            let mut state = fixture.service.inner.state.lock().unwrap();
            state.shutting_down = true;
        }
        assert!(soop_finalize::owner(&fixture.service, &job, &cancel).is_err());
        {
            let mut state = fixture.service.inner.state.lock().unwrap();
            state.shutting_down = false;
            state.cancel.insert(job.job_id.clone(), Arc::new(AtomicBool::new(false)));
        }
        assert!(soop_finalize::owner(&fixture.service, &job, &cancel).is_err());
        {
            let mut state = fixture.service.inner.state.lock().unwrap();
            state.cancel.insert(job.job_id.clone(), cancel.clone());
            state.jobs.get_mut(&job.job_id).unwrap().state = "completed".into();
        }
        assert!(soop_finalize::owner(&fixture.service, &job, &cancel).is_err());
        assert!(soop_finalize::start(&fixture.service, &job, &cancel).is_err());
        assert_eq!(fixture.service.inner.state.lock().unwrap().jobs[&job.job_id].state, "completed");
        drop(record);
        assert_eq!(fixture.settled(&job.job_id).state, "completed");
    }
    #[test]
    fn soop_incomplete_audio_tail_keeps_only_previously_committed_segments() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        let mut record = archive(&lease);
        for track in [0, 1] {
            record.append(packet(&record, track, 0, fmp4::fixtures::fragment(track, 0, 24, true))).unwrap();
        }
        record.append(packet(&record, 0, 1, fmp4::fixtures::fragment(0, 12 * u64::from(fmp4::fixtures::VIDEO_SCALE), 4, true))).unwrap();
        let job = record.snapshot().unwrap();
        let directory = record.writer.as_ref().unwrap().directory.clone();
        let previous = recovery_segments(&job, &directory).unwrap();
        assert_eq!(previous.len(), 1);
        assert_eq!(previous[0].1, 12.0);
        record.finish(None).unwrap();
        assert_eq!(fixture.settled(&job.job_id).state, "failed"); // no tool in this pure fixture
        assert_eq!(recovery_segments(&job, &directory).unwrap(), previous);
        assert!(soop_finalize::outcome(&job, &directory, &AtomicBool::new(false)).unwrap().is_some());
    }
    #[test]
    fn soop_empty_prefix_commit_collision_and_existing_stop_marker_never_promote() {
        for mode in 0..3 {
            let fixture = Fixture::new();
            let lease = fixture.lease("one");
            let mut record = archive(&lease);
            let job = record.snapshot().unwrap();
            let directory = record.writer.as_ref().unwrap().directory.clone();
            if mode != 0 {
                for track in [0, 1] {
                    record.append(packet(&record, track, 0, fmp4::fixtures::fragment(track, 0, 4, true))).unwrap();
                }
                if mode == 1 { fs::create_dir(directory.join("e00000000.mp4")).unwrap(); }
                else { fs::write(directory.join("encoded-stop.json"), b"preserve unknown marker").unwrap(); }
            }
            record.finish(Some("stopped".into())).unwrap();
            assert_eq!(fixture.settled(&job.job_id).state, "failed");
            if mode == 2 {
                assert_eq!(fs::read(directory.join("encoded-stop.json")).unwrap(), b"preserve unknown marker");
            } else { assert!(!directory.join("encoded-stop.json").exists()); }
            assert!(!directory.join("media.mp4").exists());
            assert!(directory.join("chat-status.final.json").is_file());
        }
    }
    #[test]
    fn finalizer_keeps_actual_cancel_owner_while_waiting_for_merge_slot() {
        let fixture = Fixture::new();
        let lease = fixture.lease("one");
        let mut record = archive(&lease);
        for track in [0, 1] {
            record
                .append(packet(
                    &record,
                    track,
                    0,
                    fmp4::fixtures::fragment(track, 0, 4, true),
                ))
                .unwrap();
        }
        let id = record.snapshot().unwrap().job_id;
        let merge = fixture.service.inner.merge.lock().unwrap();
        let snapshot = record.finish(None).unwrap();
        assert_eq!(snapshot.state, "finalizing");
        assert!(fixture
            .service
            .inner
            .state
            .lock()
            .unwrap()
            .cancel
            .contains_key(&id));
        assert!(fixture
            .service
            .active_ids()
            .iter()
            .any(|value| value == &format!("platform-media:{id}")));
        drop(merge);
        assert_eq!(fixture.settled(&id).state, "failed"); // fixture deliberately has no FFmpeg
    }
    #[test]
    fn encoded_source_has_no_media_fetcher_and_does_not_import_mutable_chzzk_state() {
        let source = include_str!("encoded.rs")
            .split("\nmod tests")
            .next()
            .unwrap();
        for forbidden in [
            ".yt_dlp",
            ".deno",
            "process::run",
            "OfficialBrowser",
            "BrowserCaptureStore",
            "chzzk-browser-profile",
        ] {
            assert!(!source.contains(forbidden), "{forbidden}");
        }
        assert!(source.contains("../streaming/browser_fmp4.rs"));
        assert!(source.contains("validate_proof"));
        assert_eq!(segment_sequence("e00000000.mp4"), Some(0));
        for name in [
            "../e00000000.mp4",
            "e00000000.ts",
            "s00000000.ts",
            "e00000000.mp4:stream",
        ] {
            assert!(segment_sequence(name).is_none());
        }
    }
}
