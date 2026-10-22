//! Dormant, source-local lease for pinned FFprobe/FFmpeg executables and DLLs.
//! This does not run a tool or turn a filename into recording authority.
use super::files::{Directories, GuardedFile};
use crate::platform_media::PlatformMediaTools;
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

pub(super) const PACKAGE: &str = "ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0";
pub(super) const ARCHIVE_SHA256: &str =
    "40eec25b2f55dcad7e4d4e640919b920d29818b56fdaf9353ce1fd8adefc9d6b";
const SCRATCH: usize = 131_072;
const OS_BUFFER: usize = 32_768;

#[derive(Clone, Copy)]
struct Pin {
    name: &'static str,
    length: u64,
    sha256: &'static str,
}
// Derived from the approved archive through prepare_media_tools.ps1
// -VerifyOnly -ReturnManifest (2026-10-15), not from arbitrary installed files.
const PINS: [Pin; 9] = [
    Pin {
        name: "avcodec-63.dll",
        length: 90_356_736,
        sha256: "f103787574f811bd6529f23705b3da33e02b54f045bc9380e3fcd2c8e6bc0581",
    },
    Pin {
        name: "avdevice-63.dll",
        length: 4_047_872,
        sha256: "e294086098fbd4ae5804f485dbf4578da5270aa30916bfe77455e450875eefff",
    },
    Pin {
        name: "avfilter-12.dll",
        length: 30_806_016,
        sha256: "d2470d43d55886a84dc8a9079399cddc8513d3149eea4892f17a063aceaaa1c7",
    },
    Pin {
        name: "avformat-63.dll",
        length: 22_668_800,
        sha256: "8ba080006b44e6d8fb4d1067da0d966f08c1f5c1c08ae83b0d4f33f56d1d27fd",
    },
    Pin {
        name: "avutil-61.dll",
        length: 3_014_144,
        sha256: "97f8e9c61a81d4069339596ec331da41e4b68073d14440c79134d367db3a411a",
    },
    Pin {
        name: "ffprobe.exe",
        length: 228_864,
        sha256: "713b3273eff80a441666ec66bc7ea6b339962177f38a4d57cf2f8a4d71c738fa",
    },
    Pin {
        name: "swresample-7.dll",
        length: 733_696,
        sha256: "a6bf9c0612b68cc13e4869cc8a4de57dda6f3cd1282419a71827047032db350d",
    },
    Pin {
        name: "swscale-10.dll",
        length: 2_330_112,
        sha256: "e9479dc5a706a3dcb93c331cdc9b3ad5c8f396b50c8afc1b871e4d226fa1a661",
    },
    Pin {
        name: "ffmpeg.exe",
        length: 538_112,
        sha256: "c2a51bdad17d321c9c11768a4bc6137ac6c666d62ba82fe4b4050fbd1db9d2a0",
    },
];
const ENTRIES: [&str; 10] = [
    "avcodec-63.dll",
    "avdevice-63.dll",
    "avfilter-12.dll",
    "avformat-63.dll",
    "avutil-61.dll",
    "ffprobe.exe",
    "swresample-7.dll",
    "swscale-10.dll",
    "ffmpeg.exe",
    "ffplay.exe",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fault {
    UnsupportedHost,
    Path,
    Entries,
    Integrity,
    Changed,
    Loader,
    Environment,
    Io,
    Cancelled,
    Deadline,
}
type Result<T> = std::result::Result<T, Fault>;

fn checkpoint(cancel: &AtomicBool, deadline: Instant) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err(Fault::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(Fault::Deadline);
    }
    Ok(())
}

fn native_bin(tools: &PlatformMediaTools) -> Result<&Path> {
    // The eventual caller must be the service's native path resolver, never
    // IPC or deserialization. These syntax checks do NOT establish provenance.
    if !tools.ffprobe.is_absolute() || tools.ffprobe.file_name() != Some(OsStr::new("ffprobe.exe"))
    {
        return Err(Fault::Path);
    }
    let bin = tools.ffprobe.parent().ok_or(Fault::Path)?;
    if bin.file_name() != Some(OsStr::new("bin")) {
        return Err(Fault::Path);
    }
    Ok(bin)
}

