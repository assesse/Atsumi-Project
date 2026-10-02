//! Local SSD selection only. Never infer SSD from a drive letter or a failed query.
use super::*;
use sha2::{Digest, Sha256};

pub(super) const HEADROOM: u64 = 10 * 1024 * 1024 * 1024;
const START_FREE: u64 = 2 * HEADROOM;

pub(crate) struct StoragePlan {
    pub working_root: PathBuf,
    pub archive_root: Option<PathBuf>,
}

pub(crate) fn plan(
    enabled: bool,
    data_dir: &Path,
    destination: &Path,
) -> Result<StoragePlan, StreamError> {
    let destination = checked_directory(destination)?;
    if !enabled || crate::storage_io_budget::is_solid_state(&destination) == Some(true) {
        return Ok(StoragePlan {
            working_root: destination,
            archive_root: None,
        });
    }
    let candidates = candidates(data_dir);
    let candidate = select(candidates).ok_or_else(|| error(
        "BROWSER_SSD_UNAVAILABLE",
        "20 GiB 이상 여유 공간이 있는 로컬 SSD를 찾지 못했습니다. 공간을 확보하거나 설정의 ‘CHZZK 녹화에 SSD 사용’을 꺼 주세요.", true))?;
    let base = if candidate.preferred {
        child_directory(&checked_directory(data_dir)?, "ssd-recordings")?
    } else {
        let identity = format!(
            "{:x}",
            Sha256::digest(data_dir.to_string_lossy().as_bytes())
        );
        child_directory(
            &candidate.root,
            &format!("Atsumi-Recordings-{}", &identity[..16]),
        )?
    };
    tracing::info!(working_root=%base.display(), archive_root=%destination.display(), free_gib=candidate.free / 1024 / 1024 / 1024, "SSD recording staging selected");
    Ok(StoragePlan {
        working_root: base,
        archive_root: Some(destination),
    })
}

struct Candidate {
    root: PathBuf,
    free: u64,
    ssd: bool,
    preferred: bool,
}
fn select(candidates: Vec<Candidate>) -> Option<Candidate> {
    candidates
        .into_iter()
        .filter(|c| c.ssd && c.free >= START_FREE)
        .max_by_key(|c| (c.free, c.preferred))
}

#[cfg(windows)]
fn candidates(data_dir: &Path) -> Vec<Candidate> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives},
    };
    let drives = unsafe { GetLogicalDrives() };
    let data = data_dir
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_ascii_lowercase();
    (0..26)
        .filter(|index| drives & (1 << index) != 0)
        .filter_map(|index| {
            let root = PathBuf::from(format!("{}:\\", (b'A' + index) as char));
            let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
            // DRIVE_FIXED = 3: do not select removable/remote/optical drives.
            if unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) } != 3 {
                return None;
            }
            let ssd = crate::storage_io_budget::is_solid_state(&root) == Some(true);
            Some(Candidate {
                preferred: data.starts_with(&root.to_string_lossy().to_ascii_lowercase()),
                free: fs2::available_space(&root).ok()?,
                root,
                ssd,
            })
        })
        .collect()
}
#[cfg(not(windows))]
fn candidates(_: &Path) -> Vec<Candidate> {
    Vec::new()
}

/// Reserve room for final merges of *all* unmerged recordings on this staging
/// root, not just this stream. Never delete older recordings to obtain space.
pub(super) fn ensure_headroom(
    state: &State,
    recording: &BrowserRecording,
) -> Result<(), StreamError> {
    if recording.archive.is_none() {
        return Ok(());
    }
    let volume = Path::new(&recording.output_dir)
        .ancestors()
        .nth(3)
        .ok_or_else(invalid)?;
    let reserve = state
        .recordings
        .iter()
        .filter(|r| {
            r.archive.is_some()
                && r.media_removed_at.is_none()
                && r.merge
                    .as_ref()
                    .is_none_or(|m| m.status != BrowserMergeStatus::Complete)
                && Path::new(&r.output_dir).starts_with(volume)
        })
        .fold(HEADROOM, |sum, r| sum.saturating_add(r.bytes_written));
    if fs2::available_space(volume).map_err(|_| storage())? < reserve.saturating_add(MAX_SEGMENT) {
        return Err(error("BROWSER_CAPTURE_DISK_FULL", "SSD의 녹화·병합 여유 공간이 부족합니다. 확정된 영상은 보존됩니다. 공간을 확보한 뒤 다시 녹화해 주세요.", true));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn c(name: &str, free: u64, ssd: bool) -> Candidate {
        Candidate {
            root: name.into(),
            free,
            ssd,
            preferred: false,
        }
    }
    #[test]
    fn only_confirmed_ssds_with_headroom_can_be_selected() {
        let found = select(vec![
            c("hdd", 100 * START_FREE, false),
            c("full-ssd", START_FREE - 1, true),
            c("ssd", START_FREE, true),
        ])
        .unwrap();
        assert_eq!(found.root, Path::new("ssd"));
        assert!(select(vec![c("unknown", 100 * START_FREE, false)]).is_none());
    }
    #[test]
    fn most_free_ssd_is_selected_and_off_never_creates_working_folders() {
        assert_eq!(
            select(vec![c("a", START_FREE, true), c("b", 2 * START_FREE, true)])
                .unwrap()
                .root,
            Path::new("b")
        );
        let dir = tempfile::tempdir().unwrap();
        let plan = plan(false, dir.path(), dir.path()).unwrap();
        assert!(plan.archive_root.is_none());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[test]
    #[ignore = "read-only local SSD discovery"]
    fn inspect_local_ssds() {
        let data = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("local.atsumi.next");
        for c in candidates(&data) {
            println!(
                "{} SSD={} freeGiB={}",
                c.root.display(),
                c.ssd,
                c.free / 1024 / 1024 / 1024
            );
        }
    }
}
