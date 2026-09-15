use super::{Identity, IdentityStore};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

pub(super) struct FileVault {
    path: PathBuf,
    _lock: File,
}
const LIMIT: u64 = 64 * 1024;
const STORAGE_ERROR: &str =
    "작성자 키를 안전하게 저장하지 못했습니다. 새 키 발급이나 후기 전송을 중단했습니다.";
const READ_ERROR: &str =
    "보관된 작성자 키를 읽지 못했습니다. 기존 키는 보존했으며 새로 발급하지 않았습니다.";

impl FileVault {
    pub(super) fn open(path: PathBuf) -> Result<Self, String> {
        let parent = path.parent().ok_or(STORAGE_ERROR)?;
        std::fs::create_dir_all(parent).map_err(|_| STORAGE_ERROR)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("lock"))
            .map_err(|_| STORAGE_ERROR)?;
        lock.try_lock_exclusive()
            .map_err(|_| "다른 작성자 키 작업이 진행 중입니다. 잠시 후 다시 시도해 주세요.")?;
        Ok(Self { path, _lock: lock })
    }
}

impl IdentityStore for FileVault {
    fn load(&self) -> Result<Identity, String> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Identity::Unissued)
            }
            Err(_) => return Err(READ_ERROR.into()),
        };
        let mut encrypted = Vec::new();
        file.take(LIMIT + 1)
            .read_to_end(&mut encrypted)
            .map_err(|_| READ_ERROR)?;
        if encrypted.is_empty() || encrypted.len() as u64 > LIMIT {
            return Err(READ_ERROR.into());
        }
        let bytes = platform::crypt(&encrypted, false).map_err(|_| READ_ERROR)?;
        serde_json::from_slice(&bytes).map_err(|_| READ_ERROR.into())
    }

    fn save(&self, identity: &Identity) -> Result<(), String> {
        let bytes = serde_json::to_vec(identity).map_err(|_| STORAGE_ERROR)?;
        if bytes.len() as u64 > LIMIT / 2 {
            return Err(STORAGE_ERROR.into());
        }
        let encrypted = platform::crypt(&bytes, true).map_err(|_| STORAGE_ERROR)?;
        let temporary = self.path.with_extension("pending");
        // Only ciphertext ever reaches disk. A same-directory durable rename
        // means reset/crash cannot leave a half-written credential JSON file.
        let mut file = File::create(&temporary).map_err(|_| STORAGE_ERROR)?;
        file.write_all(&encrypted)
            .and_then(|_| file.sync_all())
            .map_err(|_| STORAGE_ERROR)?;
        drop(file);
        platform::replace(&temporary, &self.path).map_err(|_| STORAGE_ERROR.into())
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
        // No CRYPTPROTECT_LOCAL_MACHINE: only the current Windows user can decrypt.
        unsafe {
            if encrypt {
                CryptProtectData(
                    &input,
                    w!("Atsumi Community identity v1"),
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
