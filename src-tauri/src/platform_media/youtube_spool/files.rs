use super::{Error, Result};
use std::{
    fs::{self, File, OpenOptions},
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    volume: u64,
    index: u64,
}

fn metadata(file: &File, directory: bool) -> Result<(Identity, u64)> {
    let meta = file.metadata().map_err(|_| Error::Io)?;
    if meta.is_dir() != directory || (!directory && !meta.is_file()) {
        return Err(Error::Path);
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| Error::Path)?;
        if info.dwFileAttributes & 0x400 != 0 || (!directory && info.nNumberOfLinks != 1) {
            return Err(Error::Path);
        }
        Ok((
            Identity {
                volume: info.dwVolumeSerialNumber as u64,
                index: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
            },
            meta.len(),
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !directory && meta.nlink() != 1 {
            return Err(Error::Path);
        }
        Ok((
            Identity {
                volume: meta.dev(),
                index: meta.ino(),
            },
            meta.len(),
        ))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = meta;
        Err(Error::Path)
    }
}

fn lexical(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(Error::Path);
    }
    for part in path.components() {
        match part {
            Component::Normal(s) => {
                let s = s.to_str().ok_or(Error::Path)?;
                if s.contains(':') || s.ends_with('.') || s.ends_with(' ') || s.is_empty() {
                    return Err(Error::Path);
                }
            }
            Component::RootDir => (),
            #[cfg(windows)]
            Component::Prefix(p) => {
                if !matches!(p.kind(), std::path::Prefix::Disk(_)) {
                    return Err(Error::Path);
                }
            }
            _ => return Err(Error::Path),
        }
    }
    Ok(())
}
fn path_type(path: &Path, directory: bool) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|_| Error::Path)?;
    if meta.file_type().is_symlink()
        || meta.is_dir() != directory
        || (!directory && !meta.is_file())
    {
        return Err(Error::Path);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(Error::Path);
        }
    }
    Ok(())
}
fn options(directory: bool, create: bool, writer: bool, inspection: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(writer).create_new(create);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(if directory || inspection { 3 } else { 1 });
        options.custom_flags(0x00200000 | if directory { 0x02000000 } else { 0 });
        if directory || inspection {
            options.access_mode(0x80);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (directory, inspection);
    }
    options
}

pub(super) struct GuardedFile {
    pub file: File,
    path: PathBuf,
    identity: Identity,
}
impl GuardedFile {
    pub fn open(path: &Path, create: bool) -> Result<Self> {
        lexical(path)?;
        if !create {
            path_type(path, false)?;
        }
        let file = options(false, create, create, false)
            .open(path)
            .map_err(|_| Error::Path)?;
        let (identity, _) = metadata(&file, false)?;
        let result = Self {
            file,
            path: path.to_path_buf(),
            identity,
        };
        result.check()?;
        Ok(result)
    }
    pub fn check(&self) -> Result<u64> {
        path_type(&self.path, false)?;
        let (held, length) = metadata(&self.file, false)?;
        let inspection = options(false, false, false, true)
            .open(&self.path)
            .map_err(|_| Error::Path)?;
        let (current, _) = metadata(&inspection, false)?;
        if held != self.identity || current != self.identity {
            Err(Error::Path)
        } else {
            Ok(length)
        }
    }
}

pub(super) struct Directories {
    held: Vec<(PathBuf, File, Identity)>,
}
impl Directories {
    pub fn hold(path: &Path) -> Result<Self> {
        lexical(path)?;
        let mut result = Self { held: Vec::new() };
        let ancestors: Vec<_> = path.ancestors().collect();
        // Path depth, not media duration, bounds this collection.
        if ancestors.len() > 128 {
            return Err(Error::Path);
        }
        for ancestor in ancestors.into_iter().rev() {
            result.push(ancestor)?;
        }
        Ok(result)
    }
    fn push(&mut self, path: &Path) -> Result<()> {
        path_type(path, true)?;
        let file = options(true, false, false, false)
            .open(path)
            .map_err(|_| Error::Path)?;
        let (id, _) = metadata(&file, true)?;
        self.held.push((path.to_path_buf(), file, id));
        Ok(())
    }
    pub fn create_child(&mut self, path: &Path) -> Result<()> {
        self.check()?;
        if path.parent() != self.held.last().map(|v| v.0.as_path()) {
            return Err(Error::Path);
        }
        fs::create_dir(path).map_err(|_| Error::Path)?;
        self.push(path)?;
        self.check()
    }
    pub fn check(&self) -> Result<()> {
        for (path, file, expected) in &self.held {
            path_type(path, true)?;
            let inspection = options(true, false, false, true)
                .open(path)
                .map_err(|_| Error::Path)?;
            if metadata(file, true)?.0 != *expected || metadata(&inspection, true)?.0 != *expected {
                return Err(Error::Path);
            }
        }
        Ok(())
    }
}
