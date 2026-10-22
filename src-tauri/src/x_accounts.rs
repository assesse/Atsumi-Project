//! Profile references only. Authentication remains in WebView2's protected store.
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

const FILE: &str = "accounts.json";
const ERROR: &str = "X 계정 목록을 저장하거나 읽지 못했습니다. 기존 로그인 정보는 보존됩니다.";
pub(super) const LEGACY: &str = "legacy";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub id: String,
    pub handle: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Registry {
    pub active: String,
    pub accounts: Vec<Account>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            active: LEGACY.into(),
            accounts: vec![],
        }
    }
}
pub(super) fn valid_profile(id: &str) -> bool {
    id == LEGACY
        || id.len() == 32
            && id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
impl Registry {
    fn validate(&self) -> bool {
        valid_profile(&self.active)
            && self.accounts.len() <= 20
            && (self.active == LEGACY || self.accounts.iter().any(|a| a.id == self.active))
            && self
                .accounts
                .iter()
                .all(|a| valid_profile(&a.id) && super::valid_handle(&a.handle))
            && self
                .accounts
                .iter()
                .map(|a| &a.id)
                .collect::<std::collections::HashSet<_>>()
                .len()
                == self.accounts.len()
    }
    pub(super) fn load(app: &AppHandle) -> Result<Self, String> {
        let path = root(app)?.join(FILE);
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() <= 16384 => {
                let result: Self = serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
                if !result.validate() {
                    return Err(ERROR.into());
                }
                Ok(result)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            _ => Err(ERROR.into()),
        }
    }
    pub(super) fn save(&self, app: &AppHandle) -> Result<(), String> {
        if !self.validate() {
            return Err(ERROR.into());
        }
        let directory = root(app)?;
        std::fs::create_dir_all(&directory).map_err(|_| ERROR)?;
        let temporary = directory.join(format!("accounts-{}.tmp", uuid::Uuid::new_v4().simple()));
        let bytes = serde_json::to_vec(self).map_err(|_| ERROR)?;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| ERROR)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| ERROR)?;
        drop(file);
        replace(&temporary, &directory.join(FILE)).map_err(|_| ERROR.into())
    }
    pub(super) fn remember(&mut self, id: &str, handle: &str) -> Result<(), String> {
        if !valid_profile(id) || !super::valid_handle(handle) {
            return Err(ERROR.into());
        }
        if let Some(account) = self.accounts.iter_mut().find(|a| a.id == id) {
            if account.handle != handle {
                return Err(
                    "선택한 X 계정과 실제 로그인 계정이 다릅니다. 계정 추가로 등록해 주세요."
                        .into(),
                );
            }
        } else {
            if self.accounts.len() >= 20 {
                return Err("저장 가능한 X 계정은 최대 20개입니다.".into());
            }
            self.accounts.push(Account {
                id: id.into(),
                handle: handle.into(),
            });
        }
        Ok(())
    }
}
pub(super) fn root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|_| ERROR)?
        .join("x-accounts"))
}
pub(super) fn profile(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    if !valid_profile(id) {
        return Err(ERROR.into());
    }
    if id == LEGACY {
        return crate::browser_identity::profile(app);
    }
    let directory = root(app)?.join("profiles").join(id);
    std::fs::create_dir_all(&directory).map_err(|_| ERROR)?;
    Ok(directory)
}
pub(super) fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            },
        };
        let source: Vec<_> = from
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let target: Vec<_> = to
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(std::io::Error::other)
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_is_bounded_and_never_accepts_paths_or_secrets() {
        for id in ["../old", "C:/old", "", "LEGACY", "../../Cookies"] {
            assert!(!valid_profile(id));
        }
        assert!(valid_profile(LEGACY));
        assert!(valid_profile(&uuid::Uuid::new_v4().simple().to_string()));
        let mut registry = Registry::default();
        registry.remember(LEGACY, "owner").unwrap();
        assert!(registry.validate());
        assert!(registry.remember(LEGACY, "other").is_err());
        registry.active = "unregistered".into();
        assert!(!registry.validate());
        assert!(serde_json::from_str::<Registry>(
            r#"{"active":"legacy","accounts":[],"cookie":"secret"}"#
        )
        .is_err());
    }
}