fn exact_entries(bin: &Path, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
    checkpoint(cancel, deadline)?;
    let mut seen = [false; ENTRIES.len()];
    for entry in fs::read_dir(bin).map_err(|_| Fault::Path)? {
        checkpoint(cancel, deadline)?;
        let entry = entry.map_err(|_| Fault::Path)?;
        let name = entry.file_name();
        let index = name
            .to_str()
            .and_then(|name| ENTRIES.iter().position(|p| *p == name))
            .ok_or(Fault::Entries)?;
        if seen[index] || !entry.file_type().map_err(|_| Fault::Path)?.is_file() {
            return Err(Fault::Entries);
        }
        seen[index] = true;
    }
    checkpoint(cancel, deadline)?;
    if seen.iter().all(|v| *v) {
        Ok(())
    } else {
        Err(Fault::Entries)
    }
}

fn hash_exact(
    reader: &mut impl Read,
    pin: Pin,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<()> {
    let mut scratch = vec![0u8; SCRATCH];
    let mut hash = Sha256::new();
    let mut remaining = pin.length;
    while remaining != 0 {
        checkpoint(cancel, deadline)?;
        let count = reader
            .read(&mut scratch[..remaining.min(SCRATCH as u64) as usize])
            .map_err(|_| Fault::Io)?;
        checkpoint(cancel, deadline)?;
        if count == 0 {
            return Err(Fault::Integrity);
        }
        hash.update(&scratch[..count]);
        remaining -= count as u64;
    }
    checkpoint(cancel, deadline)?;
    if reader.read(&mut scratch[..1]).map_err(|_| Fault::Io)? != 0 {
        return Err(Fault::Integrity);
    }
    checkpoint(cancel, deadline)?;
    if format!("{:x}", hash.finalize()) != pin.sha256 {
        return Err(Fault::Integrity);
    }
    Ok(())
}

fn empty_loader_result(length: u32, last_error: u32, first: u16) -> Result<()> {
    if length == 0 && last_error == 0 && first == 0 {
        Ok(())
    } else {
        Err(Fault::Loader)
    }
}

fn windows_directory_result(buffer: &[u16], length: u32) -> Result<OsString> {
    let length = length as usize;
    if length == 0 || length >= buffer.len() || buffer[length] != 0 || buffer[..length].contains(&0)
    {
        return Err(Fault::Environment);
    }
    let value = String::from_utf16(&buffer[..length]).map_err(|_| Fault::Environment)?;
    Ok(value.into())
}

#[cfg(windows)]
fn loader_guard() -> Result<()> {
    use windows::Win32::{
        Foundation::{GetLastError, SetLastError, WIN32_ERROR},
        System::LibraryLoader::GetDllDirectoryW,
    };
    let mut buffer = vec![0u16; OS_BUFFER];
    // Empty is both the legitimate unset result and an API failure sentinel.
    // Last-error is thread-local; never alter the process DLL search directory.
    unsafe {
        SetLastError(WIN32_ERROR(0));
    }
    let length = unsafe { GetDllDirectoryW(Some(&mut buffer)) };
    let last_error = unsafe { GetLastError() }.0;
    empty_loader_result(length, last_error, buffer[0])
}

#[cfg(windows)]
fn windows_directory() -> Result<OsString> {
    use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
    let mut buffer = vec![0u16; OS_BUFFER];
    let length = unsafe { GetWindowsDirectoryW(Some(&mut buffer)) };
    let value = windows_directory_result(&buffer, length)?;
    use std::path::{Component, Prefix};
    let path = Path::new(&value);
    if !path.is_absolute()
        || !matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)))
    {
        return Err(Fault::Environment);
    }
    Ok(value)
}

/// Neither serializable nor constructible by a browser. All ten file handles
/// remain held, but only the nine pinned runtime files are execution evidence.
/// This is NOT a sandbox against a compromised OS or hostile same-user races.
pub(super) struct ManagedProbe {
    executable: PathBuf,
    ffmpeg: PathBuf,
    bin: PathBuf,
    directories: Directories,
    files: Vec<(GuardedFile, u64)>,
    windows: OsString,
}

