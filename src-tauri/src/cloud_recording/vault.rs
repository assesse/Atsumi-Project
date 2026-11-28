use super::{catalog::Catalog, model::CloudError};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
const LIMIT: u64 = 8 * 1024 * 1024;

pub(crate) struct Vault {
    path: PathBuf,
    _lock: File,
    #[cfg(test)]
    fail_next_save: std::sync::atomic::AtomicBool,
}
impl Vault {
    pub(crate) fn open(root: &Path) -> Result<Self, CloudError> {
        let root = root.join("cloud-recording");
        std::fs::create_dir_all(&root).map_err(|_| CloudError::storage())?;
        regular_directory(&root)?;
        let lock_path = root.join("state.lock");
        if lock_path.exists() {
            regular_file(&lock_path)?;
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|_| CloudError::storage())?;
        lock.try_lock_exclusive().map_err(|_| {
            CloudError::new(
                "CLOUD_STORAGE_BUSY",
                "다른 앱에서 업로드 상태를 사용 중입니다.",
                true,
            )
        })?;
        Ok(Self {
            path: root.join("state.bin"),
            _lock: lock,
            #[cfg(test)]
            fail_next_save: std::sync::atomic::AtomicBool::new(false),
        })
    }
    pub(crate) fn load(&self) -> Result<Catalog, CloudError> {
        if !self.path.exists() {
            return Ok(Catalog::default());
        }
        regular_file(&self.path)?;
        let mut bytes = Vec::new();
        File::open(&self.path)
            .map_err(|_| CloudError::storage())?
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| CloudError::storage())?;
        if bytes.is_empty() || bytes.len() as u64 > LIMIT {
            return Err(CloudError::storage());
        }
        let plain = platform::crypt(&bytes, false).map_err(|_| CloudError::storage())?;
        let catalog: Catalog = serde_json::from_slice(&plain).map_err(|_| CloudError::storage())?;
        if !catalog.valid() {
            return Err(CloudError::storage());
        }
        Ok(catalog)
    }
    pub(crate) fn save(&self, catalog: &Catalog) -> Result<(), CloudError> {
        #[cfg(test)]
        if self
            .fail_next_save
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(CloudError::storage());
        }
        if !catalog.valid() {
            return Err(CloudError::storage());
        }
        let plain = serde_json::to_vec(catalog).map_err(|_| CloudError::storage())?;
        if plain.len() as u64 > LIMIT / 2 {
            return Err(CloudError::storage());
        }
        let encrypted = platform::crypt(&plain, true).map_err(|_| CloudError::storage())?;
        let temporary = self
            .path
            .with_extension(format!("{}.pending", uuid::Uuid::new_v4().simple()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|_| CloudError::storage())?;
            file.write_all(&encrypted)
                .and_then(|_| file.sync_all())
                .map_err(|_| CloudError::storage())?;
            drop(file);
            if self.path.exists() {
                regular_file(&self.path)?;
            }
            platform::replace(&temporary, &self.path).map_err(|_| CloudError::storage())
        })();
        // Only this uniquely named encrypted pending file is owned by this call.
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
    #[cfg(test)]
    pub(crate) fn fail_once(&self) {
        self.fail_next_save
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
fn safe_metadata(path: &Path) -> Result<std::fs::Metadata, CloudError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| CloudError::storage())?;
    if metadata.file_type().is_symlink() {
        return Err(CloudError::storage());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(CloudError::storage());
        }
    }
    Ok(metadata)
}
fn regular_file(path: &Path) -> Result<(), CloudError> {
    if safe_metadata(path)?.is_file() {
        Ok(())
    } else {
        Err(CloudError::storage())
    }
}
fn regular_directory(path: &Path) -> Result<(), CloudError> {
    if safe_metadata(path)?.is_dir() {
        Ok(())
    } else {
        Err(CloudError::storage())
    }
}

#[cfg(windows)]
mod platform {
    use std::{os::windows::ffi::OsStrExt, path::Path};
    use windows::{
        core::{w, PCWSTR},
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::Cryptography::{
                CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
            },
            Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH},
        },
    };
    pub(super) fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>, ()> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: bytes.len().try_into().map_err(|_| ())?,
            pbData: bytes.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            if encrypt {
                CryptProtectData(
                    &input,
                    w!("Atsumi cloud recording v1"),
                    None,
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            } else {
                CryptUnprotectData(
                    &input,
                    None,
                    None,
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            }
            .map_err(|_| ())?;
            if output.pbData.is_null() {
                return Err(());
            }
            let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(output.pbData as *mut _)));
            Ok(result)
        }
    }
    pub(super) fn replace(from: &Path, to: &Path) -> Result<(), ()> {
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|_| ())
    }
}
#[cfg(not(windows))]
mod platform {
    pub(super) fn crypt(_: &[u8], _: bool) -> Result<Vec<u8>, ()> {
        Err(())
    }
    pub(super) fn replace(_: &std::path::Path, _: &std::path::Path) -> Result<(), ()> {
        Err(())
    }
}
