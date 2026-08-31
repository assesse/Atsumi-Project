use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageAreaUsage {
    pub bytes: u64,
    pub exists: bool,
    pub scan_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageVolumeUsage {
    pub root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_bytes: Option<u64>,
    pub atsumi_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsageSnapshot {
    pub memory_cache_bytes: u64,
    pub disk_cache: StorageAreaUsage,
    pub app_data: StorageAreaUsage,
    pub downloads: StorageAreaUsage,
    pub volumes: Vec<StorageVolumeUsage>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct DirectoryUsage {
    bytes: u64,
    exists: bool,
    complete: bool,
}

#[derive(Debug)]
struct VolumeAccumulator {
    root: String,
    total_bytes: Option<u64>,
    available_bytes: Option<u64>,
    paths: Vec<(PathBuf, u64)>,
}

pub fn collect_storage_usage(
    data_dir: &Path,
    download_root: Option<&Path>,
    memory_cache_bytes: u64,
) -> StorageUsageSnapshot {
    let mut warnings = Vec::new();
    let data_usage = directory_usage(data_dir);
    let disk_cache_path = data_dir.join("detail-original");
    let disk_cache_usage = directory_usage(&disk_cache_path);
    let app_data_bytes = data_usage.bytes.saturating_sub(disk_cache_usage.bytes);

    if !data_usage.complete {
        warnings
            .push("일부 앱 데이터 파일을 읽지 못해 표시 용량이 실제보다 작을 수 있습니다.".into());
    }
    if disk_cache_usage.exists && !disk_cache_usage.complete {
        warnings.push("일부 디스크 임시 캐시 파일을 읽지 못했습니다.".into());
    }

    let configured_download_root = download_root.filter(|path| !path.as_os_str().is_empty());
    let download_usage = configured_download_root.map_or(
        DirectoryUsage {
            bytes: 0,
            exists: false,
            complete: true,
        },
        directory_usage,
    );
    if configured_download_root.is_some() && !download_usage.exists {
        warnings.push("설정된 다운로드 폴더가 아직 존재하지 않습니다.".into());
    } else if !download_usage.complete {
        warnings
            .push("일부 다운로드 파일을 읽지 못해 표시 용량이 실제보다 작을 수 있습니다.".into());
    }

    let mut volumes = Vec::<VolumeAccumulator>::new();
    add_physical_location(&mut volumes, data_dir, data_usage.bytes, &mut warnings);
    if let Some(download_root) = configured_download_root {
        add_physical_location(
            &mut volumes,
            download_root,
            download_usage.bytes,
            &mut warnings,
        );
    }

    let data_volume_root = volume_root_for_path(data_dir);
    let download_volume_root = configured_download_root.and_then(volume_root_for_path);
    let volumes = volumes
        .into_iter()
        .map(|volume| StorageVolumeUsage {
            root: volume.root,
            total_bytes: volume.total_bytes,
            available_bytes: volume.available_bytes,
            atsumi_bytes: unique_physical_bytes(&volume.paths),
        })
        .collect();

    StorageUsageSnapshot {
        memory_cache_bytes,
        disk_cache: StorageAreaUsage {
            bytes: disk_cache_usage.bytes,
            exists: disk_cache_usage.exists,
            scan_complete: disk_cache_usage.complete,
            volume_root: data_volume_root.clone(),
        },
        app_data: StorageAreaUsage {
            bytes: app_data_bytes,
            exists: data_usage.exists,
            scan_complete: data_usage.complete,
            volume_root: data_volume_root,
        },
        downloads: StorageAreaUsage {
            bytes: download_usage.bytes,
            exists: download_usage.exists,
            scan_complete: download_usage.complete,
            volume_root: download_volume_root,
        },
        volumes,
        warnings,
    }
}

fn directory_usage(root: &Path) -> DirectoryUsage {
    let root_metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return DirectoryUsage {
                bytes: 0,
                exists: false,
                complete: true,
            };
        }
        Err(_) => {
            return DirectoryUsage {
                bytes: 0,
                exists: true,
                complete: false,
            };
        }
    };
    // A configured root may itself be a Windows junction or a symlink. Follow
    // that single, explicit root, while still refusing links discovered below
    // it so a scan cannot escape into unrelated trees or loop indefinitely.
    let scan_root = if root_metadata.file_type().is_symlink() {
        match fs::canonicalize(root) {
            Ok(path) => path,
            Err(_) => {
                return DirectoryUsage {
                    bytes: 0,
                    exists: true,
                    complete: false,
                };
            }
        }
    } else {
        root.to_path_buf()
    };
    let metadata = match fs::metadata(&scan_root) {
        Ok(metadata) => metadata,
        Err(_) => {
            return DirectoryUsage {
                bytes: 0,
                exists: true,
                complete: false,
            };
        }
    };
    if metadata.is_file() {
        return DirectoryUsage {
            bytes: metadata.len(),
            exists: true,
            complete: true,
        };
    }

    let mut bytes = 0_u64;
    let mut complete = true;
    let mut pending = vec![scan_root];
    while let Some(directory) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                complete = false;
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    complete = false;
                    continue;
                }
            };
            let metadata = match fs::symlink_metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    complete = false;
                    continue;
                }
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                bytes = bytes.saturating_add(metadata.len());
            }
        }
    }
    DirectoryUsage {
        bytes,
        exists: true,
        complete,
    }
}

