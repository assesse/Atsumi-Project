//! Portable packaging is installation-free, not a second user-data namespace.
//! Existing AppData/login/community keys are deliberately left unchanged.
use std::path::Path;

pub(crate) fn installer_allowed_at(executable: &Path) -> bool {
    !executable
        .parent()
        .is_some_and(|root| root.join("atsumi-portable.json").exists())
}

pub(crate) fn ensure_installer_allowed() -> Result<(), crate::streaming::model::StreamError> {
    let executable = std::env::current_exe().map_err(|_| {
        crate::streaming::model::StreamError::new(
            "UPDATE_LOCATION_UNKNOWN",
            "실행 위치를 확인하지 못해 업데이트 설치를 보류했습니다.",
            true,
        )
    })?;
    if installer_allowed_at(&executable) {
        return Ok(());
    }
    Err(crate::streaming::model::StreamError::new("PORTABLE_MANUAL_UPDATE",
        "무설치판은 설치형 업데이트를 실행하지 않습니다. GitHub의 새 Atsumi-Portable.zip을 다른 폴더에 풀고, 현재 앱을 종료한 뒤 실행해 주세요. 기존 설정·로그인·녹화는 AppData와 다운로드 폴더에 유지됩니다.", false))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_marker_blocks_installer_without_touching_other_folders() {
        let root = tempfile::tempdir().unwrap();
        let exe = root.path().join("atsumi.exe");
        assert!(installer_allowed_at(&exe));
        std::fs::write(root.path().join("atsumi-portable.json"), "{\"format\":1}").unwrap();
        assert!(!installer_allowed_at(&exe));
        assert!(installer_allowed_at(&root.path().join("other/atsumi.exe")));
    }
}