impl ManagedProbe {
    pub(super) fn open(
        tools: &PlatformMediaTools,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Self> {
        #[cfg(not(windows))]
        {
            let _ = (tools, cancel, deadline);
            Err(Fault::UnsupportedHost)
        }
        #[cfg(windows)]
        {
            #[cfg(test)]
            {
                Self::open_inner(tools, cancel, deadline, None)
            }
            #[cfg(not(test))]
            {
                Self::open_inner(tools, cancel, deadline)
            }
        }
    }

    #[cfg(windows)]
    fn open_inner(
        tools: &PlatformMediaTools,
        cancel: &AtomicBool,
        deadline: Instant,
        #[cfg(test)] fixture_pins: Option<&[Pin; 9]>,
    ) -> Result<Self> {
        checkpoint(cancel, deadline)?;
        loader_guard()?;
        let windows = windows_directory()?;
        let bin = native_bin(tools)?;
        let directories = Directories::hold(bin).map_err(|_| Fault::Path)?;
        exact_entries(bin, cancel, deadline)?;
        let pins = &PINS;
        #[cfg(test)]
        let pins = fixture_pins.unwrap_or(pins);
        let mut files = Vec::with_capacity(ENTRIES.len());
        for name in ENTRIES {
            checkpoint(cancel, deadline)?;
            let mut held = GuardedFile::open(&bin.join(name), false).map_err(|_| Fault::Path)?;
            let length = held.check().map_err(|_| Fault::Path)?;
            if let Some(pin) = pins.iter().find(|pin| pin.name == name) {
                if length != pin.length {
                    return Err(Fault::Integrity);
                }
                hash_exact(&mut held.file, *pin, cancel, deadline)?;
            }
            if held.check().map_err(|_| Fault::Changed)? != length {
                return Err(Fault::Changed);
            }
            files.push((held, length));
        }
        let result = Self {
            executable: tools.ffprobe.clone(),
            // Only the verified fixed sibling is eligible; tools.ffmpeg may
            // point elsewhere and is deliberately not an execution authority.
            ffmpeg: bin.join("ffmpeg.exe"),
            bin: bin.into(),
            directories,
            files,
            windows,
        };
        result.check(cancel, deadline)?;
        Ok(result)
    }

    /// Reuse only while the original deny-write/delete handles remain held.
    /// No repeated full-library hashing and no cached path-only success flag.
    pub(super) fn check(&self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        checkpoint(cancel, deadline)?;
        #[cfg(not(windows))]
        {
            return Err(Fault::UnsupportedHost);
        }
        #[cfg(windows)]
        {
            loader_guard()?;
            self.directories.check().map_err(|_| Fault::Changed)?;
            exact_entries(&self.bin, cancel, deadline)?;
            for (file, length) in &self.files {
                checkpoint(cancel, deadline)?;
                if file.check().map_err(|_| Fault::Changed)? != *length {
                    return Err(Fault::Changed);
                }
            }
            checkpoint(cancel, deadline)
        }
    }

    pub(super) fn executable(&self) -> &Path {
        &self.executable
    }
    pub(super) fn ffmpeg(&self) -> &Path {
        &self.ffmpeg
    }
    pub(super) fn cwd(&self) -> &Path {
        &self.bin
    }
    /// A future approved runner must call env_clear before applying only these.
    pub(super) fn environment(&self) -> [(&'static str, &OsStr); 2] {
        [("SystemRoot", &self.windows), ("WINDIR", &self.windows)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Cursor, time::Duration};
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(30)
    }
    fn pin() -> Pin {
        Pin {
            name: "ffprobe.exe",
            length: 3,
            sha256: ABC,
        }
    }
    fn tools(path: PathBuf) -> PlatformMediaTools {
        PlatformMediaTools {
            ffprobe: path,
            ffmpeg: PathBuf::new(),
            yt_dlp: PathBuf::new(),
            deno: PathBuf::new(),
        }
    }

    #[test]
    fn fixed_pins_have_exact_unique_members_and_valid_bounds() {
        assert_eq!(
            PACKAGE,
            "ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0"
        );
        assert_eq!(ARCHIVE_SHA256.len(), 64);
        for (index, pin) in PINS.iter().enumerate() {
            assert_eq!(pin.name, ENTRIES[index]);
            assert!(pin.length > 0 && pin.length < 1_073_741_824);
            assert_eq!(pin.sha256.len(), 64);
            assert!(pin
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
            assert!(!ENTRIES[..index].contains(&pin.name));
        }
        assert_eq!(PINS.len(), 9);
        assert_eq!(&ENTRIES[9..], &["ffplay.exe"]);
    }

    #[test]
    fn hash_requires_complete_exact_length_and_content() {
        let cancel = AtomicBool::new(false);
        assert_eq!(
            hash_exact(&mut Cursor::new(b"abc"), pin(), &cancel, deadline()),
            Ok(())
        );
        for bytes in [b"ab".as_slice(), b"abcd", b"abd"] {
            assert_eq!(
                hash_exact(&mut Cursor::new(bytes), pin(), &cancel, deadline()),
                Err(Fault::Integrity)
            );
        }
        struct Short(Cursor<&'static [u8]>);
        impl Read for Short {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                self.0.read(&mut out[..1])
            }
        }
        assert_eq!(
            hash_exact(&mut Short(Cursor::new(b"abc")), pin(), &cancel, deadline()),
            Ok(())
        );
    }

    #[test]
    fn cancellation_during_hash_and_expired_budget_fail() {
        let cancel = AtomicBool::new(false);
        struct Revoke<'a>(&'a AtomicBool, Cursor<&'static [u8]>);
        impl Read for Revoke<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let n = self.1.read(out)?;
                self.0.store(true, Ordering::Release);
                Ok(n)
            }
        }
        assert_eq!(
            hash_exact(
                &mut Revoke(&cancel, Cursor::new(b"abc")),
                pin(),
                &cancel,
                deadline()
            ),
            Err(Fault::Cancelled)
        );
        assert_eq!(
            hash_exact(
                &mut Cursor::new(b"abc"),
                pin(),
                &AtomicBool::new(false),
                Instant::now()
            ),
            Err(Fault::Deadline)
        );
    }

    #[test]
    fn read_errors_and_revocation_at_eof_never_issue_a_lease() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::Other.into())
            }
        }
        assert_eq!(
            hash_exact(&mut Broken, pin(), &AtomicBool::new(false), deadline()),
            Err(Fault::Io)
        );
        struct FinalRevoke<'a>(&'a AtomicBool, Cursor<&'static [u8]>);
        impl Read for FinalRevoke<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let n = self.1.read(out)?;
                if n == 0 {
                    self.0.store(true, Ordering::Release);
                }
                Ok(n)
            }
        }
        let cancel = AtomicBool::new(false);
        assert_eq!(
            hash_exact(
                &mut FinalRevoke(&cancel, Cursor::new(b"abc")),
                pin(),
                &cancel,
                deadline()
            ),
            Err(Fault::Cancelled)
        );
    }

    #[test]
    fn loader_and_os_environment_decoding_are_closed_without_global_mutation() {
        assert_eq!(empty_loader_result(0, 0, 0), Ok(()));
        for (n, e, first) in [(0, 5, 0), (1, 0, 65), (32_768, 0, 0), (0, 0, 65)] {
            assert_eq!(empty_loader_result(n, e, first), Err(Fault::Loader));
        }
        let mut value: Vec<_> = "C:\\Windows".encode_utf16().collect();
        let n = value.len() as u32;
        value.push(0);
        assert_eq!(
            windows_directory_result(&value, n),
            Ok(OsString::from("C:\\Windows"))
        );
        for length in [0, n + 1, u32::MAX] {
            assert_eq!(
                windows_directory_result(&value, length),
                Err(Fault::Environment)
            );
        }
        value[1] = 0;
        assert_eq!(windows_directory_result(&value, n), Err(Fault::Environment));
        assert_eq!(
            windows_directory_result(&[0xd800, 0], 1),
            Err(Fault::Environment)
        );
    }

    #[cfg(windows)]
    struct Fixture {
        root: tempfile::TempDir,
        tools: PlatformMediaTools,
        pins: [Pin; 9],
    }
    #[cfg(windows)]
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let bin = root.path().join("bin");
            fs::create_dir(&bin).unwrap();
            for name in ENTRIES {
                fs::write(bin.join(name), b"abc").unwrap();
            }
            let pins = PINS.map(|p| Pin {
                name: p.name,
                ..pin()
            });
            Self {
                root,
                tools: tools(bin.join("ffprobe.exe")),
                pins,
            }
        }
        fn open(&self, cancel: &AtomicBool) -> Result<ManagedProbe> {
            ManagedProbe::open_inner(&self.tools, cancel, deadline(), Some(&self.pins))
        }
        fn bin(&self) -> &Path {
            self.tools.ffprobe.parent().unwrap()
        }
    }

    #[test]
    #[cfg(windows)]
    fn exact_fixture_lease_keeps_all_handles_and_only_os_environment() {
        let mut f = Fixture::new();
        f.tools.ffmpeg = f.root.path().join("untrusted-alternate.exe");
        let cancel = AtomicBool::new(false);
        let lease = f.open(&cancel).unwrap();
        assert_eq!(lease.files.len(), 10);
        assert_eq!(lease.executable(), f.tools.ffprobe);
        assert_eq!(lease.ffmpeg(), f.bin().join("ffmpeg.exe"));
        assert_ne!(lease.ffmpeg(), f.tools.ffmpeg);
        assert_eq!(lease.cwd(), f.bin());
        assert_eq!(
            lease.environment().map(|(k, _)| k),
            ["SystemRoot", "WINDIR"]
        );
        assert!(lease.environment().iter().all(|(_, v)| !v.is_empty()));
        assert_eq!(lease.check(&cancel, deadline()), Ok(()));
        assert!(fs::OpenOptions::new()
            .write(true)
            .open(&f.tools.ffprobe)
            .is_err());
        assert!(fs::OpenOptions::new()
            .write(true)
            .open(lease.ffmpeg())
            .is_err());
        assert!(fs::rename(f.bin().join("avcodec-63.dll"), f.bin().join("changed.dll")).is_err());
        assert!(fs::OpenOptions::new()
            .write(true)
            .open(f.bin().join("ffplay.exe"))
            .is_err());
        assert!(fs::rename(f.bin(), f.root.path().join("moved")).is_err());
        cancel.store(true, Ordering::Release);
        assert_eq!(lease.check(&cancel, deadline()), Err(Fault::Cancelled));
        assert_eq!(
            lease.check(&AtomicBool::new(false), Instant::now()),
            Err(Fault::Deadline)
        );
    }

    #[test]
    #[cfg(windows)]
    fn tampered_ffmpeg_sibling_cannot_issue_a_lease() {
        for bytes in [b"abd".as_slice(), b"abcd"] {
            let f = Fixture::new();
            fs::write(f.bin().join("ffmpeg.exe"), bytes).unwrap();
            assert!(matches!(
                f.open(&AtomicBool::new(false)),
                Err(Fault::Integrity)
            ));
        }
    }

    #[test]
    #[cfg(windows)]
    fn wrong_hash_length_missing_extra_and_directory_fail_closed() {
        let f = Fixture::new();
        assert!(matches!(
            ManagedProbe::open(&f.tools, &AtomicBool::new(false), deadline()),
            Err(Fault::Integrity)
        ));
        let f = Fixture::new();
        fs::write(&f.tools.ffprobe, b"abd").unwrap();
        assert!(matches!(
            f.open(&AtomicBool::new(false)),
            Err(Fault::Integrity)
        ));
        let f = Fixture::new();
        fs::write(&f.tools.ffprobe, b"abcd").unwrap();
        assert!(matches!(
            f.open(&AtomicBool::new(false)),
            Err(Fault::Integrity)
        ));
        let f = Fixture::new();
        fs::remove_file(f.bin().join("ffplay.exe")).unwrap();
        assert!(matches!(
            f.open(&AtomicBool::new(false)),
            Err(Fault::Entries)
        ));
        for name in ["rogue.dll", "ffprobe.exe.local", "ffprobe.exe.manifest"] {
            let f = Fixture::new();
            fs::write(f.bin().join(name), b"fixture").unwrap();
            assert!(matches!(
                f.open(&AtomicBool::new(false)),
                Err(Fault::Entries)
            ));
        }
        let f = Fixture::new();
        fs::create_dir(f.bin().join("unknown")).unwrap();
        assert!(matches!(
            f.open(&AtomicBool::new(false)),
            Err(Fault::Entries)
        ));
    }

    #[test]
    #[cfg(windows)]
    fn noncanonical_path_hardlinks_and_changed_directory_set_fail() {
        let f = Fixture::new();
        for path in [
            PathBuf::from("bin/ffprobe.exe"),
            f.bin().join("other.exe"),
            f.root.path().join("ffprobe.exe"),
        ] {
            assert_eq!(native_bin(&tools(path)), Err(Fault::Path));
        }
        fs::hard_link(&f.tools.ffprobe, f.root.path().join("alias.exe")).unwrap();
        assert!(matches!(f.open(&AtomicBool::new(false)), Err(Fault::Path)));
        let f = Fixture::new();
        let cancel = AtomicBool::new(false);
        let lease = f.open(&cancel).unwrap();
        // Directory holds prevent substitution, not a hostile user's insertion.
        // The fixed entry recheck must still catch a newly added file.
        fs::write(f.bin().join("rogue.dll"), b"fixture").unwrap();
        assert_eq!(lease.check(&cancel, deadline()), Err(Fault::Entries));
    }
}