fn add_physical_location(
    volumes: &mut Vec<VolumeAccumulator>,
    path: &Path,
    bytes: u64,
    warnings: &mut Vec<String>,
) {
    let Some(root) = volume_root_for_path(path) else {
        warnings.push("저장 위치의 디스크를 확인하지 못했습니다.".into());
        return;
    };
    let key = root.to_lowercase();
    let index = volumes
        .iter()
        .position(|volume| volume.root.to_lowercase() == key);
    let index = index.unwrap_or_else(|| {
        let disk = disk_capacity(path);
        if disk.is_none() {
            warnings.push(format!("{root} 디스크의 전체·남은 용량을 읽지 못했습니다."));
        }
        let (total_bytes, available_bytes) = disk.unzip();
        volumes.push(VolumeAccumulator {
            root: root.clone(),
            total_bytes,
            available_bytes,
            paths: Vec::new(),
        });
        volumes.len() - 1
    });
    volumes[index]
        .paths
        .push((normalized_physical_path(path), bytes));
}

fn unique_physical_bytes(paths: &[(PathBuf, u64)]) -> u64 {
    let mut retained = Vec::<(PathBuf, u64)>::new();
    for (path, bytes) in paths {
        if retained
            .iter()
            .any(|(existing, _)| path.starts_with(existing))
        {
            continue;
        }
        retained.retain(|(existing, _)| !existing.starts_with(path));
        retained.push((path.clone(), *bytes));
    }
    retained
        .into_iter()
        .fold(0_u64, |total, (_, bytes)| total.saturating_add(bytes))
}

fn normalized_physical_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn nearest_existing_path(path: &Path) -> Option<&Path> {
    path.ancestors().find(|candidate| candidate.exists())
}

#[cfg(not(windows))]
fn volume_root_for_path(path: &Path) -> Option<String> {
    let existing = nearest_existing_path(path)?;
    let root = existing.ancestors().last()?;
    let display = root.to_string_lossy().into_owned();
    (!display.is_empty()).then_some(display)
}

#[cfg(windows)]
fn volume_root_for_path(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStringExt;

    use windows::{core::HSTRING, Win32::Storage::FileSystem::GetVolumePathNameW};

    let existing = nearest_existing_path(path)?;
    let wide_path = HSTRING::from(existing.to_string_lossy().as_ref());
    let mut volume_path = vec![0_u16; 32_768];
    // SAFETY: `volume_path` is a live writable UTF-16 buffer and `wide_path`
    // remains NUL-terminated for the duration of this synchronous API call.
    unsafe { GetVolumePathNameW(&wide_path, &mut volume_path).ok()? };
    let length = volume_path
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(volume_path.len());
    let display = std::ffi::OsString::from_wide(&volume_path[..length])
        .to_string_lossy()
        .into_owned();
    (!display.is_empty()).then_some(display)
}

#[cfg(windows)]
fn disk_capacity(path: &Path) -> Option<(u64, u64)> {
    use windows::{core::HSTRING, Win32::Storage::FileSystem::GetDiskFreeSpaceExW};

    let existing = nearest_existing_path(path)?;
    let wide_path = HSTRING::from(existing.to_string_lossy().as_ref());
    let mut available = 0_u64;
    let mut total = 0_u64;
    let mut total_free = 0_u64;
    // SAFETY: All output pointers reference live u64 values and the HSTRING is
    // NUL-terminated for the duration of this synchronous Windows API call.
    unsafe {
        GetDiskFreeSpaceExW(
            &wide_path,
            Some(&mut available),
            Some(&mut total),
            Some(&mut total_free),
        )
        .ok()?;
    }
    Some((total, available))
}

#[cfg(not(windows))]
fn disk_capacity(_path: &Path) -> Option<(u64, u64)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_transient_cache_app_data_and_download_files() {
        let temporary = tempfile::tempdir().unwrap();
        let data_dir = temporary.path().join("data");
        let cache_dir = data_dir.join("detail-original");
        let download_root = temporary.path().join("downloads");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::create_dir_all(download_root.join("album")).unwrap();
        fs::write(data_dir.join("atsumi-next.sqlite3"), [0_u8; 5]).unwrap();
        fs::write(cache_dir.join("preview.webp"), [0_u8; 3]).unwrap();
        fs::write(download_root.join("album/0001.webp"), [0_u8; 7]).unwrap();

        let usage = collect_storage_usage(&data_dir, Some(&download_root), 11);

        assert_eq!(usage.memory_cache_bytes, 11);
        assert_eq!(usage.disk_cache.bytes, 3);
        assert_eq!(usage.app_data.bytes, 5);
        assert_eq!(usage.downloads.bytes, 7);
        assert!(usage.disk_cache.scan_complete);
        assert!(usage.downloads.scan_complete);
        assert_eq!(usage.volumes.len(), 1);
        assert_eq!(usage.volumes[0].atsumi_bytes, 15);
    }

    #[test]
    fn overlapping_roots_are_counted_once_on_the_volume() {
        let temporary = tempfile::tempdir().unwrap();
        let data_dir = temporary.path().join("data");
        let cache_dir = data_dir.join("detail-original");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(data_dir.join("atsumi-next.sqlite3"), [0_u8; 5]).unwrap();
        fs::write(cache_dir.join("preview.webp"), [0_u8; 3]).unwrap();

        let usage = collect_storage_usage(&data_dir, Some(&data_dir), 0);

        assert_eq!(usage.downloads.bytes, 8);
        assert_eq!(usage.volumes.len(), 1);
        assert_eq!(usage.volumes[0].atsumi_bytes, 8);
    }

    #[test]
    fn missing_download_root_is_nonfatal_and_reported() {
        let temporary = tempfile::tempdir().unwrap();
        let data_dir = temporary.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let missing = temporary.path().join("missing/downloads");

        let usage = collect_storage_usage(&data_dir, Some(&missing), 0);

        assert!(!usage.downloads.exists);
        assert_eq!(usage.downloads.bytes, 0);
        assert!(usage.downloads.scan_complete);
        assert!(usage
            .warnings
            .iter()
            .any(|warning| warning.contains("아직 존재하지 않습니다")));
    }
}
