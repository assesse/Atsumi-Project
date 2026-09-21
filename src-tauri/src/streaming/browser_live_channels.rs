//! Local bookmarks, deliberately independent from automatic recording rules.
use super::*;

const FILE: &str = "chzzk-live-favorites.json";
const LIMIT: usize = 128;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Favorite {
    channel_id: String,
    channel_name: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    version: u32,
    channels: Vec<Favorite>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            channels: Vec::new(),
        }
    }
}
pub(super) struct Favorites {
    directory: PathBuf,
    config: Mutex<Result<Config, ()>>,
    pub(super) profiles: live_profiles::Profiles,
}
fn invalid() -> StreamError {
    error(
        "LIVE_FAVORITES_STORAGE",
        "즐겨찾기를 읽거나 저장하지 못했습니다. 기존 파일은 유지됩니다.",
    )
}
fn validate(config: &Config) -> Result<(), StreamError> {
    let mut ids = std::collections::HashSet::new();
    if config.version != 1 || config.channels.len() > LIMIT {
        return Err(invalid());
    }
    for row in &config.channels {
        if normalize_channel_input(&row.channel_id).ok().as_deref() != Some(&row.channel_id)
            || !ids.insert(&row.channel_id)
            || row.channel_name.chars().count() > 200
            || row.channel_name.chars().any(char::is_control)
        {
            return Err(invalid());
        }
    }
    Ok(())
}
impl Favorites {
    pub fn load(directory: &Path) -> Self {
        let path = directory.join(FILE);
        let config = match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Ok(meta) if meta.is_file() && meta.len() <= 256 * 1024 => (|| {
                let bytes = std::fs::read(&path).map_err(|_| ())?;
                let config: Config = serde_json::from_slice(&bytes).map_err(|_| ())?;
                validate(&config).map_err(|_| ())?;
                Ok(config)
            })(),
            _ => Err(()),
        };
        Self {
            directory: directory.into(),
            config: Mutex::new(config),
            profiles: live_profiles::Profiles::new(directory),
        }
    }
    pub fn snapshot(&self) -> Result<Vec<Favorite>, StreamError> {
        Ok(self
            .config
            .lock()
            .map_err(|_| invalid())?
            .as_ref()
            .map_err(|_| invalid())?
            .channels
            .clone())
    }
    fn set(
        &self,
        channel_id: String,
        channel_name: String,
        favorite: bool,
    ) -> Result<Vec<Favorite>, StreamError> {
        let mut current = self.config.lock().map_err(|_| invalid())?;
        let mut next = current.as_ref().map_err(|_| invalid())?.clone();
        next.channels.retain(|row| row.channel_id != channel_id);
        if favorite {
            next.channels.push(Favorite {
                channel_id,
                channel_name,
            });
        }
        validate(&next)?;
        super::super::browser_store::atomic_write(
            &self.directory,
            FILE,
            &serde_json::to_vec(&next).map_err(|_| invalid())?,
        )?;
        let result = next.channels.clone();
        *current = Ok(next);
        Ok(result)
    }
}
#[tauri::command]
pub async fn chzzk_live_favorites(app: AppHandle, window: Webview) -> ApiResult<Vec<Favorite>> {
    (|| {
        require_main(&window)?;
        host(&app)?.inner.favorites.snapshot()
    })()
    .into()
}
#[tauri::command]
pub async fn chzzk_live_favorite_set(
    app: AppHandle,
    window: Webview,
    input: String,
    favorite: bool,
) -> ApiResult<Vec<Favorite>> {
    if let Err(cause) = require_main(&window) {
        return Err::<Vec<Favorite>, _>(cause).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        let host = host(&app)?;
        let channel = normalize_channel_input(&input)?;
        // Removal never depends on network availability. Adding a bookmark
        // fetches public metadata only; it never creates a player or recorder.
        let name = if favorite {
            ChzzkProvider::new()?.channel_profile(&channel)?.0
        } else {
            String::new()
        };
        host.inner.favorites.set(channel, name, favorite)
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    #[test]
    fn favorites_persist_without_creating_or_changing_recording_rules() {
        let dir = tempfile::tempdir().unwrap();
        let original = b"recording rules must remain untouched";
        std::fs::write(dir.path().join("chzzk-auto-record.json"), original).unwrap();
        let saved = Favorites::load(dir.path());
        saved.set(CHANNEL.into(), "channel".into(), true).unwrap();
        saved.set(CHANNEL.into(), "renamed".into(), true).unwrap();
        let loaded = Favorites::load(dir.path());
        assert_eq!(loaded.snapshot().unwrap().len(), 1);
        assert_eq!(loaded.snapshot().unwrap()[0].channel_name, "renamed");
        loaded.set(CHANNEL.into(), String::new(), false).unwrap();
        assert!(Favorites::load(dir.path()).snapshot().unwrap().is_empty());
        assert_eq!(
            std::fs::read(dir.path().join("chzzk-auto-record.json")).unwrap(),
            original
        );
    }
    #[test]
    fn corrupt_or_foreign_files_are_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), b"broken").unwrap();
        let saved = Favorites::load(dir.path());
        assert!(saved.snapshot().is_err());
        assert!(saved.set(CHANNEL.into(), "name".into(), true).is_err());
        assert_eq!(std::fs::read(dir.path().join(FILE)).unwrap(), b"broken");
    }
}
