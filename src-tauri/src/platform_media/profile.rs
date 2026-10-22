//! Immutable, bounded recording-time public portrait; never fetched on replay.
mod current;
mod snapshot;
pub(super) use snapshot::Snapshots;
use super::storage;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

const FILE: &str = "channel-profile.json";
const MAX_IMAGE: usize = 128 * 1024;
const MAX_FILE: u64 = 180 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Profile {
    version: u8,
    mime: String,
    image: String,
    sha256: String,
}
impl Profile {
    pub(super) fn from_data_url(value: &str) -> Option<Self> {
        if value.len() > MAX_FILE as usize {
            return None;
        }
        let (head, image) = value.split_once(',')?;
        let mime = head.strip_prefix("data:")?.strip_suffix(";base64")?;
        let bytes = STANDARD.decode(image).ok()?;
        validate(&bytes, mime).ok()?;
        Some(Self {
            version: 1,
            mime: mime.into(),
            image: STANDARD.encode(&bytes),
            sha256: format!("{:x}", Sha256::digest(bytes)),
        })
    }
    pub(super) fn bounded_snapshot(&self) -> Result<Self, String> {
        if self.version != 1
            || self.image.len() > MAX_FILE as usize
            || !matches!(self.mime.as_str(), "image/png" | "image/gif")
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("프로필 형식 또는 크기가 올바르지 않습니다.".into());
        }
        Ok(self.clone())
    }
    pub(super) fn data_url(&self) -> Result<String, String> {
        if self.version != 1 || self.image.len() > MAX_FILE as usize {
            return Err("저장된 프로필 형식이 올바르지 않습니다.".into());
        }
        let bytes = STANDARD
            .decode(&self.image)
            .map_err(|_| "저장된 프로필을 읽지 못했습니다.")?;
        validate(&bytes, &self.mime)?;
        if self.sha256 != format!("{:x}", Sha256::digest(&bytes)) {
            return Err("저장된 프로필이 변경됐습니다.".into());
        }
        Ok(format!("data:{};base64,{}", self.mime, self.image))
    }
}
fn validate(bytes: &[u8], mime: &str) -> Result<(), String> {
    if bytes.len() > MAX_IMAGE || !matches!(mime, "image/png" | "image/gif") {
        return Err("프로필 형식 또는 크기가 올바르지 않습니다.".into());
    }
    crate::streaming::chat_assets::validate_raster(bytes, Some(mime))
        .map_err(|_| "프로필 이미지 형식이 올바르지 않습니다.".to_string())?;
    let (width, height) = if mime == "image/gif" {
        (
            u16::from_le_bytes([bytes[6], bytes[7]]) as u32,
            u16::from_le_bytes([bytes[8], bytes[9]]) as u32,
        )
    } else {
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| "프로필 형식 오류")?;
        reader.into_dimensions().map_err(|_| "프로필 크기 오류")?
    };
    let maximum = if mime == "image/png" { 96 } else { 512 };
    if width == 0 || height == 0 || width > maximum || height > maximum {
        return Err("프로필 이미지가 너무 큽니다.".into());
    }
    Ok(())
}
fn pin_parents(path: &Path) -> Result<Vec<File>, String> {
    storage::safe_path(path)?;
    let mut guards = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        for parent in path
            .ancestors()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            let file = OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x02200000)
                .open(parent)
                .map_err(|_| "프로필 폴더를 잠그지 못했습니다.")?;
            validate_handle(&file, true)?;
            guards.push(file);
        }
    }
    Ok(guards)
}
#[cfg(windows)]
fn validate_handle(file: &File, directory: bool) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
            .map_err(|_| "프로필 파일 확인 오류")?;
    }
    if info.dwFileAttributes & 0x400 != 0
        || (info.dwFileAttributes & 0x10 != 0) != directory
        || (!directory && info.nNumberOfLinks != 1)
    {
        return Err("연결된 프로필 파일은 사용할 수 없습니다.".into());
    }
    Ok(())
}
pub(super) fn save(directory: &Path, profile: &Profile) -> Result<(), String> {
    profile.data_url()?;
    let path = directory.join(FILE);
    let _guards = pin_parents(&path)?;
    let bytes = serde_json::to_vec(profile).map_err(|_| "프로필 보관 오류")?;
    if bytes.len() > MAX_FILE as usize {
        return Err("프로필 파일이 너무 큽니다.".into());
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0).custom_flags(0x00200000);
    }
    let mut file = options
        .open(&path)
        .map_err(|_| "프로필을 보관하지 못했습니다.")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "프로필을 저장하지 못했습니다.".into())
}
pub(super) fn read(directory: &Path) -> Result<Option<String>, String> {
    let path = directory.join(FILE);
    let _guards = pin_parents(&path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = match options.open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("프로필 파일을 열지 못했습니다.".into()),
    };
    #[cfg(windows)]
    validate_handle(&file, false)?;
    #[cfg(not(windows))]
    storage::regular(&path)?;
    if file.metadata().map_err(|_| "프로필 파일 확인 오류")?.len() > MAX_FILE {
        return Err("프로필 파일이 너무 큽니다.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "프로필 파일 읽기 오류")?;
    if bytes.len() > MAX_FILE as usize {
        return Err("프로필 파일이 너무 큽니다.".into());
    }
    let profile: Profile =
        serde_json::from_slice(&bytes).map_err(|_| "프로필 파일이 손상됐습니다.")?;
    profile.data_url().map(Some)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Profile {
        let mut output = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(32, 32)
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        Profile::from_data_url(&format!(
            "data:image/png;base64,{}",
            STANDARD.encode(output.into_inner())
        ))
        .unwrap()
    }
    #[test]
    fn live_profile_snapshot_is_bounded_and_validation_rejects_corruption() {
        let original = fixture();
        let snapshot = original.bounded_snapshot().unwrap();
        assert_eq!(snapshot.data_url().unwrap(), original.data_url().unwrap());
        let mut corrupt = original.clone();
        corrupt.sha256 = "0".repeat(64);
        assert!(corrupt.bounded_snapshot().unwrap().data_url().is_err());
        let mut oversized = original.clone();
        oversized.image = "A".repeat(MAX_FILE as usize + 1);
        assert!(oversized.bounded_snapshot().is_err());
        let mut remote = original;
        remote.mime = "https://example.invalid/avatar".into();
        assert!(remote.bounded_snapshot().is_err());
    }
    #[test]
    fn immutable_local_profile_roundtrip_and_absent_legacy() {
        let root = tempfile::tempdir().unwrap();
        assert!(read(root.path()).unwrap().is_none());
        let profile = fixture();
        save(root.path(), &profile).unwrap();
        assert_eq!(
            read(root.path()).unwrap(),
            Some(profile.data_url().unwrap())
        );
        assert!(save(root.path(), &profile).is_err());
    }
    #[test]
    fn normalized_gif_profile_is_preserved_and_legacy_small_gif_remains_readable() {
        let mut source = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut source);
            encoder
                .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                    600,
                    600,
                    image::Rgba([25, 50, 100, 255]),
                )))
                .unwrap();
        }
        let normalized = crate::soop::normalize_portrait(&source).unwrap();
        assert!(normalized.starts_with("data:image/png;base64,"));
        let root = tempfile::tempdir().unwrap();
        save(root.path(), &Profile::from_data_url(&normalized).unwrap()).unwrap();
        assert_eq!(read(root.path()).unwrap(), Some(normalized));

        let mut legacy = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut legacy);
            encoder
                .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                    32,
                    32,
                    image::Rgba([25, 50, 100, 255]),
                )))
                .unwrap();
        }
        let original = format!("data:image/gif;base64,{}", STANDARD.encode(legacy));
        let old = tempfile::tempdir().unwrap();
        save(old.path(), &Profile::from_data_url(&original).unwrap()).unwrap();
        assert_eq!(read(old.path()).unwrap(), Some(original));
    }
    #[test]
    fn tampered_proof_and_non_raster_rejected() {
        let root = tempfile::tempdir().unwrap();
        let mut profile = fixture();
        profile.sha256 = "0".repeat(64);
        std::fs::write(
            root.path().join(FILE),
            serde_json::to_vec(&profile).unwrap(),
        )
        .unwrap();
        assert!(read(root.path()).is_err());
        assert!(Profile::from_data_url("data:image/svg+xml;base64,PHN2Zz4=").is_none());
    }
    #[test]
    fn linked_profile_rejected_without_touching_original() {
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("other.json");
        std::fs::write(&outside, serde_json::to_vec(&fixture()).unwrap()).unwrap();
        std::fs::hard_link(&outside, root.path().join(FILE)).unwrap();
        assert!(read(root.path()).is_err());
        assert!(save(root.path(), &fixture()).is_err());
        assert!(outside.exists());
    }
}
