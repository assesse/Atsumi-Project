//! Small, anonymous channel portraits. Kept out of the live/scheduler poll path.
use super::*;
use std::{collections::HashMap, io::Cursor, sync::atomic::AtomicUsize};

const FRESH_MS: u64 = 24 * 60 * 60 * 1000;
const RETRY_MS: u64 = 60 * 1000;
const MAX_ENTRIES: usize = 160;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    channel_name: String,
    image: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    channel_id: String,
    saved_at: u64,
    profile: Profile,
}
struct Cached {
    retry_at: u64,
    value: Saved,
}
pub(super) struct Profiles {
    directory: PathBuf,
    cache: Mutex<HashMap<String, Cached>>,
    active: AtomicUsize,
}
struct Permit<'a>(&'a AtomicUsize);
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl Profiles {
    pub fn new(directory: &Path) -> Self {
        Self {
            directory: directory.into(),
            cache: Mutex::new(HashMap::new()),
            active: AtomicUsize::new(0),
        }
    }
    pub fn get(&self, channel: &str) -> Result<Profile, StreamError> {
        self.get_with(channel, now_ms(), || {
            let (channel_name, image) = ChzzkProvider::new()?.channel_profile(channel)?;
            let had_image = image.is_some();
            let image = image.and_then(|url| portrait(&url).ok());
            let complete = !had_image || image.is_some();
            Ok((
                Profile {
                    channel_name,
                    image,
                },
                complete,
            ))
        })
    }
    fn get_with(
        &self,
        channel: &str,
        now: u64,
        fetch: impl FnOnce() -> Result<(Profile, bool), StreamError>,
    ) -> Result<Profile, StreamError> {
        if normalize_channel_input(channel).ok().as_deref() != Some(channel) {
            return Err(unavailable());
        }
        if let Some(cached) = self
            .cache
            .lock()
            .map_err(|_| unavailable())?
            .get(channel)
            .filter(|entry| now < entry.retry_at)
        {
            return Ok(cached.value.profile.clone());
        }
        // Two bounded metadata/image workers at most; never hold the cache or
        // recording mutex over network I/O, and never forward login cookies.
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| unavailable())?;
        let _permit = Permit(&self.active);
        let name = format!("chzzk-live-profile-{channel}.json");
        let path = self.directory.join(&name);
        let saved = std::fs::symlink_metadata(&path)
            .ok()
            .filter(|m| m.is_file() && !m.file_type().is_symlink() && m.len() <= 256 * 1024)
            .and_then(|_| std::fs::read(&path).ok())
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
            .filter(|saved| saved.channel_id == channel && valid_profile(&saved.profile));
        let (value, retry_at) = if let Some(saved) = saved
            .as_ref()
            .filter(|s| s.saved_at <= now && now - s.saved_at < FRESH_MS)
        {
            (saved.clone(), saved.saved_at.saturating_add(FRESH_MS))
        } else {
            match fetch() {
                Ok((mut profile, complete)) => {
                    if !complete && profile.image.is_none() {
                        profile.image = saved.as_ref().and_then(|old| old.profile.image.clone());
                    }
                    let value = Saved {
                        channel_id: channel.into(),
                        saved_at: now,
                        profile,
                    };
                    if complete {
                        if let Ok(bytes) = serde_json::to_vec(&value) {
                            let _ = super::super::browser_store::atomic_write(
                                &self.directory,
                                &name,
                                &bytes,
                            );
                        }
                    }
                    (
                        value,
                        now.saturating_add(if complete { FRESH_MS } else { RETRY_MS }),
                    )
                }
                Err(cause) => match saved {
                    Some(saved) => (saved, now.saturating_add(RETRY_MS)),
                    None => return Err(cause),
                },
            }
        };
        let profile = value.profile.clone();
        let mut cache = self.cache.lock().map_err(|_| unavailable())?;
        if cache.len() >= MAX_ENTRIES {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, row)| row.retry_at)
                .map(|(id, _)| id.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(channel.into(), Cached { retry_at, value });
        Ok(profile)
    }
}
fn valid_profile(profile: &Profile) -> bool {
    profile.channel_name.chars().count() <= 200
        && !profile.channel_name.chars().any(char::is_control)
        && profile.image.as_ref().is_none_or(|image| {
            image.len() <= 192 * 1024
                && image
                    .split_once(";base64,")
                    .is_some_and(|(prefix, encoded)| {
                        let Some(mime) = prefix.strip_prefix("data:").filter(|mime| {
                            matches!(
                                *mime,
                                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
                            )
                        }) else {
                            return false;
                        };
                        STANDARD.decode(encoded).ok().is_some_and(|bytes| {
                            super::super::chat_assets::validate_raster(&bytes, Some(mime)).is_ok()
                        })
                    })
        })
}
fn portrait(url: &str) -> Result<String, StreamError> {
    let asset = super::super::chat_assets::fetch_chat_asset(url)?;
    // The CDN already supplies 120px portraits, including animated GIFs. Keep
    // the verified raster: the app intentionally has no Rust GIF decoder, and
    // attempting to re-encode every format lost otherwise valid GIF portraits.
    if asset.data_url.len() <= 192 * 1024 {
        return Ok(asset.data_url);
    }
    let bytes = STANDARD
        .decode(asset.data_url.split_once(',').ok_or_else(unavailable)?.1)
        .map_err(|_| unavailable())?;
    let image = image::load_from_memory(&bytes)
        .map_err(|_| unavailable())?
        .thumbnail(120, 120);
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(|_| unavailable())?;
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(encoded.into_inner())
    ))
}

