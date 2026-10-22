//! Optional display metadata, separate from the protected login/profile registry.
//! Identity is supplied only by a current, owner-proven native X document.
use super::{accounts, avatar_url, image_data, valid_handle, XSession};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};

const FILE: &str = "presentation.json";
const MAX_FILE: u64 = 2 * 1024 * 1024;
const MAX_IMAGE: u64 = 2 * 1024 * 1024;
const RETRY: Duration = Duration::from_secs(60);
const DEFAULT_AVATAR: &str = "https://abs.twimg.com/sticky/default_profile_images/default_profile_normal.png";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ObservedPresentation {
    handle: String,
    name: String,
    avatar: Option<String>,
    #[serde(default, rename = "avatarCandidates")]
    avatar_candidates: Vec<AvatarCandidate>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AvatarCandidate {
    url: String,
    width: u32,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Presentation {
    handle: String,
    name: String,
    avatar_data: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    presentation: Presentation,
    source: Option<String>,
}

struct Attempt {
    source: String,
    revision: u64,
    started: Instant,
    nonce: u64,
}

#[derive(Default)]
pub(super) struct Store {
    loaded: bool,
    entries: HashMap<String, Saved>,
    attempts: HashMap<String, Attempt>,
    nonce: u64,
}

fn valid_name(value: &str) -> bool {
    value.chars().count() <= 120 && !value.chars().any(char::is_control)
}

fn account_avatar_url(value: &str) -> Result<tauri::Url, String> {
    if value == DEFAULT_AVATAR {
        return tauri::Url::parse(value).map_err(|_| "X 프로필 이미지 주소 오류".into());
    }
    avatar_url(value)
}

// Compare only known rendition suffixes of the same already-observed resource.
// This key is never used to look up another account.
fn avatar_resource(value: &str) -> Option<(String, u32)> {
    let url = account_avatar_url(value).ok()?;
    if value == DEFAULT_AVATAR { return Some((value.to_owned(), 48)); }
    let (stem, extension) = url.path().rsplit_once('.')?;
    let (stem, size) = [("_mini", 24), ("_normal", 48), ("_bigger", 73), ("_200x200", 200), ("_400x400", 400)]
        .into_iter().find_map(|(suffix, size)| stem.strip_suffix(suffix).map(|base| (base, size)))
        .unwrap_or((stem, 0));
    Some((format!("{stem}.{extension}"), size))
}

fn same_avatar(left: &str, right: &str) -> bool {
    avatar_resource(left).zip(avatar_resource(right)).is_some_and(|(left, right)| left.0 == right.0)
}

// Explicit user exception: ONLY the own-account switcher display cache may
// request this same-image rendition. Other avatar/media consumers do not call
// this helper; default images and unknown suffixes are never rewritten.
fn own_switcher_rendition(value: &str) -> Option<String> {
    let mut url = account_avatar_url(value).ok()?;
    if url.host_str() != Some("pbs.twimg.com") { return None; }
    let (stem, extension) = url.path().rsplit_once('.')?;
    let stem = ["_mini", "_normal", "_bigger"].into_iter().find_map(|suffix| stem.strip_suffix(suffix))?;
    if stem.ends_with('/') { return None; }
    let path = format!("{stem}_400x400.{extension}");
    url.set_path(&path);
    let result = String::from(url);
    same_avatar(value, &result).then_some(result)
}

fn validate_observation(mut value: ObservedPresentation, owner: &str) -> Option<ObservedPresentation> {
    if !valid_handle(&value.handle)
        || !value.handle.eq_ignore_ascii_case(owner)
        || !valid_name(&value.name)
    {
        return None;
    }
    value.handle = owner.to_ascii_lowercase();
    value.name = value.name.trim().to_owned();
    value.avatar = match value.avatar {
        Some(url) => Some(String::from(account_avatar_url(&url).ok()?)),
        None => None,
    };
    if value.avatar_candidates.len() > 8 { return None; }
    let base = value.avatar.as_deref();
    value.avatar_candidates.retain(|candidate| candidate.width <= 8192
        && base.is_some_and(|base| same_avatar(base, &candidate.url)));
    Some(value)
}

// Persisted bytes are untrusted. Only our small JPEG display copy is accepted;
// never return arbitrary data URLs, SVG, remote URLs or a large decoder input.
fn avatar_data_size(value: &str) -> Option<u32> {
    if value.len() > 64 * 1024 {
        return None;
    }
    let body = value.strip_prefix("data:image/jpeg;base64,")?;
    let bytes = STANDARD.decode(body).ok()?;
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(96);
    limits.max_image_height = Some(96);
    limits.max_alloc = Some(1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    (decoded.width() > 0 && decoded.height() > 0 && decoded.width() <= 96 && decoded.height() <= 96)
        .then_some(decoded.width().min(decoded.height()))
}

fn valid_avatar_data(value: &str) -> bool {
    avatar_data_size(value).is_some()
}

fn prepare_portrait(previous: Option<&Saved>, observed: ObservedPresentation) -> (Saved, Option<String>) {
    let previous = previous.filter(|saved| saved.presentation.handle.eq_ignore_ascii_case(&observed.handle));
    let base = observed.avatar.as_deref().or_else(|| previous.and_then(|saved| saved.source.as_deref()));
    let retained = previous.filter(|saved| saved.source.as_deref().zip(base).is_some_and(|(old, new)| same_avatar(old, new)));
    let old_data = retained.and_then(|saved| saved.presentation.avatar_data.clone());
    let old_size = old_data.as_deref().and_then(avatar_data_size).unwrap_or(0);
    let candidate = base.map(|base| {
        let base_size = avatar_resource(base).map(|(_, size)| size).unwrap_or(0);
        observed.avatar_candidates.iter().filter(|candidate| same_avatar(base, &candidate.url))
            .map(|candidate| (candidate.url.as_str(), candidate.width.max(avatar_resource(&candidate.url).map(|(_, size)| size).unwrap_or(0))))
            .chain(std::iter::once((base, base_size))).max_by_key(|(_, size)| *size).unwrap()
    });
    let observed_upgrade = candidate.filter(|(url, size)| old_data.is_none() || (old_size < 96 && *size > old_size
        && retained.and_then(|saved| saved.source.as_deref()) != Some(*url))).map(|(url, _)| url.to_owned());
    let rendition = base.and_then(own_switcher_rendition).filter(|_| old_size < 96);
    let fetch = match observed_upgrade {
        // Actual larger observed renditions win. Without one, try only the
        // explicitly authorized same-image suffix before the original image.
        Some(url) if base != Some(url.as_str()) => Some(url),
        original => rendition.or(original),
    };
    // Keep a successful same-photo display copy while a larger candidate loads.
    // On failure neither its source nor its bytes is relabelled as the new URL.
    let source = if old_data.is_some() { retained.and_then(|saved| saved.source.clone()) }
        else { base.map(str::to_owned) };
    let name = if observed.name.is_empty() { previous.map(|saved| saved.presentation.name.clone()).unwrap_or_default() }
        else { observed.name };
    (Saved { presentation: Presentation { handle: observed.handle, name, avatar_data: old_data }, source }, fetch)
}

fn finish_portrait(saved: &mut Saved, source: &str, data: String) -> bool {
    if !saved.source.as_deref().is_some_and(|current| same_avatar(current, source)) { return false; }
    let Some(size) = avatar_data_size(&data) else { return false };
    if saved.presentation.avatar_data.as_deref().and_then(avatar_data_size).is_some_and(|old| old >= size) { return false; }
    saved.source = Some(source.to_owned());
    saved.presentation.avatar_data = Some(data);
    true
}

fn fetch_portrait(
    source: &str, observed_base: Option<&str>, original_fallback: bool,
    mut load: impl FnMut(&str) -> Result<String, String>,
) -> Result<(String, String), String> {
    let mut candidates = vec![source.to_owned()];
    if let Some(base) = observed_base.filter(|base| same_avatar(source, base)) {
        if let Some(larger) = own_switcher_rendition(base).filter(|url| !candidates.contains(url)) { candidates.push(larger); }
        if original_fallback && !candidates.iter().any(|url| url == base) { candidates.push(base.to_owned()); }
    }
    // At most one observed choice, one authorized rendition and the observed
    // original. No retry loop, redirects, cookies, general URL guessing or
    // synthetic pixel upscaling. Retain a successful smaller result if a
    // later, larger candidate fails.
    let mut best = None;
    let mut best_size = 0;
    for candidate in candidates.into_iter().take(3) {
        let Ok(data) = load(&candidate) else { continue };
        let Some(size) = avatar_data_size(&data) else { continue };
        if size > best_size { best_size = size; best = Some((candidate, data)); }
        if best_size >= 96 { break; }
    }
    best.ok_or_else(|| "X 프로필 이미지 표시 형식 오류".into())
}

fn valid_saved(profile: &str, saved: &Saved) -> bool {
    accounts::valid_profile(profile)
        && valid_handle(&saved.presentation.handle)
        && valid_name(&saved.presentation.name)
        && saved.source.as_deref().is_none_or(|url| account_avatar_url(url).is_ok())
        && saved.presentation.avatar_data.as_deref().is_none_or(valid_avatar_data)
        && (saved.presentation.avatar_data.is_none() || saved.source.is_some())
}

impl Store {
    fn load(&mut self, app: &AppHandle) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        // This is only a display cache. Invalid/missing cache never breaks login
        // or overwrites the independent existing accounts.json registry.
        let loaded = (|| {
            let mut file = std::fs::File::open(accounts::root(app).ok()?.join(FILE)).ok()?;
            if file.metadata().ok()?.len() > MAX_FILE { return None; }
            let mut bytes = Vec::new();
            (&mut file).take(MAX_FILE + 1).read_to_end(&mut bytes).ok()?;
            if bytes.len() as u64 > MAX_FILE { return None; }
            let entries: HashMap<String, Saved> = serde_json::from_slice(&bytes).ok()?;
            (entries.len() <= 20 && entries.iter().all(|(id, saved)| valid_saved(id, saved)))
                .then_some(entries)
        })();
        if let Some(entries) = loaded {
            self.entries = entries;
        }
    }

    fn save(&self, app: &AppHandle) -> Result<(), String> {
        let directory = accounts::root(app)?;
        std::fs::create_dir_all(&directory).map_err(|_| "X 표시 정보 저장 실패")?;
        let bytes = serde_json::to_vec(&self.entries).map_err(|_| "X 표시 정보 변환 실패")?;
        if bytes.len() as u64 > MAX_FILE { return Err("X 표시 정보 크기 제한".into()); }
        let temporary = directory.join(format!("presentation-{}.tmp", uuid::Uuid::new_v4().simple()));
        let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(&temporary)
            .map_err(|_| "X 표시 정보 저장 실패")?;
        file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|_| "X 표시 정보 저장 실패")?;
        drop(file);
        accounts::replace(&temporary, &directory.join(FILE)).map_err(|_| "X 표시 정보 저장 실패".into())
    }

    fn matches_attempt(&self, profile: &str, revision: u64, nonce: u64) -> bool {
        self.attempts.get(profile).is_some_and(|attempt| attempt.revision == revision && attempt.nonce == nonce)
    }
}

fn owner_matches(registry: &accounts::Registry, profile: &str, owner: &str) -> bool {
    registry.active == profile
        && registry.accounts.iter().any(|account| account.id == profile && account.handle.eq_ignore_ascii_case(owner))
}

pub(super) fn snapshot(session: &XSession, app: &AppHandle, registry: &accounts::Registry) -> HashMap<String, Presentation> {
    let mut store = session.presentations.lock().unwrap_or_else(|p| p.into_inner());
    store.load(app);
    registry.accounts.iter().filter_map(|account| {
        store.entries.get(&account.id)
            .filter(|saved| saved.presentation.handle.eq_ignore_ascii_case(&account.handle))
            .map(|saved| (account.id.clone(), saved.presentation.clone()))
    }).collect()
}

/// Call only for status=ready, with current exact profile-link + account-switcher
/// proof. Revision, selected profile, and handle are independently checked here.
pub(super) fn observe(
    session: &XSession, app: &AppHandle, profile: &str, owner: &str, revision: u64,
    observed: ObservedPresentation,
) {
    let Some(observed) = validate_observation(observed, owner) else { return };
    let registry = session.registry.lock().unwrap_or_else(|p| p.into_inner());
    let Some(registry) = registry.as_ref().filter(|registry| owner_matches(registry, profile, owner)) else { return };
    if session.revision.load(Ordering::Acquire) != revision { return; }
    let mut store = session.presentations.lock().unwrap_or_else(|p| p.into_inner());
    store.load(app);
    store.entries.retain(|id, saved| registry.accounts.iter().any(|account| {
        account.id == *id && account.handle.eq_ignore_ascii_case(&saved.presentation.handle)
    }));
    let observed_base = observed.avatar.clone();
    let previous = store.entries.get(profile);
    let (saved, source) = prepare_portrait(previous, observed);
    let original_fallback = saved.presentation.avatar_data.is_none();
    let changed = previous.is_none_or(|old| old.source != saved.source
        || old.presentation.name != saved.presentation.name || old.presentation.handle != saved.presentation.handle);
    if changed {
        store.entries.insert(profile.to_owned(), saved);
        if store.save(app).is_err() { tracing::warn!("X optional account display cache could not be saved"); }
    }
    let source = source.filter(|source| !store.attempts.get(profile).is_some_and(|attempt| {
        attempt.source == *source && attempt.revision == revision && attempt.started.elapsed() < RETRY
    }));
    if let Some(source) = source {
        store.nonce = store.nonce.wrapping_add(1);
        let nonce = store.nonce;
        store.attempts.insert(profile.to_owned(), Attempt { source: source.clone(), revision, started: Instant::now(), nonce });
        let host = session.clone();
        let app = app.clone();
        let profile = profile.to_owned();
        let owner = owner.to_owned();
        tauri::async_runtime::spawn(async move {
            let Ok(_permit) = host.image_slots.acquire().await else { return };
            if host.revision.load(Ordering::Acquire) != revision { return; }
            let result = tauri::async_runtime::spawn_blocking(move || fetch_portrait(&source, observed_base.as_deref(), original_fallback, fetch_avatar)).await;
            let Ok(Ok((source, avatar_data))) = result else { return };
            let registry = host.registry.lock().unwrap_or_else(|p| p.into_inner());
            if host.revision.load(Ordering::Acquire) != revision
                || !registry.as_ref().is_some_and(|saved| owner_matches(saved, &profile, &owner)) { return; }
            let mut store = host.presentations.lock().unwrap_or_else(|p| p.into_inner());
            if !store.matches_attempt(&profile, revision, nonce) { return; }
            let Some(saved) = store.entries.get_mut(&profile) else { return };
            if !saved.presentation.handle.eq_ignore_ascii_case(&owner) { return; }
            if !finish_portrait(saved, &source, avatar_data) { return; }
            if store.save(&app).is_err() { tracing::warn!("X optional account portrait cache could not be saved"); }
            drop(store);
            drop(registry);
            let _ = app.emit("x:surface-changed", serde_json::json!({"accountPresentation":true}));
        });
    }
}

fn fetch_avatar(source: &str) -> Result<String, String> {
    let url = account_avatar_url(source)?;
    let response = reqwest::blocking::Client::builder()
        .user_agent("Atsumi/2.0").timeout(Duration::from_secs(12)).redirect(reqwest::redirect::Policy::none())
        .build().map_err(|_| "X 프로필 이미지 연결 실패")?
        .get(url).header(reqwest::header::ACCEPT, "image/jpeg,image/png,image/webp")
        .header(reqwest::header::REFERER, "https://x.com/").send()
        .map_err(|_| "X 프로필 이미지 연결 실패")?;
    if !response.status().is_success() || response.content_length().is_some_and(|length| length > MAX_IMAGE) {
        return Err("X 프로필 이미지 응답 오류".into());
    }
    let mut bytes = Vec::new();
    response.take(MAX_IMAGE + 1).read_to_end(&mut bytes).map_err(|_| "X 프로필 이미지 읽기 실패")?;
    if bytes.len() as u64 > MAX_IMAGE { return Err("X 프로필 이미지 크기 제한".into()); }
    let result = image_data(&bytes, true)?;
    if valid_avatar_data(&result) { Ok(result) } else { Err("X 프로필 이미지 표시 형식 오류".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation(handle: &str, avatar: &str) -> ObservedPresentation {
        ObservedPresentation { handle: handle.into(), name: "내 이름".into(), avatar: Some(avatar.into()), avatar_candidates: vec![] }
    }
    const AVATAR: &str = "https://pbs.twimg.com/profile_images/1234/avatar_normal.jpg";
    const MINI: &str = "https://pbs.twimg.com/profile_images/1234/avatar_mini.jpg";
    const LARGE: &str = "https://pbs.twimg.com/profile_images/1234/avatar_400x400.jpg";
    fn pixels(size: u32) -> String {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes).encode_image(&image::RgbImage::new(size, size)).unwrap();
        format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))
    }
    fn cached(source: &str, size: u32) -> Saved {
        Saved { presentation: Presentation { handle: "owner".into(), name: "Owner".into(), avatar_data: Some(pixels(size)) }, source: Some(source.into()) }
    }
    fn with_large() -> ObservedPresentation {
        let mut value = observation("owner", MINI);
        value.avatar_candidates.push(AvatarCandidate { url: LARGE.into(), width: 400 });
        validate_observation(value, "owner").unwrap()
    }
    #[test]
    fn observed_candidates_are_bounded_and_cannot_borrow_other_profile_resources() {
        let mut value = with_large();
        for url in ["https://pbs.twimg.com/profile_images/5678/avatar_400x400.jpg", "https://pbs.twimg.com/profile_images/1234/other_400x400.jpg", "https://pbs.twimg.com/profile_images/1234/avatar_400x400.png", "https://evil.test/profile_images/1234/avatar_400x400.jpg", "https://pbs.twimg.com/profile_images/1234/avatar_400x400.jpg?token=no"] {
            value.avatar_candidates.push(AvatarCandidate { url: url.into(), width: 8192 });
        }
        let validated = validate_observation(value, "owner").unwrap();
        assert_eq!(validated.avatar_candidates.len(), 1);
        assert_eq!(validated.avatar_candidates[0].url, LARGE);
        let mut excessive = with_large();
        excessive.avatar_candidates.resize(9, AvatarCandidate { url: LARGE.into(), width: 400 });
        assert!(validate_observation(excessive, "owner").is_none());
        let mut invalid_size = with_large();
        invalid_size.avatar_candidates[0].width = 8193;
        assert!(validate_observation(invalid_size, "owner").unwrap().avatar_candidates.is_empty());
    }
    #[test]
    fn same_resource_comparison_does_not_expand_the_own_switcher_exception() {
        assert!(same_avatar(MINI, LARGE));
        assert!(!same_avatar(MINI, "https://pbs.twimg.com/profile_images/1234/avatar_large.jpg"));
        assert!(!same_avatar(MINI, DEFAULT_AVATAR));
        let (_, request) = prepare_portrait(None, observation("owner", MINI));
        assert_eq!(request.as_deref(), Some(LARGE));
        let (_, request) = prepare_portrait(Some(&cached(MINI, 24)), observation("owner", MINI));
        assert_eq!(request.as_deref(), Some(LARGE));
        let unknown = "https://pbs.twimg.com/profile_images/1234/avatar_custom.jpg";
        let (_, request) = prepare_portrait(None, observation("owner", unknown));
        assert_eq!(request.as_deref(), Some(unknown));
    }
    #[test]
    fn observed_upgrade_keeps_previous_image_until_success_and_never_downgrades_it() {
        let previous = cached(MINI, 24);
        let (mut next, request) = prepare_portrait(Some(&previous), with_large());
        assert_eq!(request.as_deref(), Some(LARGE));
        assert_eq!(next.source, previous.source);
        assert_eq!(next.presentation.avatar_data, previous.presentation.avatar_data);
        assert!(finish_portrait(&mut next, LARGE, pixels(96)));
        assert_eq!(next.source.as_deref(), Some(LARGE));
        let (still_large, request) = prepare_portrait(Some(&next), observation("owner", MINI));
        assert!(request.is_none());
        assert_eq!(still_large.source.as_deref(), Some(LARGE));
        assert_eq!(avatar_data_size(still_large.presentation.avatar_data.as_deref().unwrap()), Some(96));
    }
    #[test]
    fn old_low_resolution_cache_can_upgrade_without_registry_or_cache_reset() {
        let previous: Saved = serde_json::from_value(serde_json::json!({"presentation":{"handle":"owner","name":"Owner","avatarData":pixels(24)},"source":MINI})).unwrap();
        let (next, request) = prepare_portrait(Some(&previous), with_large());
        assert!(valid_saved("legacy", &next));
        assert_eq!(request.as_deref(), Some(LARGE));
        assert_eq!(next.presentation.avatar_data, previous.presentation.avatar_data);
    }
    #[test]
    fn a_real_avatar_change_does_not_reuse_old_photograph_or_late_upgrade() {
        let previous = cached(LARGE, 96);
        let new_url = "https://pbs.twimg.com/profile_images/5678/new_mini.jpg";
        let (mut next, request) = prepare_portrait(Some(&previous), observation("owner", new_url));
        assert_eq!(request.as_deref(), Some("https://pbs.twimg.com/profile_images/5678/new_400x400.jpg"));
        assert!(next.presentation.avatar_data.is_none());
        assert!(!finish_portrait(&mut next, LARGE, pixels(96)));
        assert!(finish_portrait(&mut next, new_url, pixels(24)));
    }
    #[test]
    fn smaller_or_invalid_upgrade_response_preserves_known_good_bytes() {
        let mut previous = cached(MINI, 24);
        let original = previous.presentation.avatar_data.clone();
        assert!(!finish_portrait(&mut previous, LARGE, pixels(16)));
        assert!(!finish_portrait(&mut previous, LARGE, pixels(24)));
        assert!(!finish_portrait(&mut previous, LARGE, "data:image/jpeg;base64,YQ==".into()));
        assert!(!finish_portrait(&mut previous, LARGE, pixels(97)));
        assert_eq!(previous.presentation.avatar_data, original);
        assert_eq!(previous.source.as_deref(), Some(MINI));
    }
    #[test]
    fn missing_observation_preserves_only_the_exact_owners_optional_cache() {
        let previous = cached(LARGE, 96);
        let missing = ObservedPresentation { handle: "owner".into(), name: String::new(), avatar: None, avatar_candidates: vec![] };
        let (same, request) = prepare_portrait(Some(&previous), missing.clone());
        assert!(request.is_none());assert_eq!(same.presentation.name, "Owner");assert_eq!(same.source, previous.source);
        let (other, request) = prepare_portrait(Some(&previous), ObservedPresentation { handle: "other".into(), ..missing });
        assert!(request.is_none());assert!(other.source.is_none());assert!(other.presentation.avatar_data.is_none());
    }
    #[test]
    fn authorized_rendition_preserves_host_image_id_name_and_format_only_for_known_suffixes() {
        for suffix in ["mini", "normal", "bigger"] {
            let source = format!("https://pbs.twimg.com/profile_images/1234/avatar_{suffix}.jpg");
            assert_eq!(own_switcher_rendition(&source).as_deref(), Some(LARGE));
        }
        for source in [DEFAULT_AVATAR, LARGE, "https://pbs.twimg.com/profile_images/1234/avatar.jpg", "https://pbs.twimg.com/profile_images/1234/avatar_custom.jpg", "https://pbs.twimg.com/profile_images/1234/_mini.jpg", "https://pbs.twimg.com/profile_images/1234/avatar_mini.svg", "https://pbs.twimg.com/profile_images/1234/avatar_mini.jpg?x=1", "http://pbs.twimg.com/profile_images/1234/avatar_mini.jpg", "https://other.test/profile_images/1234/avatar_mini.jpg"] {
            assert!(own_switcher_rendition(source).is_none(), "unapproved shape");
        }
    }
    #[test]
    fn first_load_uses_the_observed_original_when_larger_rendition_fails() {
        let mut requests = Vec::new();
        let (source, data) = fetch_portrait(LARGE, Some(MINI), true, |url| {
            requests.push(url.to_owned());
            if url == MINI { Ok(pixels(24)) } else { Err("fixture unavailable".into()) }
        }).unwrap();
        assert_eq!(requests, [LARGE, MINI]);assert_eq!(source, MINI);assert_eq!(avatar_data_size(&data), Some(24));
    }
    #[test]
    fn observed_larger_candidate_wins_and_fallback_requests_are_bounded() {
        let observed = "https://pbs.twimg.com/profile_images/1234/avatar_200x200.jpg";
        let mut requests = Vec::new();
        let (source, _) = fetch_portrait(observed, Some(MINI), true, |url| { requests.push(url.to_owned());Ok(pixels(96)) }).unwrap();
        assert_eq!(source, observed);assert_eq!(requests, [observed]);
        requests.clear();
        assert!(fetch_portrait(observed, Some(MINI), true, |url| { requests.push(url.to_owned());Err("fixture unavailable".into()) }).is_err());
        assert_eq!(requests, [observed, LARGE, MINI]);
        requests.clear();
        assert!(fetch_portrait(LARGE, Some(MINI), false, |url| { requests.push(url.to_owned());Err("fixture unavailable".into()) }).is_err());
        assert_eq!(requests, [LARGE]);
    }
    #[test]
    fn a_smaller_observed_success_survives_failed_larger_upgrade_but_no_other_image_is_fetched() {
        let mut requests = Vec::new();
        let (source, data) = fetch_portrait(AVATAR, Some(MINI), false, |url| {
            requests.push(url.to_owned());if url == AVATAR { Ok(pixels(48)) } else { Err("fixture unavailable".into()) }
        }).unwrap();
        assert_eq!(source, AVATAR);assert_eq!(avatar_data_size(&data), Some(48));assert_eq!(requests, [AVATAR, LARGE]);
        requests.clear();
        let _ = fetch_portrait(LARGE, Some("https://pbs.twimg.com/profile_images/5678/other_mini.jpg"), true, |url| { requests.push(url.to_owned());Err("fixture unavailable".into()) });
        assert_eq!(requests, [LARGE]);
    }
    #[test]
    fn observation_requires_exact_owner_and_bounded_public_avatar() {
        assert!(validate_observation(observation("OwNeR", AVATAR), "owner").is_some());
        assert!(validate_observation(observation("owner", DEFAULT_AVATAR), "owner").is_some());
        assert!(validate_observation(observation("other", AVATAR), "owner").is_none());
        for source in ["https://evil.test/profile_images/1234/a.jpg", "http://pbs.twimg.com/profile_images/1234/a.jpg", "https://pbs.twimg.com/profile_images/1234/a.svg", "https://pbs.twimg.com/profile_images/1234/a.jpg?token=secret", "https://abs.twimg.com/sticky/default_profile_images/other.png", "data:image/jpeg;base64,YQ=="] {
            assert!(validate_observation(observation("owner", source), "owner").is_none());
        }
        let mut value = observation("owner", AVATAR);
        value.name = "n".repeat(121);
        assert!(validate_observation(value, "owner").is_none());
        let mut value = observation("owner", AVATAR);
        value.name = "name\nnot a name".into();
        assert!(validate_observation(value, "owner").is_none());
    }
    #[test]
    fn cached_avatar_must_be_small_raster_not_arbitrary_data_url() {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode_image(&image::RgbImage::new(16, 16)).unwrap();
        assert!(valid_avatar_data(&format!("data:image/jpeg;base64,{}", STANDARD.encode(&bytes))));
        assert!(!valid_avatar_data("data:image/svg+xml,<svg/>"));
        assert!(!valid_avatar_data("data:image/jpeg;base64,YQ=="));
        bytes.clear();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode_image(&image::RgbImage::new(97, 16)).unwrap();
        assert!(!valid_avatar_data(&format!("data:image/jpeg;base64,{}", STANDARD.encode(&bytes))));
    }
    #[test]
    fn saved_metadata_never_changes_legacy_registry_or_crosses_profile_owner() {
        let registry: accounts::Registry = serde_json::from_str(r#"{"active":"legacy","accounts":[{"id":"legacy","handle":"owner"}]}"#).unwrap();
        assert!(owner_matches(&registry, "legacy", "OWNER"));
        assert!(!owner_matches(&registry, "legacy", "other"));
        assert!(!owner_matches(&registry, &"a".repeat(32), "owner"));
        assert_eq!(serde_json::to_value(registry).unwrap()["accounts"][0].as_object().unwrap().len(), 2);
        let saved = Saved { presentation: Presentation { handle: "owner".into(), name: "Owner".into(), avatar_data: None }, source: Some(AVATAR.into()) };
        assert!(valid_saved("legacy", &saved));
        assert!(!valid_saved("../profile", &saved));
    }
    #[test]
    fn old_fetch_cannot_replace_newer_observation() {
        let mut store = Store::default();
        store.attempts.insert("legacy".into(), Attempt { source: AVATAR.into(), revision: 2, started: Instant::now(), nonce: 4 });
        assert!(store.matches_attempt("legacy", 2, 4));
        assert!(!store.matches_attempt("legacy", 1, 4));
        assert!(!store.matches_attempt("legacy", 2, 3));
        assert!(!store.matches_attempt("other", 2, 4));
    }
}
