//! Leave HDD headroom for interactive reads, downloads and playback. This is
//! cooperative pacing for bulk verification, not a change to integrity checks.
//! SSDs/unknown devices are not rate-limited. No OS/system settings are changed.
use std::{
    fs::File,
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const HASH_BYTES_PER_SECOND: u64 = 24 * 1024 * 1024;
const MEDIA_BYTES_PER_SECOND: u64 = 16 * 1024 * 1024;
const CANCEL_POLL: Duration = Duration::from_millis(25);

/// Unknown is deliberately not treated as SSD. This does not alter I/O policy.
pub(crate) fn is_solid_state(path: &Path) -> Option<bool> {
    #[cfg(windows)]
    {
        native::rotational(path).map(|rotating| !rotating)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

struct Schedule {
    next: Instant,
}
impl Schedule {
    fn reserve(&mut self, now: Instant, bytes: usize, read_time: Duration) -> Instant {
        let start = self.next.max(now);
        // A slow seek/SMR read can occupy a disk at very low MB/s. Yield at
        // least its elapsed read time as well as enforcing the byte ceiling.
        let byte_time = Duration::from_secs_f64(bytes as f64 / HASH_BYTES_PER_SECOND as f64);
        self.next = start + byte_time.max(read_time);
        self.next
    }
}

#[derive(Clone, Default)]
pub(crate) struct BulkReadBudget(Option<Arc<Mutex<Schedule>>>);

impl BulkReadBudget {
    pub(crate) fn for_path(path: &Path) -> Self {
        #[cfg(windows)]
        {
            Self(native::budget(path))
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Self::default()
        }
    }

    pub(crate) fn for_file(file: &File) -> Self {
        #[cfg(windows)]
        {
            native::file_path(file).map_or_else(Self::default, |path| Self::for_path(&path))
        }
        #[cfg(not(windows))]
        {
            let _ = file;
            Self::default()
        }
    }

    /// Called outside repository/host locks. Returns false promptly on cancel.
    pub(crate) fn account(
        &self,
        bytes: usize,
        read_time: Duration,
        cancelled: impl Fn() -> bool,
    ) -> bool {
        if cancelled() {
            return false;
        }
        let Some(schedule) = &self.0 else {
            return true;
        };
        let deadline = schedule.lock().unwrap_or_else(|p| p.into_inner()).reserve(
            Instant::now(),
            bytes,
            read_time,
        );
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            if cancelled() {
                return false;
            }
            thread::sleep(remaining.min(CANCEL_POLL));
        }
        !cancelled()
    }
}

/// FFmpeg's input pacing is in media-seconds, derived from this file's bitrate.
/// It is used only for local remux/decode, never the live recorder/network path.
pub(crate) fn media_read_rate(path: &Path, bytes: u64, duration: f64) -> Option<String> {
    if BulkReadBudget::for_path(path).0.is_none()
        || bytes == 0
        || !duration.is_finite()
        || duration <= 0.0
    {
        return None;
    }
    Some(format!("{:.4}", media_rate(bytes, duration)))
}

fn media_rate(bytes: u64, duration: f64) -> f64 {
    (MEDIA_BYTES_PER_SECOND as f64 * duration / bytes as f64).clamp(0.05, 64.0)
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        collections::HashMap,
        ffi::OsString,
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            io::AsRawHandle,
        },
        path::PathBuf,
        sync::OnceLock,
    };
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{CloseHandle, HANDLE},
            Storage::FileSystem::{
                CreateFileW, GetFinalPathNameByHandleW, GetVolumeNameForVolumeMountPointW,
                GetVolumePathNameW, FILE_ATTRIBUTE_NORMAL, FILE_NAME_NORMALIZED, FILE_SHARE_READ,
                FILE_SHARE_WRITE, OPEN_EXISTING,
            },
            System::{
                Ioctl::{
                    PropertyStandardQuery, StorageDeviceSeekPenaltyProperty,
                    DEVICE_SEEK_PENALTY_DESCRIPTOR, IOCTL_STORAGE_QUERY_PROPERTY,
                    STORAGE_PROPERTY_QUERY,
                },
                IO::DeviceIoControl,
            },
        },
    };

    struct Cached {
        checked: Instant,
        schedule: Option<Arc<Mutex<Schedule>>>,
    }
    static VOLUMES: OnceLock<Mutex<HashMap<String, Cached>>> = OnceLock::new();

    pub(super) fn file_path(file: &File) -> Option<PathBuf> {
        let mut path = vec![0u16; 32768];
        let count = unsafe {
            GetFinalPathNameByHandleW(
                HANDLE(file.as_raw_handle()),
                &mut path,
                FILE_NAME_NORMALIZED,
            )
        } as usize;
        if count == 0 || count >= path.len() {
            return None;
        }
        Some(PathBuf::from(OsString::from_wide(&path[..count])))
    }

    pub(super) fn budget(path: &Path) -> Option<Arc<Mutex<Schedule>>> {
        let input: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut mount = vec![0u16; 32768];
        unsafe {
            GetVolumePathNameW(PCWSTR(input.as_ptr()), &mut mount).ok()?;
        }
        let len = mount.iter().position(|c| *c == 0)?;
        let mount_text = String::from_utf16_lossy(&mount[..len]);
        // File handles use extended DOS paths, callers may use ordinary paths.
        // They must share a single per-volume budget, not double its allowance.
        let key = mount_text
            .strip_prefix(r"\\?\")
            .unwrap_or(&mount_text)
            .to_ascii_lowercase();
        let volumes = VOLUMES.get_or_init(Default::default);
        {
            let cached = volumes.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(entry) = cached
                .get(&key)
                .filter(|entry| entry.checked.elapsed() < Duration::from_secs(300))
            {
                return entry.schedule.clone();
            }
        }
        // Never hold our global scheduling mutex while querying a device.
        let rotational = unsafe { seek_penalty(&mount) };
        let mut cached = volumes.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = cached
            .get(&key)
            .filter(|entry| entry.checked.elapsed() < Duration::from_secs(300))
        {
            return entry.schedule.clone();
        }
        let schedule = if rotational == Some(true) {
            cached
                .get(&key)
                .and_then(|e| e.schedule.clone())
                .or_else(|| {
                    Some(Arc::new(Mutex::new(Schedule {
                        next: Instant::now(),
                    })))
                })
        } else {
            None
        };
        tracing::info!(
            volume = key,
            ?rotational,
            hash_mib_per_second = if schedule.is_some() { 24 } else { 0 },
            "bulk storage read policy detected (zero means unthrottled)"
        );
        cached.insert(
            key,
            Cached {
                checked: Instant::now(),
                schedule: schedule.clone(),
            },
        );
        schedule
    }

    pub(super) fn rotational(path: &Path) -> Option<bool> {
        let input: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut mount = vec![0u16; 32768];
        unsafe {
            GetVolumePathNameW(PCWSTR(input.as_ptr()), &mut mount).ok()?;
            seek_penalty(&mount)
        }
    }

    unsafe fn seek_penalty(mount: &[u16]) -> Option<bool> {
        let mut volume = [0u16; 128];
        GetVolumeNameForVolumeMountPointW(PCWSTR(mount.as_ptr()), &mut volume).ok()?;
        let len = volume.iter().position(|c| *c == 0)?;
        if len < 2 {
            return None;
        }
        volume[len - 1] = 0; // Volume handle syntax omits the trailing backslash.
        let handle = CreateFileW(
            PCWSTR(volume.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .ok()?;
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceSeekPenaltyProperty,
            QueryType: PropertyStandardQuery,
            ..Default::default()
        };
        let mut descriptor = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
        let mut returned = 0;
        let result = DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some((&query as *const STORAGE_PROPERTY_QUERY).cast()),
            std::mem::size_of_val(&query) as u32,
            Some((&mut descriptor as *mut DEVICE_SEEK_PENALTY_DESCRIPTOR).cast()),
            std::mem::size_of_val(&descriptor) as u32,
            Some(&mut returned),
            None,
        );
        let _ = CloseHandle(handle);
        if result.is_err() || returned < std::mem::size_of_val(&descriptor) as u32 {
            return None;
        }
        Some(descriptor.IncursSeekPenalty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readers_share_one_schedule_and_idle_time_does_not_accumulate_credit() {
        let start = Instant::now();
        let mut schedule = Schedule { next: start };
        assert_eq!(
            schedule.reserve(start, HASH_BYTES_PER_SECOND as usize, Duration::ZERO),
            start + Duration::from_secs(1)
        );
        assert_eq!(
            schedule.reserve(start, HASH_BYTES_PER_SECOND as usize, Duration::ZERO),
            start + Duration::from_secs(2)
        );
        let later = start + Duration::from_secs(20);
        let first = schedule.reserve(later, 1024, Duration::ZERO);
        assert!(first > later && first < later + Duration::from_millis(1));
        assert!(schedule.reserve(later, 1024, Duration::ZERO) > first);
    }
    #[test]
    fn cancellation_never_waits_for_reserved_disk_time() {
        let budget = BulkReadBudget(Some(Arc::new(Mutex::new(Schedule {
            next: Instant::now() + Duration::from_secs(60),
        }))));
        assert!(!budget.account(128 * 1024, Duration::ZERO, || true));
        assert!(BulkReadBudget::default().account(1024, Duration::from_secs(30), || false));
    }
    #[test]
    fn slow_low_byte_reads_still_yield_disk_time() {
        let now = Instant::now();
        let mut schedule = Schedule { next: now };
        assert_eq!(
            schedule.reserve(now, 4096, Duration::from_millis(100)),
            now + Duration::from_millis(100)
        );
        assert_eq!(
            schedule.reserve(now, 0, Duration::from_millis(80)),
            now + Duration::from_millis(180)
        );
    }
    #[test]
    fn media_pacing_is_derived_from_bitrate_not_playback_speed() {
        assert_eq!(media_rate(1024 * 1024 * 100, 100.0), 16.0);
        assert_eq!(media_rate(4 * 1024 * 1024 * 100, 100.0), 4.0);
        assert_eq!(media_rate(1024, 100.0), 64.0);
    }
    #[test]
    #[ignore = "read-only local drive policy diagnostic; set ATSUMI_IO_PROBE_PATH"]
    fn inspect_local_volume_policy() {
        let path = std::env::var_os("ATSUMI_IO_PROBE_PATH").expect("probe path required");
        let budget = BulkReadBudget::for_path(Path::new(&path));
        println!("HDD background pacing enabled: {}", budget.0.is_some());
    }
}