#[tauri::command]
pub async fn chzzk_live_channel_profile(
    app: AppHandle,
    window: Webview,
    channel_id: String,
) -> ApiResult<Profile> {
    if let Err(cause) = require_main(&window) {
        return Err::<Profile, _>(cause).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        host(&app)?.inner.favorites.profiles.get(&channel_id)
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    #[test]
    #[ignore = "Explicit anonymous network smoke test; no account or recordings"]
    fn public_portrait_smoke() {
        let channel = std::env::var("ATSUMI_PROFILE_SMOKE_CHANNEL").expect("explicit channel");
        let dir = tempfile::tempdir().unwrap();
        let before = Instant::now();
        let (_, url) = ChzzkProvider::new()
            .unwrap()
            .channel_profile(&channel)
            .unwrap();
        let probe = portrait(url.as_deref().expect("public avatar URL"));
        assert!(probe.is_ok(), "portrait fetch/decode: {:?}", probe.err());
        let cache = Profiles::new(dir.path());
        let profile = cache.get(&channel).unwrap();
        assert!(valid_profile(&profile));
        assert!(profile.image.is_some());
        println!(
            "PROFILE_OK name={} imageBytes={} elapsedMs={}",
            profile.channel_name,
            profile.image.as_ref().unwrap().len(),
            before.elapsed().as_millis()
        );
        let before = Instant::now();
        let restored = Profiles::new(dir.path()).get(&channel).unwrap();
        assert_eq!(profile.image, restored.image);
        println!(
            "PROFILE_CACHE_OK elapsedMs={}",
            before.elapsed().as_millis()
        );
    }
    fn profile() -> Profile {
        Profile {
            channel_name: "채널".into(),
            image: None,
        }
    }
    #[test]
    fn portraits_survive_restart_and_do_not_refetch_during_polls() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Profiles::new(dir.path());
        cache.get_with(ID, 100, || Ok((profile(), true))).unwrap();
        assert_eq!(
            cache
                .get_with(ID, 101, || panic!("cached"))
                .unwrap()
                .channel_name,
            "채널"
        );
        let restarted = Profiles::new(dir.path());
        assert_eq!(
            restarted
                .get_with(ID, 102, || panic!("saved"))
                .unwrap()
                .channel_name,
            "채널"
        );
        assert_eq!(
            restarted
                .get_with(ID, FRESH_MS + 101, || Err(unavailable()))
                .unwrap()
                .channel_name,
            "채널"
        );
        restarted
            .get_with(ID, FRESH_MS + 102, || panic!("retry backoff"))
            .unwrap();
    }
    #[test]
    fn missing_images_are_retryable_and_inputs_cannot_be_paths_or_remote_images() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Profiles::new(dir.path());
        assert!(cache
            .get_with("../elsewhere", 100, || panic!("invalid id"))
            .is_err());
        assert!(!valid_profile(&Profile {
            channel_name: "x".into(),
            image: Some("https://example.com/a.png".into())
        }));
        cache.get_with(ID, 100, || Ok((profile(), false))).unwrap();
        cache.get_with(ID, 101, || panic!("backoff")).unwrap();
        assert!(cache
            .get_with(ID, RETRY_MS + 101, || Err(unavailable()))
            .is_err());
        assert!(!dir
            .path()
            .join(format!("chzzk-live-profile-{ID}.json"))
            .exists());
    }
}
