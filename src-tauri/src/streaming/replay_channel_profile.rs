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
    let image = value
        .image_asset_id
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
