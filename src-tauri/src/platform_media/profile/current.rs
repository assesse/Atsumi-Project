//! Explicit current-picture lookup for visible library cards. Never saves a file.
use super::Profile;
use super::super::{storage, MediaJob, PlatformMediaService};
use std::{collections::HashMap, sync::{Mutex, OnceLock, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

type Cache = HashMap<String, (Instant, Option<String>)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
const LIMIT: usize = 128;
fn fresh(at: Instant, value: &Option<String>) -> bool {
    at.elapsed() < if value.is_some() { Duration::from_secs(600) } else { Duration::from_secs(30) }
}
fn remember(cache: &mut Cache, key: String, image: Option<String>) {
    if !cache.contains_key(&key) && cache.len() >= LIMIT {
        if let Some(oldest) = cache.iter().min_by_key(|(_, (at, _))| *at).map(|(key, _)| key.clone()) { cache.remove(&oldest); }
    }
    cache.insert(key, (Instant::now(), image));
}
fn soop_video_owner(id: &str, value: &serde_json::Value) -> Option<String> {
    let title = &value["data"]["title_no"];
    let title = title.as_str().map(str::to_owned).or_else(|| title.as_u64().map(|v| v.to_string()))?;
    if value["result"].as_i64() != Some(1) || title != id || value["data"]["adult_status"] == "notLogin" { return None; }
    let owner = value["data"]["bj_id"].as_str()?;
    (!owner.is_empty() && owner.len() <= 40 && owner.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b,b'_'|b'-')))
        .then(|| owner.to_ascii_lowercase())
}
pub(super) fn lookup(job: &MediaJob, cancel: &AtomicBool) -> Option<String> {
    if cancel.load(Ordering::Acquire) { return None; }
    let value = match (job.reference.provider.as_str(), job.reference.kind.as_str()) {
        ("youtube", "live" | "video") => crate::youtube::current_video_portrait(&job.reference.external_id, cancel),
        ("soop", "live" | "video") => {
            let channel = if job.reference.kind == "live" { job.reference.external_id.clone() } else {
                let value = super::super::soop_vod::fetch(&job.reference.external_id, cancel).ok()?;
                soop_video_owner(&job.reference.external_id, &value)?
            };
            if cancel.load(Ordering::Acquire) { return None; }
            let observed = crate::soop::observe_channel(&channel).ok()?;
            if cancel.load(Ordering::Acquire) { return None; }
            observed.recording_profile_url.as_deref().and_then(crate::soop::recording_portrait)
        }
        _ => None,
    }?;
    if cancel.load(Ordering::Acquire) { return None; }
    // Reuse the same bounded decoded raster proof; do not cache arbitrary strings.
    Profile::from_data_url(&value)?.data_url().ok()
}
fn same_job(current: &MediaJob, expected: &MediaJob) -> bool {
    current.state == "completed" && current.job_id == expected.job_id && current.request_id == expected.request_id
        && current.reference == expected.reference && current.download_root == expected.download_root
        && current.revision == expected.revision
        && current.library.as_ref().zip(expected.library.as_ref()).is_some_and(|(a,b)| a.sha256 == b.sha256 && a.bytes == b.bytes && a.modified_millis == b.modified_millis)
}
impl PlatformMediaService {
    pub fn current_profile(&self, library_id: &str) -> Result<Option<String>, String> {
        let job = {
            let state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
            if state.shutting_down { return Err("앱을 종료하는 중입니다.".into()); }
            let job = state.jobs.get(library_id).filter(|job| job.state == "completed").ok_or("영상을 찾을 수 없습니다.")?;
            let item = job.library.as_ref().ok_or("영상 검증 정보가 없습니다.")?;
            storage::owned_file(job, item)?;
            job.clone()
        };
        let key = job.reference.canonical_url()?;
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        let cached = cache.lock().ok().and_then(|cache| cache.get(&key).filter(|(at,value)| fresh(*at,value)).map(|(_,value)| value.clone()));
        let image = if let Some(image) = cached { image } else {
            let image = lookup(&job, &AtomicBool::new(false));
            if let Ok(mut cache) = cache.lock() { remember(&mut cache, key, image.clone()); }
            image
        };
        // An obsolete/deleted/replaced card never obtains a result from a prior job.
        let state = self.inner.state.lock().map_err(|_| "영상 작업 상태 오류")?;
        let current = state.jobs.get(library_id).filter(|current| !state.shutting_down && same_job(current,&job)).ok_or("영상이 변경되었거나 종료 중입니다.")?;
        storage::owned_file(current, current.library.as_ref().ok_or("영상 검증 정보가 없습니다.")?)?;
        Ok(image)
    }
}

#[cfg(test)] mod tests {
    use super::*;
    fn job() -> MediaJob {
        serde_json::from_value(serde_json::json!({"jobId":"job","requestId":"request","reference":{"provider":"youtube","kind":"video","externalId":"jNQXAC9IVRw"},"revision":2,"state":"completed","title":"video","channelName":"channel","createdAt":1,"receivedBytes":1024,"totalBytes":1024,"error":null,"downloadRoot":"owned-root","library":{"libraryId":"job","revision":2,"reference":{"provider":"youtube","kind":"video","externalId":"jNQXAC9IVRw"},"title":"video","channelName":"channel","bytes":1024,"duration":19.0,"sha256":"a".repeat(64),"completedAt":1,"modifiedMillis":2}})).unwrap()
    }
    #[test] fn late_public_picture_is_bound_to_the_same_completed_owned_job() {
        let expected=job(); assert!(same_job(&expected,&expected));
        for field in 0..8 {
            let mut changed=expected.clone();
            match field {0=>changed.state="deleted".into(),1=>changed.request_id="other".into(),2=>changed.reference.external_id="abcdefghijk".into(),3=>changed.download_root="elsewhere".into(),4=>changed.revision+=1,5=>changed.library.as_mut().unwrap().sha256="b".repeat(64),6=>changed.library.as_mut().unwrap().modified_millis+=1,_=>changed.library=None}
            assert!(!same_job(&changed,&expected));
        }
    }
    #[test] fn public_vod_portrait_requires_exact_title_and_unambiguous_owner_field() {
        let value = serde_json::json!({"result":1,"data":{"title_no":123,"bj_id":"Creator_1"}});
        assert_eq!(soop_video_owner("123",&value).as_deref(),Some("creator_1"));
        assert!(soop_video_owner("124",&value).is_none());
        for value in [serde_json::json!({"result":1,"data":{"title_no":123,"user_id":"guessed"}}), serde_json::json!({"result":1,"data":{"title_no":123,"bj_id":"https://evil.invalid"}}), serde_json::json!({"result":1,"data":{"title_no":123,"bj_id":"creator","adult_status":"notLogin"}}), serde_json::json!({"result":-1,"data":{"title_no":123,"bj_id":"creator"}})] { assert!(soop_video_owner("123",&value).is_none()); }
    }
    #[test] fn current_cache_is_bounded_and_failure_is_short_lived() {
        let mut cache = Cache::new();
        for n in 0..200 { remember(&mut cache,n.to_string(),None); }
        assert_eq!(cache.len(),LIMIT);
        assert!(!fresh(Instant::now()-Duration::from_secs(31),&None));
        assert!(fresh(Instant::now()-Duration::from_secs(31),&Some("bounded".into())));
        assert!(!fresh(Instant::now()-Duration::from_secs(601),&Some("bounded".into())));
    }
}
