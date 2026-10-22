//! Native video jobs for YouTube/SOOP. Image-platform state is intentionally separate.
mod broadcast;
mod capture_outcome;
mod chat_assets;
mod chat_parts;
mod chat_status;
pub mod encoded;
pub mod groups;
mod lifecycle;
pub mod live;
pub mod live_chat;
mod process;
mod profile;
pub mod protocol;
mod queries;
pub mod replay_chat;
pub mod screenshots;
mod soop_vod_chat;
mod soop_vod;
pub(crate) mod storage;
mod youtube_chat;
mod youtube_publication;
mod youtube_raw;
mod youtube_spool;
#[cfg(debug_assertions)]
pub use youtube_spool::{inspect_automatic_raw_qa, RawAutomaticQaEvidence, RawAutomaticQaError};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
pub struct PlatformMediaTools {
    pub yt_dlp: PathBuf,
    pub deno: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaRef {
    pub provider: String,
    pub kind: String,
    pub external_id: String,
}
impl MediaRef {
    pub fn canonical_url(&self) -> Result<String, String> {
        let id = &self.external_id;
        match (self.provider.as_str(), self.kind.as_str()) {
            ("youtube", "video" | "live")
                if id.len() == 11
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) =>
            {
                Ok(format!("https://www.youtube.com/watch?v={id}"))
            }
            ("soop", "video")
                if !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit()) =>
            {
                Ok(format!("https://vod.sooplive.com/player/{id}"))
            }
            ("soop", "live")
                if !id.is_empty()
                    && id.len() <= 40
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) =>
            {
                Ok(format!("https://play.sooplive.co.kr/{id}"))
            }
            _ => Err("지원하는 영상이 아닙니다.".into()),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    #[serde(
        default,
        deserialize_with = "broadcast::deserialize_optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub broadcast_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_archive: Option<chat_status::ChatArchiveSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_outcome: Option<capture_outcome::CaptureOutcome>,
    pub library_id: String,
    pub revision: u64,
    pub reference: MediaRef,
    pub title: String,
    pub channel_name: String,
    pub bytes: u64,
    pub duration: f64,
    pub sha256: String,
    pub completed_at: u64,
    #[serde(default)]
    pub modified_millis: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaJob {
    #[serde(
        default,
        deserialize_with = "broadcast::deserialize_optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub broadcast_id: Option<String>,
    pub job_id: String,
    pub request_id: String,
    pub reference: MediaRef,
    pub revision: u64,
    pub state: String,
    pub title: String,
    pub channel_name: String,
    pub created_at: u64,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<String>,
    pub library: Option<LibraryItem>,
    pub download_root: PathBuf,
}
impl MediaJob {
    pub fn active(&self) -> bool {
        matches!(
            self.state.as_str(),
            "queued" | "resolving" | "downloading" | "recording" | "finalizing" | "verifying"
        )
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPage {
    pub items: Vec<LibraryItem>,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playback {
    pub token: String,
    pub url: String,
    pub item: LibraryItem,
}
struct State {
    jobs: HashMap<String, MediaJob>,
    queue: VecDeque<String>,
    cancel: HashMap<String, Arc<AtomicBool>>,
    shutting_down: bool,
    persistence_errors: HashMap<String, String>,
    persistence_overflow: bool,
}
struct Inner {
    state: Mutex<State>,
    ready: Condvar,
    verify: Mutex<()>,
    merge: Mutex<()>,
    journal: PathBuf,
    tools: PlatformMediaTools,
    pub(crate) playback: Mutex<HashMap<String, protocol::Session>>,
    pub(crate) development_url: Option<reqwest::Url>,
    live: Mutex<live::LiveRegistry>,
    network: Mutex<usize>,
    screenshots: screenshots::Store,
    profiles: profile::Snapshots,
    #[cfg(test)]
    synthetic_live_receiver: AtomicBool,
}
#[derive(Clone)]
pub struct PlatformMediaService {
    inner: Arc<Inner>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn clean(value: Option<&str>, limit: usize) -> String {
    value
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}

impl PlatformMediaService {
    pub fn new(
        data_dir: PathBuf,
        tools: PlatformMediaTools,
        development_url: Option<reqwest::Url>,
    ) -> Result<Self, String> {
        storage::safe_path(&data_dir)?;
        let journal = data_dir.join("platform-media/jobs");
        fs::create_dir_all(&journal).map_err(|_| "영상 작업 저장소를 열지 못했습니다.")?;
        let mut jobs = HashMap::new();
        for mut job in storage::load_jobs(&journal) {
            if job.active() {
                job.state = "interrupted".into();
                job.revision += 1;
                if !encoded::raw_youtube_integrity_failed(&job) {
                    job.error = Some("앱 종료로 중단됐습니다. 다시 시도할 수 있습니다.".into());
                }
                storage::save_job(&journal, &job)?;
            }
            jobs.insert(job.job_id.clone(), job);
        }
        let service = Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    jobs,
                    queue: VecDeque::new(),
                    cancel: HashMap::new(),
                    shutting_down: false,
                    persistence_errors: HashMap::new(),
                    persistence_overflow: false,
                }),
                ready: Condvar::new(),
                verify: Mutex::new(()),
                merge: Mutex::new(()),
                journal,
                tools,
                playback: Mutex::new(HashMap::new()),
                development_url,
                live: Mutex::new(live::LiveRegistry::default()),
                network: Mutex::new(0),
                screenshots: screenshots::Store::new(data_dir.join("platform-media/screenshots")),
                profiles: profile::Snapshots::default(),
                #[cfg(test)]
                synthetic_live_receiver: AtomicBool::new(false),
            }),
        };
        for _ in 0..2 {
            let service = service.clone();
            std::thread::Builder::new()
                .name("platform-media".into())
                .spawn(move || service.worker())
                .map_err(|_| "영상 작업을 준비하지 못했습니다.")?;
        }
        Ok(service)
    }
    pub fn download(
        &self,
        reference: MediaRef,
        request_id: String,
        download_root: PathBuf,
    ) -> Result<MediaJob, String> {
        self.download_batch(vec![reference], request_id, download_root)?
            .into_iter()
            .next()
            .ok_or("저장 요청이 없습니다.".into())
    }
    pub fn download_batch(
        &self,
        references: Vec<MediaRef>,
        request_id: String,
        download_root: PathBuf,
    ) -> Result<Vec<MediaJob>, String> {
        if references.is_empty() || references.len() > 100 {
            return Err("한 번에 1~100개 영상을 선택해 주세요.".into());
        }
        for reference in &references {
            reference.canonical_url()?;
            if reference.kind != "video" {
                return Err("라이브를 재생한 뒤 녹화 버튼을 눌러 주세요.".into());
            }
        }
        if uuid::Uuid::parse_str(&request_id).is_err() {
            return Err("요청 식별자가 올바르지 않습니다.".into());
        }
        storage::safe_path(&download_root)?;
        // Tools are supplied by the managed, hash-verified startup resolver, never searched on PATH.
        for path in [
            &self.inner.tools.yt_dlp,
            &self.inner.tools.deno,
            &self.inner.tools.ffmpeg,
            &self.inner.tools.ffprobe,
        ] {
            storage::regular(path)?;
        }
        let mut state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
        if state.shutting_down {
            return Err("앱을 종료하는 중입니다.".into());
        }
        let mut result = Vec::new();
        let mut pending = Vec::new();
        for (index, reference) in references.into_iter().enumerate() {
            let item_request_id = format!("{request_id}:{index}");
            if let Some(job) = state.jobs.values().chain(pending.iter()).find(|j| {
                j.request_id == item_request_id
                    || (j.reference == reference && (j.active() || j.state == "completed"))
            }) {
                if job.request_id == item_request_id && job.reference != reference {
                    return Err("이미 사용한 요청 식별자입니다.".into());
                }
                result.push(job.clone());
                continue;
            }
            let job = MediaJob {
                broadcast_id: None,
                job_id: uuid::Uuid::new_v4().to_string(),
                request_id: item_request_id,
                reference,
                revision: 1,
                state: "queued".into(),
                title: String::new(),
                channel_name: String::new(),
                created_at: now(),
                received_bytes: 0,
                total_bytes: None,
                error: None,
                library: None,
                download_root: download_root.clone(),
            };
            result.push(job.clone());
            pending.push(job);
        }
        if state.jobs.values().filter(|j| j.active()).count() + pending.len()
            > queries::ACTIVE_JOB_LIMIT
        {
            return Err("대기 작업이 많습니다. 완료 후 다시 시도해 주세요.".into());
        }
        // No worker can observe a partial batch, and the outer quit/update gate covers registration.
        storage::save_batch(&self.inner.journal, &pending)?;
        let mut portraits = Vec::new();
        for job in pending {
            state.queue.push_back(job.job_id.clone());
            let cancel = Arc::new(AtomicBool::new(false));
            state.cancel.insert(job.job_id.clone(), cancel.clone());
            portraits.push((job.clone(), cancel));
            state.jobs.insert(job.job_id.clone(), job);
        }
        self.inner.ready.notify_all();
        drop(state);
        for (job, cancel) in portraits { self.schedule_profile(&job, &cancel, None); }
        Ok(result)
    }
    pub fn jobs(&self, provider: Option<&str>) -> Vec<MediaJob> {
        let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        queries::jobs(state.jobs.values(), provider)
    }
    pub fn active_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel
            .keys()
            .map(|id| format!("platform-media:{id}"))
            .collect();
        ids.extend(self.live_active_ids());
        ids.extend(self.inner.screenshots.active_ids());
        ids.extend(self.inner.profiles.active_ids());
        ids.sort();
        ids.dedup();
        ids
    }
    pub fn cancel(&self, job_id: &str, revision: u64) -> Result<MediaJob, String> {
        let mut state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
        let job = state.jobs.get(job_id).ok_or("작업을 찾을 수 없습니다.")?;
        if job.revision != revision {
            return Err("작업 상태가 변경됐습니다. 다시 시도해 주세요.".into());
        }
        if !job.active() {
            return Ok(job.clone());
        }
        if let Some(cancel) = state.cancel.get(job_id) {
            cancel.store(true, Ordering::Release);
        }
        if job.state == "queued" && state.queue.iter().any(|id| id == job_id) {
            let job = state.jobs.get_mut(job_id).unwrap();
            job.state = "cancelled".into();
            job.revision += 1;
            let persisted = storage::save_job(&self.inner.journal, job);
            state.queue.retain(|id| id != job_id);
            state.cancel.remove(job_id);
            if let Err(error) = persisted {
                let error = lifecycle::persistence_message(&error);
                lifecycle::remember_persistence_error(&mut state, job_id, error.clone());
                state.jobs.get_mut(job_id).unwrap().error = Some(error.clone());
                return Err(error);
            }
            state.persistence_errors.remove(job_id);
        }
        Ok(state.jobs[job_id].clone())
    }
    pub fn retry(&self, job_id: &str, revision: u64) -> Result<MediaJob, String> {
        let old = {
            let state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
            let job = state.jobs.get(job_id).ok_or("작업을 찾을 수 없습니다.")?;
            if job.revision != revision
                || !matches!(job.state.as_str(), "failed" | "interrupted" | "cancelled")
            {
                return Err("다시 시도할 수 있는 작업이 아닙니다.".into());
            }
            job.clone()
        };
        if encoded::raw_youtube_integrity_failed(&old) {
            return Err(old.error.unwrap());
        }
        if old.reference.kind == "live" {
            // Retry the existing archive, never a new broadcast under the same channel ID.
            let mut state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
            if state.shutting_down {
                return Err("앱을 종료하는 중입니다.".into());
            }
            if state
                .jobs
                .get(job_id)
                .is_none_or(|job| job.revision != revision)
            {
                return Err("작업 상태가 변경됐습니다.".into());
            }
            if state.jobs.values().filter(|j| j.active()).count() >= queries::ACTIVE_JOB_LIMIT {
                return Err("대기 작업이 많습니다.".into());
            }
            let mut resumed = old;
            resumed.state = "queued".into();
            resumed.error = None;
            resumed.revision += 1;
            storage::save_job(&self.inner.journal, &resumed)?;
            state.persistence_errors.remove(job_id);
            state
                .cancel
                .insert(job_id.into(), Arc::new(AtomicBool::new(false)));
            state.queue.push_back(job_id.into());
            state.jobs.insert(job_id.into(), resumed.clone());
            self.inner.ready.notify_all();
            return Ok(resumed);
        }
        self.download(
            old.reference,
            uuid::Uuid::new_v4().to_string(),
            old.download_root,
        )
    }
    pub fn library(
        &self,
        provider: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<LibraryPage, String> {
        let (mut page, roots) = {
            let state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
            let page = queries::library(state.jobs.values(), provider, cursor, limit)?;
            let roots: HashMap<_, _> = page
                .items
                .iter()
                .filter_map(|item| {
                    state
                        .jobs
                        .get(&item.library_id)
                        .map(|job| (item.library_id.clone(), job.download_root.clone()))
                })
                .collect();
            (page, roots)
        };
        for item in &mut page.items {
            if let Some(root) = roots.get(&item.library_id) {
                Self::refresh_chat_summary(item, root);
            }
        }
        Ok(page)
    }
    pub fn library_groups(
        &self,
        provider: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<groups::GroupsPage, String> {
        let (mut page, roots) = {
            let state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
            let page = groups::page(state.jobs.values(), provider, cursor, limit)?;
            let roots: HashMap<_, _> = match &page {
                groups::GroupsPage::Ready { entries, .. } => entries
                    .iter()
                    .filter_map(|entry| {
                        let item = entry.item();
                        state
                            .jobs
                            .get(&item.library_id)
                            .map(|job| (item.library_id.clone(), job.download_root.clone()))
                    })
                    .collect(),
                groups::GroupsPage::Changed { .. } => HashMap::new(),
            };
            (page, roots)
        };
        if let groups::GroupsPage::Ready { entries, .. } = &mut page {
            for entry in entries {
                let item = entry.item_mut();
                if let Some(root) = roots.get(&item.library_id) {
                    Self::refresh_chat_summary(item, root);
                }
            }
        }
        Ok(page)
    }
    pub fn library_group_members(
        &self,
        provider: &str,
        group_key: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<groups::MembersPage, String> {
        let (mut page, roots) = {
            let state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
            let page = groups::members(state.jobs.values(), provider, group_key, cursor, limit)?;
            let roots: HashMap<_, _> = match &page {
                groups::MembersPage::Ready { items, .. } => items
                    .iter()
                    .filter_map(|item| {
                        state
                            .jobs
                            .get(&item.library_id)
                            .map(|job| (item.library_id.clone(), job.download_root.clone()))
                    })
                    .collect(),
                _ => HashMap::new(),
            };
            (page, roots)
        };
        if let groups::MembersPage::Ready { group, items, .. } = &mut page {
            for item in items.iter_mut() {
                if let Some(root) = roots.get(&item.library_id) {
                    Self::refresh_chat_summary(item, root);
                }
            }
            // Never inspect an extra representative beyond the page's100 files.
            // Its warning is file-specific, not a health claim about the group.
            if let Some(item) = items
                .iter()
                .find(|item| item.library_id == group.item().library_id)
            {
                group.item_mut().chat_archive = item.chat_archive.clone();
            }
        }
        Ok(page)
    }
    fn refresh_chat_summary(item: &mut LibraryItem, root: &std::path::Path) {
        if item.reference.kind != "live" {
            return;
        }
        match storage::job_dir(root, &item.reference.provider, &item.library_id)
            .and_then(|dir| chat_status::inspect(&dir, &item.reference.provider, &item.library_id))
        {
            Ok(Some(summary)) => item.chat_archive = Some(summary),
            Ok(None) if item.chat_archive.is_none() => {} // legacy; no retroactive migration
            _ => item.chat_archive = Some(chat_status::ChatArchiveSummary::unknown()),
        }
    }
    pub fn open(&self, library_id: &str) -> Result<Playback, String> {
        let state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
        let job = state
            .jobs
            .get(library_id)
            .filter(|j| j.state == "completed")
            .ok_or("완료된 영상을 찾을 수 없습니다.")?
            .clone();
        drop(state);
        let mut item = job.library.clone().ok_or("파일 검증 정보가 없습니다.")?;
        Self::refresh_chat_summary(&mut item, &job.download_root);
        let path = storage::owned_file(&job, &item)?;
        let mut playback = self.inner.playback.lock().map_err(|_| "재생 상태 오류")?;
        if playback.len() >= 8 {
            return Err("열려 있는 영상을 먼저 닫아 주세요.".into());
        }
        let token = uuid::Uuid::new_v4().to_string();
        let session = protocol::Session::new(path, library_id.into(), item.bytes)?;
        playback.insert(token.clone(), session);
        Ok(Playback {
            url: format!("http://platform-media.localhost/{token}/media.mp4"),
            token,
            item,
        })
    }
    pub fn close(&self, token: &str) {
        let released = self.inner.playback.lock().ok().and_then(|mut playback| {
            let session = playback.remove(token)?;
            (!playback
                .values()
                .any(|other| other.library_id == session.library_id))
            .then_some(session.library_id)
        });
        if let Some(id) = released {
            let directory = self.inner.state.lock().ok().and_then(|state| {
                state.jobs.get(&id).and_then(|job| {
                    storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id).ok()
                })
            });
            if let Some(directory) = directory {
                replay_chat::release_directory(&directory);
            }
        }
    }
    pub fn profile(&self, library_id: &str) -> Result<Option<String>, String> {
        let directory = {
            let state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
            let job = state
                .jobs
                .get(library_id)
                .filter(|job| job.state == "completed")
                .ok_or("영상을 찾을 수 없습니다.")?;
            if !matches!(job.reference.provider.as_str(), "soop" | "youtube") {
                return Ok(None);
            }
            let item = job.library.as_ref().ok_or("영상 검증 정보가 없습니다.")?;
            storage::owned_file(job, item)?;
            storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id)?
        };
        profile::read(&directory)
    }
    pub fn thumbnail(&self, library_id: &str) -> Result<Option<String>, String> {
        let state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
        let job = state
            .jobs
            .get(library_id)
            .filter(|j| j.state == "completed")
            .ok_or("영상을 찾을 수 없습니다.")?;
        let path = storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id)?
            .join("thumbnail.jpg");
        if !path.exists() {
            return Ok(None);
        }
        let meta = storage::regular(&path)?;
        if meta.len() > 96 * 1024 {
            return Err("미리보기 파일이 너무 큽니다.".into());
        }
        let bytes = fs::read(path).map_err(|_| "미리보기를 읽지 못했습니다.")?;
        if !bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            return Err("미리보기 형식이 올바르지 않습니다.".into());
        }
        Ok(Some(format!(
            "data:image/jpeg;base64,{}",
            STANDARD.encode(bytes)
        )))
    }
    pub fn replay_chat(
        &self,
        library_id: &str,
        position_ms: u64,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<replay_chat::ReplayChatPage, String> {
        let job = {
            let state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
            state
                .jobs
                .get(library_id)
                .filter(|job| job.state == "completed")
                .cloned()
                .ok_or("완료된 녹화를 찾을 수 없습니다.")?
        };
        let item = job.library.as_ref().ok_or("녹화 검증 정보가 없습니다.")?;
        storage::owned_file(&job, item)?;
        let directory = storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id)?;
        replay_chat::read_page_for_job(
            &directory,
            &job.reference.provider,
            &job.job_id,
            position_ms,
            cursor,
            limit,
        )
    }
    pub fn replay_chat_asset(
        &self,
        library_id: &str,
        asset_id: &str,
    ) -> Result<live_chat::ChatAssetRead, String> {
        let job = self
            .inner
            .state
            .lock()
            .map_err(|_| "영상 작업 상태 오류")?
            .jobs
            .get(library_id)
            .filter(|job| job.state == "completed")
            .cloned()
            .ok_or("완료된 녹화를 찾을 수 없습니다.")?;
        storage::owned_file(
            &job,
            job.library.as_ref().ok_or("녹화 검증 정보가 없습니다.")?,
        )?;
        let directory = storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id)?;
        chat_assets::replay_asset(&directory, asset_id).map(chat_assets::ChatAssetRead::local)
    }
    pub fn open_folder(&self, library_id: &str) -> Result<(), String> {
        let state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
        let job = state
            .jobs
            .get(library_id)
            .ok_or("영상을 찾을 수 없습니다.")?;
        let path = storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id)?;
        if !path.is_dir() {
            return Err("저장 폴더를 찾을 수 없습니다.".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows::{
                core::{w, PCWSTR},
                Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
            };
            let text: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe {
                ShellExecuteW(
                    None,
                    w!("open"),
                    PCWSTR(text.as_ptr()),
                    None,
                    None,
                    SW_SHOWNORMAL,
                )
            }
            .0 as isize
                <= 32
            {
                return Err("저장 폴더를 열지 못했습니다.".into());
            }
        }
        Ok(())
    }
    pub fn delete(&self, library_id: &str, revision: u64) -> Result<(), String> {
        let mut state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
        let job = state
            .jobs
            .get(library_id)
            .ok_or("영상을 찾을 수 없습니다.")?;
        if state.cancel.contains_key(library_id)
            || job.active()
            || job
                .library
                .as_ref()
                .is_none_or(|item| item.revision != revision)
        {
            return Err("파일 상태가 변경됐습니다.".into());
        }
        if self
            .inner
            .playback
            .lock()
            .map_err(|_| "재생 상태 오류")?
            .values()
            .any(|s| s.library_id == library_id)
        {
            return Err("재생 창을 닫은 뒤 삭제해 주세요.".into());
        }
        let path = storage::job_dir(&job.download_root, &job.reference.provider, &job.job_id)?
            .join("media.mp4");
        if path.exists() {
            let item = job.library.as_ref().unwrap();
            storage::owned_file(job, item)?;
            fs::remove_file(&path).map_err(|_| "파일을 삭제하지 못했습니다.")?;
        } else if !job.download_root.is_dir() {
            return Err("저장 드라이브에 연결할 수 없습니다.".into());
        }
        let job = state.jobs.get_mut(library_id).unwrap();
        job.state = "deleted".into();
        job.library = None;
        job.revision += 1;
        storage::save_job(&self.inner.journal, job)
    }
    pub fn shutdown(&self) {
        self.inner.screenshots.shutdown();
        self.live_shutdown();
        let queued = {
            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            state.shutting_down = true;
            for c in state.cancel.values() {
                c.store(true, Ordering::Release);
            }
            let queued: Vec<_> = state.queue.drain(..).collect();
            let queued = queued
                .into_iter()
                .filter_map(|id| state.cancel.get(&id).cloned().map(|cancel| (id, cancel)))
                .collect::<Vec<_>>();
            self.inner.ready.notify_all();
            queued
        };
        for (id, cancel) in queued {
            let _owner = lifecycle::JobOwnership::new(self.clone(), id.clone(), cancel.clone());
            self.terminal_error(&id, &cancel, "앱 종료로 대기 작업이 중단됐습니다.");
        }
        if let Ok(mut playback) = self.inner.playback.lock() {
            playback.clear();
        }
        replay_chat::release_all();
    }
    fn update(&self, id: &str, apply: impl FnOnce(&mut MediaJob)) -> Result<(), String> {
        let mut state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
        let mut job = state
            .jobs
            .get(id)
            .ok_or("작업을 찾을 수 없습니다.")?
            .clone();
        apply(&mut job);
        job.revision += 1;
        storage::save_job(&self.inner.journal, &job)?;
        state.persistence_errors.remove(id);
        state.jobs.insert(id.to_string(), job);
        Ok(())
    }
    fn worker(&self) {
        loop {
            let next = {
                let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if state.shutting_down {
                        return;
                    }
                    if let Some(id) = state.queue.pop_front() {
                        if state.jobs.get(&id).is_some_and(|j| j.state == "queued") {
                            break Some((state.jobs[&id].clone(), state.cancel[&id].clone()));
                        }
                        state.cancel.remove(&id);
                        continue;
                    }
                    state = self
                        .inner
                        .ready
                        .wait(state)
                        .unwrap_or_else(|e| e.into_inner());
                }
            };
            if let Some((job, cancel)) = next {
                let _owner =
                    lifecycle::JobOwnership::new(self.clone(), job.job_id.clone(), cancel.clone());
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.execute(&job, &cancel)
                }))
                .unwrap_or_else(|_| {
                    Err("영상 처리 작업이 중단됐습니다. 원본은 보존했습니다.".into())
                });
                if let Err(error) = result {
                    self.chat_aware_terminal_error(&job.job_id, &cancel, &error);
                }
            }
        }
    }
    fn chat_aware_terminal_error(&self, id: &str, cancel: &Arc<AtomicBool>, error: &str) {
        self.terminal_error(id, cancel, error);
        if error == chat_status::ERROR {
            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            lifecycle::remember_persistence_error(&mut state, id, error.into());
        }
    }
    fn tool_args(&self) -> Vec<String> {
        [
            "--ignore-config",
            "--no-plugin-dirs",
            "--no-cache-dir",
            "--no-mark-watched",
            "--no-remote-components",
            "--no-js-runtimes",
            "--no-playlist",
            "--socket-timeout",
            "15",
            "--retries",
            "2",
            "--extractor-retries",
            "1",
            "--fragment-retries",
            "2",
            "--concurrent-fragments",
            "1",
            "--no-warnings",
        ]
        .into_iter()
        .map(String::from)
        .chain([
            "--js-runtimes".into(),
            format!("deno:{}", self.inner.tools.deno.display()),
            "--ffmpeg-location".into(),
            self.inner.tools.ffmpeg.to_string_lossy().into_owned(),
        ])
        .collect()
    }
    fn execute(&self, job: &MediaJob, cancel: &Arc<AtomicBool>) -> Result<(), String> {
        if job.reference.kind == "live" {
            return self.resume_live_recording(job, cancel);
        }
        let _network = self.acquire_network(cancel)?;
        // Check the pinned executable contents at use time, not merely when packaging.
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
            if storage::hash(tool, cancel)? != expected {
                return Err("영상 도구 검증에 실패했습니다. 앱을 다시 설치해 주세요.".into());
            }
        }
        let id = &job.job_id;
        let directory = storage::job_dir(&job.download_root, &job.reference.provider, id)?;
        fs::create_dir_all(&directory).map_err(|_| "저장 폴더를 만들지 못했습니다.")?;
        storage::safe_path(&directory)?;
        self.update(id, |j| j.state = "resolving".into())?;
        let mut args = self.tool_args();
        args.extend([
            "--simulate".into(),
            "--dump-single-json".into(),
            "--".into(),
            job.reference.canonical_url()?,
        ]);
        let result = process::run(
            &self.inner.tools.yt_dlp,
            &args,
            &directory,
            cancel,
            Duration::from_secs(90),
            4 * 1024 * 1024,
        )?;
        if !result.success {
            return Err(classify_failure(&result.stderr));
        }
        let metadata: serde_json::Value =
            serde_json::from_slice(&result.stdout).map_err(|_| "영상 정보를 읽지 못했습니다.")?;
        if metadata["is_live"].as_bool() == Some(true)
            || metadata["live_status"].as_str() == Some("is_upcoming")
        {
            return Err("라이브를 재생한 뒤 녹화 버튼을 눌러 주세요.".into());
        }
        let soop_plan = if job.reference.provider == "soop" {
            Some(soop_vod::bind(
                &job.reference.external_id,
                soop_vod::fetch(&job.reference.external_id, cancel)?,
                &metadata,
            )?)
        } else { None };
        if soop_plan.is_none() && metadata["id"].as_str() != Some(&job.reference.external_id) {
            return Err("요청한 영상과 저장 대상이 다릅니다.".into());
        }
        let title = clean(metadata["title"].as_str(), 300);
        let channel = clean(
            metadata["channel"]
                .as_str()
                .or(metadata["uploader"].as_str()),
            160,
        );
        let expected_duration = soop_plan.as_ref().map(|plan| plan.duration)
            .unwrap_or_else(|| metadata["duration"].as_f64().unwrap_or(0.0));
        if expected_duration <= 0.0 || expected_duration > 24.0 * 3600.0 {
            return Err("영상 길이를 확인하지 못했거나 저장 한도를 넘었습니다.".into());
        }
        self.update(id, |j| {
            j.title = title;
            j.channel_name = channel;
            j.state = "downloading".into();
        })?;
        let mut args = self.tool_args();
        args.extend(["--no-progress","--no-part","--no-overwrites","--keep-video","--no-write-info-json","--no-write-thumbnail","--no-write-subs","--no-write-auto-subs","--max-filesize","50G","--format","bv*[ext=mp4][vcodec^=avc1][height<=1080]+ba[ext=m4a]/b[ext=mp4][height<=1080]/b[ext=mp4]","--merge-output-format","mp4"].into_iter().map(String::from));
        if let Some(plan) = &soop_plan { args.extend(plan.output_args()); }
        else { args.extend(["--output", "download.%(ext)s"].map(String::from)); }
        args.push("--".into());
        args.push(job.reference.canonical_url()?);
        let result = process::run(
            &self.inner.tools.yt_dlp,
            &args,
            &directory,
            cancel,
            Duration::from_secs(24 * 3600),
            65536,
        )?;
        if !result.success {
            return Err(classify_failure(&result.stderr));
        }
        if let Some(plan) = &soop_plan {
            plan.verify_receipt(&result.stdout)?;
            if plan.has_chat() {
                soop_vod_chat::download_with_metadata(job, &directory, cancel, &plan.metadata)?;
            }
        }
        self.verify_publish(job, cancel, &directory, Some(expected_duration))
    }
    fn verify_publish(
        &self,
        job: &MediaJob,
        cancel: &Arc<AtomicBool>,
        directory: &std::path::Path,
        expected_duration: Option<f64>,
    ) -> Result<(), String> {
        self.verify_publish_from(job, cancel, directory, expected_duration, "download.mp4")
    }
    // Completion is the cancellation linearization point. Unlike progress
    // updates, it must hold the same state lock as cancel/shutdown through the
    // durable journal write; a revoked worker must never reach input cleanup.
    fn commit_verified_completion(
        &self,
        expected: &MediaJob,
        cancel: &Arc<AtomicBool>,
        library: impl FnOnce(&MediaJob, u64) -> LibraryItem,
    ) -> Result<(), String> {
        const CHANGED: &str = "영상 마무리 상태가 변경됐습니다. 원본은 보존했습니다.";
        let mut state = self.inner.state.lock().map_err(|_| CHANGED)?;
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        let current = state.jobs.get(&expected.job_id).ok_or(CHANGED)?;
        if state.shutting_down
            || !state.cancel.get(&expected.job_id).is_some_and(|known| Arc::ptr_eq(known, cancel))
            || !current.active() || current.state != "finalizing" || current.library.is_some()
            || current.job_id != expected.job_id || current.request_id != expected.request_id
            || current.reference != expected.reference || current.broadcast_id != expected.broadcast_id
            || current.download_root != expected.download_root || current.created_at != expected.created_at
        {
            return Err(CHANGED.into());
        }
        let revision = current.revision.checked_add(1).ok_or(CHANGED)?;
        let mut next = current.clone();
        next.library = Some(library(current, revision));
        next.state = "completed".into();
        next.error = None;
        next.revision = revision;
        storage::save_job(&self.inner.journal, &next)?;
        state.persistence_errors.remove(&expected.job_id);
        state.jobs.insert(expected.job_id.clone(), next);
        Ok(())
    }
    fn verify_publish_from(
        &self,
        job: &MediaJob,
        cancel: &Arc<AtomicBool>,
        directory: &std::path::Path,
        expected_duration: Option<f64>,
        input_name: &str,
    ) -> Result<(), String> {
        let id = &job.job_id;
        let capture_outcome = encoded::soop_finalize::outcome(job, directory, cancel)?;
        if input_name != "download.mp4"
            && input_name != "media.mp4"
            && !(input_name.starts_with("merge-")
                && input_name.ends_with(".mp4")
                && uuid::Uuid::parse_str(&input_name[6..input_name.len() - 4]).is_ok())
        {
            return Err("검증 파일 경로가 올바르지 않습니다.".into());
        }
        let output = directory.join(input_name);
        let bytes = storage::regular(&output)?.len();
        if bytes < 1024 {
            return Err("저장 영상이 비어 있습니다.".into());
        }
        self.update(id, |j| {
            j.state = "verifying".into();
            j.received_bytes = bytes;
            j.total_bytes = Some(bytes);
        })?;
        let _verification = loop {
            if cancel.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            if let Ok(lock) = self.inner.verify.try_lock() {
                break lock;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let probe = process::run(
            &self.inner.tools.ffprobe,
            &[
                "-v",
                "error",
                "-show_entries",
                "format=duration:stream=codec_type",
                "-of",
                "json",
                input_name,
            ]
            .map(String::from),
            &directory,
            cancel,
            Duration::from_secs(30),
            65536,
        )?;
        if !probe.success {
            return Err("저장 파일 정보를 검증하지 못했습니다.".into());
        }
        let info: serde_json::Value = serde_json::from_slice(&probe.stdout)
            .map_err(|_| "저장 파일 정보를 확인하지 못했습니다.")?;
        let duration = info["format"]["duration"]
            .as_str()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);
        if !duration.is_finite()
            || duration < 1.0
            || expected_duration
                .is_some_and(|expected| (duration - expected).abs() > expected.mul_add(0.02, 3.0))
            || !info["streams"]
                .as_array()
                .is_some_and(|s| s.iter().any(|v| v["codec_type"] == "video"))
        {
            return Err("저장 영상의 길이 또는 영상 스트림이 올바르지 않습니다.".into());
        }
        let decoded = process::run(
            &self.inner.tools.ffmpeg,
            &[
                "-hide_banner",
                "-nostdin",
                "-v",
                "error",
                "-xerror",
                "-threads",
                "2",
                "-protocol_whitelist",
                "file,pipe",
                "-i",
                input_name,
                "-map",
                "0:v:0",
                "-map",
                "0:a?",
                "-f",
                "null",
                "-",
            ]
            .map(String::from),
            &directory,
            cancel,
            Duration::from_secs(24 * 3600),
            1024,
        )?;
        if !decoded.success {
            return Err("영상 검증에 실패했습니다. 원본은 보존했습니다.".into());
        }
        let sha256 = storage::hash(&output, cancel)?;
        self.update(id, |j| j.state = "finalizing".into())?;
        let final_path = directory.join("media.mp4");
        if input_name != "media.mp4" {
            if final_path.exists() {
                return Err("완성 파일이 이미 있습니다. 덮어쓰지 않았습니다.".into());
            }
            fs::rename(&output, &final_path).map_err(|_| "완성 파일을 보관하지 못했습니다.")?;
        }
        let modified_millis = storage::regular(&final_path)?
            .modified()
            .ok()
            .and_then(|v| v.duration_since(UNIX_EPOCH).ok())
            .map(|v| v.as_millis() as u64)
            .unwrap_or(0);
        let _poster = process::run(
            &self.inner.tools.ffmpeg,
            &[
                "-hide_banner",
                "-nostdin",
                "-v",
                "error",
                "-n",
                "-protocol_whitelist",
                "file,pipe",
                "-i",
                "media.mp4",
                "-frames:v",
                "1",
                "-vf",
                "scale=480:270:force_original_aspect_ratio=decrease",
                "-q:v",
                "5",
                "-threads",
                "1",
                "thumbnail.jpg",
            ]
            .map(String::from),
            &directory,
            cancel,
            Duration::from_secs(30),
            1024,
        );
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        let chat_archive = if job.reference.kind == "live" {
            Some(chat_status::recovery_with_cancel(
                directory,
                &job.reference.provider,
                id,
                cancel,
            )?)
        } else {
            chat_status::inspect(directory, &job.reference.provider, id)?
        };
        self.commit_verified_completion(job, cancel, |j, revision| {
            LibraryItem {
                broadcast_id: j.broadcast_id.clone(),
                chat_archive: chat_archive.clone(),
                capture_outcome: capture_outcome.clone(),
                library_id: id.clone(),
                revision,
                reference: j.reference.clone(),
                title: j.title.clone(),
                channel_name: j.channel_name.clone(),
                bytes,
                duration,
                sha256,
                completed_at: now(),
                modified_millis,
            }
        })?;
        // Keep merger inputs until the completed proof is durable. Only predictable extractor
        // filenames in this UUID job folder are eligible; failed/unverified inputs stay intact.
        if capture_outcome.is_some() || chat_archive
            .as_ref()
            .is_some_and(|status| status.originals_retained)
        {
            return Ok(());
        }
        if let Ok(entries) = fs::read_dir(&directory) {
            for entry in entries.take(128).filter_map(Result::ok) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if (name.starts_with("download.f")
                    || (job.reference.provider == "soop" && soop_vod::owned_intermediate(&name)))
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
                    && matches!(
                        entry.path().extension().and_then(|s| s.to_str()),
                        Some("mp4" | "m4a" | "webm")
                    )
                    && storage::regular(&entry.path()).is_ok()
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        Ok(())
    }
}
fn classify_failure(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    if text.contains("sign in")
        || text.contains("login")
        || text.contains("private video")
        || text.contains("age-restricted")
    {
        "로그인이 필요하거나 접근이 제한된 영상입니다."
    } else if text.contains("drm") {
        "보호된 영상은 저장할 수 없습니다."
    } else if text.contains("unavailable") || text.contains("not available") {
        "영상을 사용할 수 없습니다."
    } else if text.contains("no space") || text.contains("disk full") {
        "저장 공간이 부족합니다. 미완료 파일은 보존했습니다."
    } else if text.contains("403") || text.contains("429") {
        "서비스가 요청을 제한했습니다. 잠시 후 다시 시도해 주세요."
    } else {
        "영상을 저장하지 못했습니다. 네트워크 또는 서비스 응답을 확인해 주세요."
    }
    .into()
}
#[cfg(test)]
mod completion_fence_tests {
    use super::*;
    use std::{cell::Cell, sync::mpsc};

    struct Fixture {
        root: tempfile::TempDir,
        service: PlatformMediaService,
        job: MediaJob,
        cancel: Arc<AtomicBool>,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let never = root.path().join("never-executed.exe");
            let service = PlatformMediaService::new(root.path().join("data"), PlatformMediaTools {
                yt_dlp: never.clone(), deno: never.clone(), ffmpeg: never.clone(), ffprobe: never,
            }, None).unwrap();
            let job = MediaJob {
                broadcast_id: Some("123".into()), job_id: uuid::Uuid::new_v4().to_string(),
                request_id: uuid::Uuid::new_v4().to_string(),
                reference: MediaRef { provider: "soop".into(), kind: "live".into(), external_id: "fixture".into() },
                revision: 7, state: "finalizing".into(), title: "current title".into(),
                channel_name: "fixture channel".into(), created_at: 1,
                received_bytes: 2048, total_bytes: Some(2048), error: Some("previous warning".into()),
                library: None, download_root: root.path().join("media"),
            };
            let cancel = Arc::new(AtomicBool::new(false));
            storage::save_job(&service.inner.journal, &job).unwrap();
            {
                let mut state = service.inner.state.lock().unwrap();
                state.jobs.insert(job.job_id.clone(), job.clone());
                state.cancel.insert(job.job_id.clone(), cancel.clone());
            }
            fs::write(root.path().join("source.fragment"), b"preserved source fixture").unwrap();
            fs::write(root.path().join("chat.fixture"), b"preserved chat fixture").unwrap();
            Self { root, service, job, cancel }
        }
        fn state_bytes(&self) -> Vec<u8> {
            serde_json::to_vec(&self.service.inner.state.lock().unwrap().jobs[&self.job.job_id]).unwrap()
        }
        fn journal_bytes(&self) -> Vec<u8> {
            fs::read(self.service.inner.journal.join(format!("{}.json", self.job.job_id))).unwrap()
        }
        fn assert_sources(&self) {
            assert_eq!(fs::read(self.root.path().join("source.fragment")).unwrap(), b"preserved source fixture");
            assert_eq!(fs::read(self.root.path().join("chat.fixture")).unwrap(), b"preserved chat fixture");
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { self.service.shutdown(); } }

    // Only fixture metadata; no test treats this as media verification evidence.
    fn item(job: &MediaJob, revision: u64) -> LibraryItem {
        LibraryItem {
            broadcast_id: job.broadcast_id.clone(),
            chat_archive: Some(chat_status::ChatArchiveSummary::unknown()),
            capture_outcome: Some(capture_outcome::CaptureOutcome::soop_interrupted()),
            library_id: job.job_id.clone(), revision, reference: job.reference.clone(),
            title: job.title.clone(), channel_name: job.channel_name.clone(), bytes: 2048,
            duration: 3.0, sha256: "a".repeat(64), completed_at: 2, modified_millis: 3,
        }
    }

    #[test]
    fn cancellation_between_last_check_and_completion_preserves_journal_and_sources() {
        let fixture = Fixture::new();
        let before = fixture.state_bytes();
        let journal = fixture.journal_bytes();
        let (checked_tx, checked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let service = fixture.service.clone();
        let job = fixture.job.clone();
        let cancel = fixture.cancel.clone();
        let worker = std::thread::spawn(move || {
            assert!(!cancel.load(Ordering::Acquire)); // Former final pre-chat check.
            checked_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let published = Cell::new(false);
            let result = service.commit_verified_completion(&job, &cancel, |job, revision| {
                published.set(true);
                item(job, revision)
            });
            let reached_cleanup = result.is_ok(); // Caller uses ? before cleanup.
            (result, published.get(), reached_cleanup)
        });
        checked_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        fixture.service.cancel(&fixture.job.job_id, fixture.job.revision).unwrap();
        release_tx.send(()).unwrap();
        let (result, published, cleanup) = worker.join().unwrap();
        assert_eq!(result.unwrap_err(), "cancelled");
        assert!(!published && !cleanup);
        assert_eq!(fixture.state_bytes(), before);
        assert_eq!(fixture.journal_bytes(), journal);
        fixture.assert_sources();
    }

    #[test]
    fn revoked_or_different_attempt_never_replaces_current_or_other_job() {
        for case in ["replacement", "missing_token", "shutdown", "completed", "cancelled", "failed",
            "library", "job", "request", "reference", "broadcast", "root", "created", "overflow", "queued"] {
            let fixture = Fixture::new();
            let mut other = fixture.job.clone();
            other.job_id = uuid::Uuid::new_v4().to_string();
            other.request_id = uuid::Uuid::new_v4().to_string();
            storage::save_job(&fixture.service.inner.journal, &other).unwrap();
            let other_bytes = serde_json::to_vec(&other).unwrap();
            {
                let mut state = fixture.service.inner.state.lock().unwrap();
                state.jobs.insert(other.job_id.clone(), other.clone());
                match case {
                    "replacement" => { state.cancel.insert(fixture.job.job_id.clone(), Arc::new(AtomicBool::new(false))); }
                    "missing_token" => { state.cancel.remove(&fixture.job.job_id); }
                    "shutdown" => { state.shutting_down = true; }
                    _ => {
                        let current = state.jobs.get_mut(&fixture.job.job_id).unwrap();
                        match case {
                            "completed" | "cancelled" | "failed" | "queued" => current.state = case.into(),
                            "library" => current.library = Some(item(current, current.revision)),
                            "job" => current.job_id = uuid::Uuid::new_v4().to_string(),
                            "request" => current.request_id = uuid::Uuid::new_v4().to_string(),
                            "reference" => current.reference.external_id = "other".into(),
                            "broadcast" => current.broadcast_id = Some("456".into()),
                            "root" => current.download_root = fixture.root.path().join("other"),
                            "created" => current.created_at += 1,
                            "overflow" => current.revision = u64::MAX,
                            _ => unreachable!(),
                        }
                    }
                }
            }
            let before = fixture.state_bytes();
            let journal = fixture.journal_bytes();
            let called = Cell::new(false);
            assert!(fixture.service.commit_verified_completion(&fixture.job, &fixture.cancel, |job, revision| {
                called.set(true); item(job, revision)
            }).is_err(), "{case}");
            assert!(!called.get(), "{case}");
            assert_eq!(fixture.state_bytes(), before, "{case}");
            assert_eq!(fixture.journal_bytes(), journal, "{case}");
            assert_eq!(serde_json::to_vec(&fixture.service.inner.state.lock().unwrap().jobs[&other.job_id]).unwrap(), other_bytes);
            assert_eq!(fs::read(fixture.service.inner.journal.join(format!("{}.json", other.job_id))).unwrap(), other_bytes);
            fixture.assert_sources();
        }
    }

    #[test]
    fn successful_commit_preserves_current_metadata_and_partial_truth_once() {
        let fixture = Fixture::new();
        fixture.service.update(&fixture.job.job_id, |job| job.title = "newer resolved title".into()).unwrap();
        fixture.service.commit_verified_completion(&fixture.job, &fixture.cancel, item).unwrap();
        let completed = fixture.service.jobs(None).into_iter().find(|job| job.job_id == fixture.job.job_id).unwrap();
        assert_eq!(completed.state, "completed");
        assert_eq!(completed.revision, 9);
        assert!(completed.error.is_none());
        let saved = completed.library.as_ref().unwrap();
        assert_eq!(saved.revision, completed.revision);
        assert_eq!(saved.title, "newer resolved title");
        assert_eq!(serde_json::to_value(saved.chat_archive.as_ref().unwrap()).unwrap()["state"], "unknown");
        assert_eq!(serde_json::to_value(saved.capture_outcome.as_ref().unwrap()).unwrap()["state"], "interrupted");
        let before = fixture.state_bytes();
        assert_eq!(fixture.journal_bytes(), before);
        fixture.service.cancel(&fixture.job.job_id, completed.revision).unwrap();
        assert!(!fixture.cancel.load(Ordering::Acquire));
        assert!(fixture.service.commit_verified_completion(&fixture.job, &fixture.cancel, |_, _| panic!("duplicate completion")).is_err());
        assert_eq!(fixture.state_bytes(), before);
        assert_eq!(fixture.journal_bytes(), before);
        fixture.assert_sources();
    }

    #[test]
    fn completion_holds_state_lock_until_durable_commit_before_later_cancel() {
        let fixture = Fixture::new();
        let (inside_tx, inside_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let service = fixture.service.clone();
        let job = fixture.job.clone();
        let cancel = fixture.cancel.clone();
        let worker = std::thread::spawn(move || service.commit_verified_completion(&job, &cancel, |job, revision| {
            inside_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            item(job, revision)
        }));
        inside_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(fixture.service.inner.state.try_lock(), Err(std::sync::TryLockError::WouldBlock)));
        release_tx.send(()).unwrap();
        worker.join().unwrap().unwrap();
        let completed = fixture.service.jobs(None).pop().unwrap();
        fixture.service.cancel(&fixture.job.job_id, completed.revision).unwrap();
        assert_eq!(completed.state, "completed");
        assert!(!fixture.cancel.load(Ordering::Acquire));
        assert_eq!(fixture.journal_bytes(), fixture.state_bytes());
    }

    #[test]
    fn failed_journal_write_keeps_previous_state_and_never_reaches_cleanup() {
        let fixture = Fixture::new();
        let before = fixture.state_bytes();
        let old_journal = fixture.journal_bytes();
        let preserved = fixture.root.path().join("preserved-journal");
        fs::rename(&fixture.service.inner.journal, &preserved).unwrap();
        fs::write(&fixture.service.inner.journal, b"synthetic IO blocker").unwrap();
        let result = fixture.service.commit_verified_completion(&fixture.job, &fixture.cancel, item);
        assert!(result.is_err());
        assert_eq!(fixture.state_bytes(), before);
        assert_eq!(fs::read(preserved.join(format!("{}.json", fixture.job.job_id))).unwrap(), old_journal);
        fixture.assert_sources();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chat_proof_failure_stays_latched_after_terminal_journal_success() {
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
        let id = uuid::Uuid::new_v4().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        let job = MediaJob {
            broadcast_id: None,
            job_id: id.clone(),
            request_id: uuid::Uuid::new_v4().to_string(),
            reference: MediaRef {
                provider: "soop".into(),
                kind: "live".into(),
                external_id: "fixture".into(),
            },
            revision: 1,
            state: "recording".into(),
            title: String::new(),
            channel_name: String::new(),
            created_at: 0,
            received_bytes: 0,
            total_bytes: None,
            error: None,
            library: None,
            download_root: root.path().join("media"),
        };
        {
            let mut state = service.inner.state.lock().unwrap();
            state.jobs.insert(id.clone(), job);
            state.cancel.insert(id.clone(), cancel.clone());
        }
        let owner = lifecycle::JobOwnership::new(service.clone(), id.clone(), cancel.clone());
        service.chat_aware_terminal_error(&id, &cancel, chat_status::ERROR);
        assert_eq!(service.jobs(None)[0].state, "failed");
        assert!(service.jobs(None)[0].library.is_none());
        assert!(service.shutdown_error().is_some());
        drop(owner);
        assert!(service.active_ids().is_empty());
        assert!(service.shutdown_error().is_some());
        service.shutdown();
    }
    #[test]
    fn refs_are_provider_scoped() {
        assert!(MediaRef {
            provider: "youtube".into(),
            kind: "video".into(),
            external_id: "M7lc1UVf-VE".into()
        }
        .canonical_url()
        .is_ok());
        for id in ["../../secret", "https://localhost/", "M7lc1UVf-VE?x"] {
            assert!(MediaRef {
                provider: "youtube".into(),
                kind: "video".into(),
                external_id: id.into()
            }
            .canonical_url()
            .is_err());
        }
    }
    #[test]
    fn errors_do_not_disclose_urls() {
        assert!(!classify_failure(b"403 https://secret.invalid/?token=x").contains("secret"));
    }
    #[test]
    #[ignore = "explicit anonymous public sample; isolated .runtime output only"]
    fn public_youtube_vod_roundtrip() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let root = repo
            .join(".runtime/platform-media-tests")
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
        let job = service
            .download(
                MediaRef {
                    provider: "youtube".into(),
                    kind: "video".into(),
                    external_id: "jNQXAC9IVRw".into(),
                },
                uuid::Uuid::new_v4().to_string(),
                root.join("media"),
            )
            .unwrap();
        let started = std::time::Instant::now();
        let final_job = loop {
            let jobs = service.jobs(Some("youtube"));
            let item = jobs.iter().find(|j| j.job_id == job.job_id).unwrap();
            if !item.active() {
                break item.clone();
            }
            if started.elapsed() > Duration::from_secs(180) {
                service.shutdown();
                panic!("bounded isolated sample timed out");
            }
            std::thread::sleep(Duration::from_millis(250));
        };
        if final_job.state != "completed" {
            service.shutdown();
            panic!("sample failed safely: {:?}", final_job.error);
        }
        let opened = service.open(&job.job_id).unwrap();
        let response = protocol::respond(
            &service,
            "main",
            tauri::http::Request::builder()
                .uri(format!(
                    "http://platform-media.localhost/{}/media.mp4",
                    opened.token
                ))
                .header("Range", "bytes=0-1023")
                .body(Vec::new())
                .unwrap(),
        );
        assert_eq!(response.status(), 206);
        assert_eq!(response.body().len(), 1024);
        assert!(service.delete(&job.job_id, opened.item.revision).is_err());
        service.close(&opened.token);
        service.shutdown();
        eprintln!(
            "isolated VOD verified: duration={} bytes={} sha256={} root={}",
            opened.item.duration,
            opened.item.bytes,
            opened.item.sha256,
            root.display()
        );
        assert!(opened.item.duration > 10.0);
        assert_eq!(opened.item.sha256.len(), 64);
    }
}
