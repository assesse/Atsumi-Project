//! Portable channel identity captured once for a newly started recording.
//! The descriptor stores an opaque asset ID, never a remote URL or credentials.
use super::*;

const LIMIT: u64 = 4096;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedProfile {
    version: u8,
    channel_id: String,
    name: String,
    image_asset_id: Option<String>,
}

pub(super) fn save(
    root: &Path,
    channel: &str,
    name: &str,
    image_id: Option<&str>,
) -> Result<(), StreamError> {
    plain_path(root)?;
    if !root.is_dir()
        || channel.len() != 32
        || !channel.bytes().all(|c| c.is_ascii_hexdigit())
        || image_id.is_some_and(|id| !valid_id(id))
    {
        return Err(unavailable());
    }
    let path = root.join("channel-profile.json");
    plain_path(&path)?;
    if path.exists() {
        return Err(unavailable());
    }
    let value = SavedProfile {
        version: 1,
        channel_id: channel.into(),
        name: name.chars().filter(|c| !c.is_control()).take(160).collect(),
        image_asset_id: image_id.map(str::to_owned),
    };
    let bytes = serde_json::to_vec(&value).map_err(|_| unavailable())?;
    let temp = root.join(format!(
        "channel-profile-{}.partial",
        uuid::Uuid::new_v4().simple()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|_| unavailable())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| unavailable())?;
    drop(file);
    if path.exists() {
        return Err(unavailable());
    }
    fs::rename(temp, path).map_err(|_| unavailable())
}

fn load(root: &Path, channel: &str) -> Result<SavedProfile, StreamError> {
    let path = root.join("channel-profile.json");
    plain_path(&path)?;
    let metadata = fs::symlink_metadata(&path).map_err(|_| unavailable())?;
    if !metadata.is_file() || metadata.len() > LIMIT {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| unavailable())?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() as u64 > LIMIT {
        return Err(unavailable());
    }
    let value: SavedProfile = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    if value.version != 1 || value.channel_id != channel {
        return Err(unavailable());
    }
    Ok(value)
}

pub(super) fn saved_image_id(root: &Path, channel: &str) -> Result<Option<String>, StreamError> {
    let id = load(root, channel)?.image_asset_id;
    if id.as_deref().is_some_and(|id| !valid_id(id)) {
        return Err(unavailable());
    }
    Ok(id)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CurrentFallback { version:u8, channel_id:String, image_asset_id:String, fetched_at:u64 }
const FALLBACK: &str = "channel-profile-current.json";

/// Keep the captured descriptor immutable. A later public avatar is explicitly
/// dated and separate; it is never represented as a captured historical image.
pub(super) fn save_current_fallback(root:&Path, channel:&str, id:&str) -> Result<(),StreamError> {
    if load(root,channel)?.image_asset_id.is_some() || !valid_id(id) { return Err(unavailable()); }
    plain_path(root)?;
    let path=root.join(FALLBACK);plain_path(&path)?;
    if path.exists() {
        return if fallback_id(root,channel).as_deref()==Some(id) { Ok(()) } else { Err(unavailable()) };
    }
    let value=CurrentFallback {version:1,channel_id:channel.into(),image_asset_id:id.into(),fetched_at:std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_|unavailable())?.as_millis() as u64};
    let bytes=serde_json::to_vec(&value).map_err(|_|unavailable())?;
    let temp=root.join(format!("channel-profile-current-{}.partial",uuid::Uuid::new_v4().simple()));
    let mut file=OpenOptions::new().create_new(true).write(true).open(&temp).map_err(|_|unavailable())?;
    file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|_|unavailable())?;
    drop(file);
    if path.exists() { return Err(unavailable()); }
    fs::rename(temp,path).map_err(|_|unavailable())
}
fn fallback_id(root:&Path,channel:&str)->Option<String> {
    let path=root.join(FALLBACK);plain_path(&path).ok()?;
    let meta=fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len()>LIMIT {return None;}
    let value:CurrentFallback=serde_json::from_reader(fs::File::open(path).ok()?).ok()?;
    (value.version==1 && value.channel_id==channel && value.fetched_at>0 && valid_id(&value.image_asset_id)).then_some(value.image_asset_id)
}
pub fn uses_current_fallback(root:&Path,channel:&str)->bool {
    saved_image_id(root,channel).ok().is_some_and(|id|id.is_none())
        && fallback_id(root,channel).is_some_and(|id|read_recording(root,&id).is_ok())
}

/// Once per replay open. A small verified raster travels as inline data through
/// the existing trusted bridge. The opaque player needs no new network access.
pub fn read(root: &Path, channel: &str) -> (Option<String>, Option<String>) {
    let Ok(value) = load(root, channel) else {
        return (None, None);
    };
    let name: String = value
        .name
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect();
    let image_id = value.image_asset_id.or_else(||fallback_id(root,channel));
    let image = image_id
        .as_deref()
        .and_then(|id| read_recording(root, id).ok())
        .map(|asset| {
            format!(
                "data:{};base64,{}",
                asset.mime,
                STANDARD.encode(asset.bytes)
            )
        });
    ((!name.trim().is_empty()).then_some(name), image)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_legacy_corrupt_foreign_and_external_descriptors_fall_back_locally() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let channel = "a".repeat(32);
        assert_eq!(read(&root, &channel), (None, None));
        assert!(save(&root, &channel, "name", Some("../../private")).is_err());
        save(&root, &channel, "채널 이름\n", Some(&"b".repeat(64))).unwrap();
        assert_eq!(read(&root, &channel), (Some("채널 이름".into()), None));
        assert_eq!(read(&root, &"c".repeat(32)), (None, None));
        assert!(save(&root, &channel, "changed", None).is_err());
        fs::write(
            root.join("channel-profile.json"),
            "x".repeat(LIMIT as usize + 1),
        )
        .unwrap();
        assert_eq!(read(&root, &channel), (None, None));
    }
}
