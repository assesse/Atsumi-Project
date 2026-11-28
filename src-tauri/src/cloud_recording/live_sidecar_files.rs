//! Immutable lightweight history files for one native-owned relay. No media or network.
use super::super::relay_timing_read::{cancelled, pinned, FileProof, TimingReadError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

type Result<T> = std::result::Result<T, TimingReadError>;
const NAME: &str = "relay-history-seal.json";
const MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
// Existing SOOP multipart writer: 512 parts / 4 GiB, plus 256 MiB receive
// ledger and 128 MiB assets. CHZZK: <=4096 assets and <=64 MiB raw text.
const FILES: usize = 8192;
const FILE_BYTES: u64 = 256 * 1024 * 1024;
const TOTAL_BYTES: u64 = 5 * 1024 * 1024 * 1024;
fn pending_name(name: &str) -> bool {
    name.strip_prefix(".relay-history-").or_else(|| name.strip_prefix(".relay-frame-reference-"))
        .and_then(|s| s.strip_suffix(".pending"))
        .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok_and(|v| v.to_string() == s))
}
// Only this fresh create_new file may be removed. It never names a recording,
// existing sidecar, final manifest or any caller-selected path.
struct Pending(PathBuf);
impl Drop for Pending { fn drop(&mut self) { let _ = fs::remove_file(&self.0); } }
fn publish_new(from: &Path, to: &Path) -> Result<()> {
    #[cfg(windows)] {
        use std::os::windows::ffi::OsStrExt;
        use windows::{core::PCWSTR, Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH}};
        let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // Omitting REPLACE_EXISTING is essential: a prior sealed generation is immutable.
        unsafe { MoveFileExW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()), MOVEFILE_WRITE_THROUGH) }.map_err(|_| TimingReadError::Io)
    }
    #[cfg(not(windows))] {
        // Atomic no-replace publication; Pending drops our second link before
        // any admission can accept the resulting single-link final manifest.
        fs::hard_link(from, to).map_err(|_| TimingReadError::Io)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry { name: String, proof: FileProof }
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest { version: u8, files: Vec<Entry> }
pub(super) struct HistoryPins {
    root: PathBuf,
    entries: Vec<Entry>,
    files: Vec<File>,
    _manifest: File,
    _directories: Vec<File>,
}
fn names(root: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(root).map_err(|_| TimingReadError::Io)? {
        let entry = entry.map_err(|_| TimingReadError::Io)?;
        let name = entry.file_name().into_string().map_err(|_| TimingReadError::UnsafePath)?;
        if pending_name(&name) { continue; }
        if matches!(name.as_str(), "relay-owner.json" | "relay-summary.json" | "relay-timing.jsonl" | NAME) { continue; }
        let meta = fs::symlink_metadata(entry.path()).map_err(|_| TimingReadError::Io)?;
        if meta.is_dir() && matches!(name.as_str(), "chat-assets" | "replay-assets") {
            for asset in fs::read_dir(entry.path()).map_err(|_| TimingReadError::Io)? {
                let asset = asset.map_err(|_| TimingReadError::Io)?;
                let leaf = asset.file_name().into_string().map_err(|_| TimingReadError::UnsafePath)?;
                if !leaf.bytes().all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b)) {
                    return Err(TimingReadError::UnsafePath);
                }
                names.push(format!("{name}/{leaf}"));
                if names.len() > FILES { return Err(TimingReadError::Limit); }
            }
        } else if meta.is_file() && (name.starts_with("chat.") || name.starts_with("chat-") || matches!(name.as_str(), "relay-receive.jsonl" | "viewer-metrics.jsonl" | "relay-frame-reference.json")) {
            names.push(name);
        } else {
            // A newly owned relay has only text/asset files. Do not hash an
            // unexpected local file merely because it was placed in this root.
            return Err(TimingReadError::Schema);
        }
        if names.len() > FILES { return Err(TimingReadError::Limit); }
    }
    names.sort();
    Ok(names)
}
fn digest(file: &mut File, bound: u64, cancel: &AtomicBool) -> Result<FileProof> {
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        cancelled(cancel)?;
        let read = file.read(&mut buffer).map_err(|_| TimingReadError::Io)?;
        if read == 0 { break; }
        bytes = bytes.checked_add(read as u64).filter(|v| *v <= bound).ok_or(TimingReadError::Limit)?;
        hash.update(&buffer[..read]);
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| TimingReadError::Io)?;
    Ok(FileProof { bytes, sha256: hash.finalize().into() })
}
fn directories(root: &Path) -> Result<Vec<File>> {
    let mut pins = crate::streaming::browser_store::cleanup::pin_directories(root).map_err(|_| TimingReadError::UnsafePath)?;
    for sub in ["chat-assets", "replay-assets"] {
        if root.join(sub).exists() {
            pins.extend(crate::streaming::browser_store::cleanup::pin_directories(&root.join(sub)).map_err(|_| TimingReadError::UnsafePath)?);
        }
    }
    Ok(pins)
}
pub(super) fn seal(root: &Path, cancel: &AtomicBool) -> Result<FileProof> {
    let _directories = directories(root)?;
    let names = names(root)?;
    if names.is_empty() { return Err(TimingReadError::Schema); }
    let mut files = Vec::new();
    let mut entries = Vec::new();
    let mut total = 0u64;
    for name in names {
        cancelled(cancel)?;
        let path = root.join(&name);
        let bytes = fs::symlink_metadata(&path).map_err(|_| TimingReadError::Io)?.len();
        // pinned() validates regular/reparse/hardlink identity and exact size.
        let mut file = pinned(&path, &FileProof { bytes, sha256: [0; 32] }, FILE_BYTES)?;
        let proof = digest(&mut file, FILE_BYTES, cancel)?;
        total = total.checked_add(proof.bytes).filter(|v| *v <= TOTAL_BYTES).ok_or(TimingReadError::Limit)?;
        entries.push(Entry { name, proof });
        files.push(file);
    }
    let bytes = serde_json::to_vec(&Manifest { version: 1, files: entries }).map_err(|_| TimingReadError::Schema)?;
    if bytes.len() as u64 > MANIFEST_BYTES { return Err(TimingReadError::Limit); }
    let proof = FileProof { bytes: bytes.len() as u64, sha256: Sha256::digest(&bytes).into() };
    cancelled(cancel)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)] {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let temp = root.join(format!(".relay-history-{}.pending", uuid::Uuid::new_v4()));
    let mut file = options.open(&temp).map_err(|_| TimingReadError::Io)?;
    let pending = Pending(temp);
    let written = file.write_all(&bytes).and_then(|_| file.sync_all());
    drop(file);
    written.map_err(|_| TimingReadError::Io)?;
    cancelled(cancel)?;
    let published = publish_new(&pending.0, &root.join(NAME));
    drop(pending);
    match published {
        Ok(()) => {},
        Err(_) if root.join(NAME).exists() => {
            let mut file = pinned(&root.join(NAME), &proof, MANIFEST_BYTES)?;
            if digest(&mut file, MANIFEST_BYTES, cancel)? != proof { return Err(TimingReadError::Hash); }
        }
        Err(error) => return Err(error),
    }
    // All source handles remain pinned through manifest publication.
    drop(files);
    Ok(proof)
}
impl HistoryPins {
    pub(super) fn open(root: &Path, proof: &FileProof, cancel: &AtomicBool) -> Result<Self> {
        let directories = directories(root)?;
        let mut manifest = pinned(&root.join(NAME), proof, MANIFEST_BYTES)?;
        if digest(&mut manifest, MANIFEST_BYTES, cancel)? != *proof { return Err(TimingReadError::Hash); }
        let mut bytes = Vec::new();
        (&mut manifest).take(MANIFEST_BYTES + 1).read_to_end(&mut bytes).map_err(|_| TimingReadError::Io)?;
        let sealed: Manifest = serde_json::from_slice(&bytes).map_err(|_| TimingReadError::Schema)?;
        if sealed.version != 1 || sealed.files.is_empty() || sealed.files.len() > FILES
            || sealed.files.iter().map(|v| v.name.clone()).collect::<Vec<_>>() != names(root)? { return Err(TimingReadError::Identity); }
        let mut total = 0u64;
        let mut files = Vec::new();
        for entry in &sealed.files {
            cancelled(cancel)?;
            total = total.checked_add(entry.proof.bytes).filter(|v| *v <= TOTAL_BYTES).ok_or(TimingReadError::Limit)?;
            let mut file = pinned(&root.join(&entry.name), &entry.proof, FILE_BYTES)?;
            if digest(&mut file, FILE_BYTES, cancel)? != entry.proof { return Err(TimingReadError::Hash); }
            files.push(file);
        }
        Ok(Self { root: root.into(), entries: sealed.files, files, _manifest: manifest, _directories: directories })
    }
    pub(super) fn valid(&self) -> Result<()> {
        if self.entries.iter().map(|v| v.name.clone()).collect::<Vec<_>>() != names(&self.root)? { return Err(TimingReadError::Identity); }
        for (entry, file) in self.entries.iter().zip(&self.files) {
            if file.metadata().map_err(|_| TimingReadError::Io)?.len() != entry.proof.bytes { return Err(TimingReadError::Hash); }
        }
        Ok(())
    }
    pub(super) fn file(&self, name: &str) -> Result<File> {
        let index = self.entries.iter().position(|v| v.name == name).ok_or(TimingReadError::Identity)?;
        self.files[index].try_clone().map_err(|_| TimingReadError::Io)
    }
    pub(super) fn contains(&self, name: &str) -> bool { self.entries.iter().any(|v| v.name == name) }
    pub(super) fn files(&self) -> usize { self.files.len() }
    #[cfg(test)]
    pub(super) fn asset_ids(&self, provider: &str) -> Vec<String> {
        self.entries.iter().filter_map(|e| {
            let id = if provider == "soop" { e.name.strip_prefix("chat-assets/")? }
                else { e.name.strip_prefix("replay-assets/")?.strip_suffix(".json")? };
            (id.len() == 64 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).then(|| id.to_owned())
        }).collect()
    }
}
