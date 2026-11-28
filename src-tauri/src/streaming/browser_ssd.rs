//! Local SSD selection only. Never infer SSD from a drive letter or a failed query.
use super::*;
use sha2::{Digest, Sha256};

pub(super) const HEADROOM: u64 = 10 * 1024 * 1024 * 1024;
const START_FREE: u64 = 2 * HEADROOM;

pub(crate) struct StoragePlan {
    pub working_root: PathBuf,
    pub archive_root: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStorageStatus {
    pub destination_kind: &'static str,
    pub detected_ssds: usize,
    pub staging_available: bool,
}

/// Read-only settings inspection. Never creates a staging directory or changes
/// the recording plan. Unknown hardware stays unknown (including network disks).
pub(crate) fn inspect(data_dir: &Path, destination: Option<&Path>) -> RecordingStorageStatus {
    let destination_kind = destination
        .and_then(|p| p.ancestors().find(|p| p.exists()))
        .and_then(|p| fs::canonicalize(p).ok())
        .and_then(|p| crate::storage_io_budget::is_solid_state(&p));
    status(destination_kind, &candidates(data_dir))
}

fn status(destination: Option<bool>, candidates: &[Candidate]) -> RecordingStorageStatus {
    RecordingStorageStatus {
        destination_kind: match destination {
            Some(true) => "ssd",
            Some(false) => "hdd",
            None => "unknown",
        },
        detected_ssds: candidates.iter().filter(|c| c.ssd).count(),
        staging_available: candidates.iter().any(|c| c.ssd && c.free >= START_FREE),
    }
}

fn named_staging_root(data_dir: &Path, root: &Path) -> PathBuf {
    let identity = format!(
        "{:x}",
        Sha256::digest(data_dir.to_string_lossy().as_bytes())
    );
    root.join(format!("Atsumi-Recordings-{}", &identity[..16]))
}

/// Only app-owned staging paths, not entire drives. Include old paths even if
/// the disk is no longer selected, so unfinished media isn't counted as a DB.
pub(crate) fn staging_roots(data_dir: &Path) -> Vec<PathBuf> {
    let mut roots = vec![data_dir.join("ssd-recordings")];
    roots.extend(
        candidates(data_dir)
            .iter()
            .map(|c| named_staging_root(data_dir, &c.root)),
    );
    roots
}

pub(crate) fn plan(
    enabled: bool,
    data_dir: &Path,
    destination: &Path,
) -> Result<StoragePlan, StreamError> {
    let destination = checked_directory(destination)?;
    if !needs_staging(
        enabled,
        crate::storage_io_budget::is_solid_state(&destination),
    ) {
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
        let root = named_staging_root(data_dir, &candidate.root);
        child_directory(
            &candidate.root,
            root.file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(invalid)?,
        )?
    };
    tracing::info!(working_root=%base.display(), archive_root=%destination.display(), free_gib=candidate.free / 1024 / 1024 / 1024, "SSD recording staging selected");
    Ok(StoragePlan {
        working_root: base,
        archive_root: Some(destination),
    })
}

fn needs_staging(enabled: bool, destination_ssd: Option<bool>) -> bool {
    enabled && destination_ssd != Some(true)
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

/// Legacy recordings merge on the staging SSD. Rolling recordings publish their
/// originals on the archive drive before the final merge. Only the single part
/// worker needs SSD scratch for those recordings, including after a cold start.
fn required_space<'a>(
    recordings: impl IntoIterator<Item = &'a BrowserRecording>,
    unverified: &HashSet<String>,
    volume: &Path,
    rolling: bool,
) -> u64 {
    let mut reserve = HEADROOM.saturating_add(MAX_SEGMENT);
    let mut needs_part = rolling;
    let mut resident = 0u64;
    for r in recordings {
        let Some(a) = &r.archive else { continue };
        if a.status == archive::ArchiveStatus::Complete
            || r.media_removed_at.is_some()
            || r.merge
                .as_ref()
                .is_some_and(|m| m.status == BrowserMergeStatus::Complete)
            || r.output_dir != a.source_dir
            || !Path::new(&a.source_dir).starts_with(volume)
        {
            continue;
        }
        if a.rolling {
            needs_part = true;
            // Cold index entries cannot acquire a part/merge job until recovery validates
            // their sources and receipts. Do not reserve a destination-side final on C.
            if !unverified.contains(&r.id) {
                let evicted = r.progressive.as_ref().map_or(0, |p| p.offloaded_bytes);
                resident = resident.saturating_add(r.bytes_written.saturating_sub(evicted));
            }
        } else {
            reserve = reserve.saturating_add(r.bytes_written);
        }
    }
    if needs_part {
        reserve.saturating_add(resident.max(parts::MAX_PART_BYTES))
    } else {
        reserve
    }
}

pub(super) fn check_staging_space(
    state: &State,
    volume: &Path,
    rolling: bool,
) -> Result<(), StreamError> {
    let required = required_space(&state.recordings, &state.unverified, volume, rolling);
    let free = fs2::available_space(volume).map_err(|_| storage())?;
    if free < required {
        let gib = 1024.0_f64.powi(3);
        let disk = volume
            .components()
            .next()
            .map(|p| p.as_os_str().to_string_lossy().replace(r"\\?\", ""))
            .unwrap_or_default();
        return Err(error("BROWSER_CAPTURE_DISK_FULL", &format!(
            "{disk} SSD 여유 공간 {:.1} GiB · 녹화에 필요한 공간 {:.1} GiB. 저장된 영상은 보존됩니다.",
            free as f64 / gib, required as f64 / gib
        ), true));
    }
    Ok(())
}

pub(super) fn ensure_headroom(
    state: &State,
    recording: &BrowserRecording,
) -> Result<(), StreamError> {
    let Some(archive) = &recording.archive else {
        return Ok(());
    };
    let volume = Path::new(&recording.output_dir)
        .ancestors()
        .nth(3)
        .ok_or_else(invalid)?;
    check_staging_space(state, volume, archive.rolling)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cold_rolling_history_only_reserves_one_local_part_but_legacy_merges_are_charged() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let mut r = store
            .begin(
                dir.path(),
                "0123456789abcdef0123456789abcdef",
                "fixture",
                "video/webm",
            )
            .unwrap();
        let base = PathBuf::from(&r.output_dir);
        let volume = base.ancestors().nth(3).unwrap();
        let mut a = archive::ArchiveTransfer::new(&base, dir.path());
        a.rolling = true;
        r.archive = Some(a);
        r.bytes_written = 14 * 1024 * 1024 * 1024;
        let mut entries = vec![r; 13];
        for (i, entry) in entries.iter_mut().enumerate() {
            entry.id = i.to_string();
        }
        let cold: HashSet<_> = entries.iter().map(|entry| entry.id.clone()).collect();
        let rolling_space = HEADROOM + MAX_SEGMENT + parts::MAX_PART_BYTES;
        assert_eq!(required_space(&entries, &cold, volume, true), rolling_space);
        assert!(rolling_space < 158 * 1024 * 1024 * 1024);
        assert!(rolling_space > 10 * 1024 * 1024 * 1024);
        let verified = HashSet::new();
        assert!(required_space(&entries, &verified, volume, true) > 158 * 1024 * 1024 * 1024);
        for entry in &mut entries {
            entry.progressive = Some(ProgressiveSummary {
                offloaded_bytes: entry.bytes_written,
                ..Default::default()
            });
        }
        assert_eq!(
            required_space(&entries, &verified, volume, true),
            rolling_space
        );
        entries[0].archive.as_mut().unwrap().rolling = false;
        assert_eq!(
            required_space(&entries, &cold, volume, true),
            rolling_space + entries[0].bytes_written
        );
        entries[0].archive.as_mut().unwrap().status = archive::ArchiveStatus::Complete;
        assert_eq!(
            required_space(&entries, &cold, volume, false),
            rolling_space
        );
        assert_eq!(
            required_space(&entries, &cold, &dir.path().join("another-drive"), false),
            HEADROOM + MAX_SEGMENT
        );
    }

    #[test]
    #[ignore = "reads the local CHZZK startup index without hydrating or modifying recordings"]
    fn inspect_local_staging_reserve() {
        let data = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("local.atsumi.next");
        let bytes = fs::read(data.join("streaming/browser/startup-index.json")).unwrap();
        #[derive(Deserialize)]
        struct Index {
            recordings: Vec<BrowserRecording>,
        }
        let index: Index = serde_json::from_slice(&bytes).unwrap();
        let volume = fs::canonicalize(data.join("ssd-recordings")).unwrap();
        let cold = index.recordings.iter().map(|r| r.id.clone()).collect();
        let needed = required_space(&index.recordings, &cold, &volume, true);
        let available = fs2::available_space(&volume).unwrap();
        println!(
            "recordings={} available_gib={:.2} required_gib={:.2} admitted={}",
            index.recordings.len(),
            available as f64 / (1u64 << 30) as f64,
            needed as f64 / (1u64 << 30) as f64,
            available >= needed
        );
    }

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
    fn inspection_distinguishes_destination_from_available_staging_without_writes() {
        let disks = vec![
            c("hdd", 100 * START_FREE, false),
            c("ssd", START_FREE, true),
        ];
        assert_eq!(status(Some(true), &disks).destination_kind, "ssd");
        assert_eq!(status(Some(false), &disks).destination_kind, "hdd");
        assert_eq!(status(None, &disks).destination_kind, "unknown");
        assert_eq!(status(None, &disks).detected_ssds, 1);
        assert!(status(None, &disks).staging_available);
        assert!(!status(Some(false), &[c("full-ssd", START_FREE - 1, true)]).staging_available);
        assert!(!needs_staging(true, Some(true)));
        assert!(needs_staging(true, Some(false)));
        assert!(needs_staging(true, None));
        assert!(!needs_staging(false, None));
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
