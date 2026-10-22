//! Immutable local text-storage proof. Never equate video validation with chat completeness.
use super::{chat_parts, storage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

pub(super) const ERROR: &str =
    "채팅 보존 상태를 저장·확인하지 못했습니다. 완성 영상과 원본은 보존합니다.";
const LIMIT: u64 = 16 * 1024;
const MARKER: &str = "chat-status.json";
const FINAL: &str = "chat-status.final.json";
static DIAGNOSTICS: AtomicUsize = AtomicUsize::new(0);
fn admit_diagnostic(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
            (count < 128).then_some(count + 1)
        })
        .is_ok()
}
fn diagnostic_code(domain: &'static str, code: Option<i32>) -> String {
    match (domain, code) {
        ("hresult", Some(code)) => format!("HRESULT_{:08X}", code as u32),
        ("win32", Some(code)) if code >= 0 => format!("WIN32_{code}"),
        ("os", Some(code)) if code >= 0 => format!("OS_{code}"),
        ("validation", _) => "VALIDATION".into(),
        (_, Some(code)) => format!("OS_HEX_{:08X}", code as u32),
        _ => "OS_UNAVAILABLE".into(),
    }
}
fn failure(stage: &'static str, domain: &'static str, code: Option<i32>) -> String {
    if admit_diagnostic(&DIAGNOSTICS) {
        let error_code = diagnostic_code(domain, code);
        tracing::warn!(
            diag_stage = stage,
            diag_operation = "chat_status",
            error_code = error_code.as_str(),
            "platform chat status operation failed"
        );
        // lib tests have no application tracing subscriber. Only these static,
        // numeric fields reach captured stderr; never print the original error.
        #[cfg(test)]
        eprintln!("chat_status stage={stage} domain={domain} code={code:?}");
    }
    ERROR.into()
}
fn io_failure(stage: &'static str, error: std::io::Error) -> String {
    failure(stage, "os", error.raw_os_error())
}
#[cfg(windows)]
fn windows_code(code: i32) -> (&'static str, i32) {
    if code as u32 & 0xffff_0000 == 0x8007_0000 {
        ("win32", (code as u32 & 0xffff) as i32)
    } else {
        ("hresult", code)
    }
}
#[cfg(windows)]
fn windows_failure(stage: &'static str, error: windows::core::Error) -> String {
    let (domain, code) = windows_code(error.code().0);
    failure(stage, domain, Some(code))
}
#[cfg(windows)]
fn rename_buffer(path: &[u16]) -> (Vec<usize>, u32) {
    use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::FILE_RENAME_INFO};
    // FileNameLength excludes NUL, but FileName itself is a NUL-terminated path.
    // Do not rely on Vec<usize> alignment padding to provide that final WCHAR.
    let size = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + (path.len() + 1) * 2;
    let allocation = size.max(std::mem::size_of::<FILE_RENAME_INFO>());
    let mut buffer = vec![0usize; allocation.div_ceil(std::mem::size_of::<usize>())];
    unsafe {
        let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).RootDirectory = HANDLE::default();
        (*info).FileNameLength = (path.len() * 2) as u32;
        let name = (*info).FileName.as_mut_ptr();
        std::ptr::copy_nonoverlapping(path.as_ptr(), name, path.len());
        name.add(path.len()).write(0);
    }
    (buffer, size as u32)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    ArchiveLimit,
    WriteFailed,
    FlushFailed,
    Interrupted,
    StatusUnknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TextState {
    Complete,
    Partial,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatArchiveSummary {
    pub state: TextState,
    pub reasons: Vec<Reason>,
    pub originals_retained: bool,
    pub assets_incomplete: bool,
}
impl ChatArchiveSummary {
    pub(super) fn unknown() -> Self {
        Self {
            state: TextState::Unknown,
            reasons: vec![Reason::StatusUnknown],
            originals_retained: true,
            assets_incomplete: false,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Marker {
    pub(super) version: u8,
    pub(super) provider: String,
    pub(super) job_id: String,
    pub(super) attachment_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileProof {
    name: String,
    bytes: u64,
    sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Proof {
    marker: Marker,
    summary: ChatArchiveSummary,
    // Only complete archives need a hash proof for cleanup eligibility.
    files: Vec<FileProof>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MultipartProof {
    marker: Marker,
    summary: ChatArchiveSummary,
    catalog: chat_parts::CatalogProof,
}
pub(super) fn pin_parents(path: &Path) -> Result<Vec<File>, String> {
    storage::safe_path(path).map_err(|_| failure("ancestor_path", "validation", None))?;
    let mut guards = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        for parent in path
            .ancestors()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            let file = OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x02200000)
                .open(parent)
                .map_err(|e| io_failure("ancestor_open", e))?;
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            unsafe {
                GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                    .map_err(|e| windows_failure("ancestor_info", e))?;
            }
            if info.dwFileAttributes & 0x400 != 0 || info.dwFileAttributes & 0x10 == 0 {
                return Err(failure("ancestor_type", "validation", None));
            }
            guards.push(file);
        }
    }
    Ok(guards)
}
pub(super) fn checked_handle(file: &File, max: u64) -> Result<u64, String> {
    let meta = file
        .metadata()
        .map_err(|e| io_failure("handle_metadata", e))?;
    if !meta.is_file() || meta.len() > max {
        return Err(failure("handle_size_type", "validation", None));
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                .map_err(|e| windows_failure("handle_info", e))?;
        }
        if info.nNumberOfLinks != 1 || info.dwFileAttributes & (0x400 | 0x10) != 0 {
            return Err(failure("handle_links_attributes", "validation", None));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(failure("handle_links", "validation", None));
        }
    }
    Ok(meta.len())
}
pub(super) struct OwnedReader {
    pub(super) file: File,
    pub(super) _parents: Vec<File>,
}
pub(super) fn open_reader(path: &Path, max: u64) -> Result<Option<OwnedReader>, String> {
    let parents = pin_parents(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = match options.open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_failure("reader_open", e)),
    };
    checked_handle(&file, max)?;
    Ok(Some(OwnedReader {
        file,
        _parents: parents,
    }))
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let Some(mut reader) = open_reader(path, LIMIT)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    (&mut reader.file)
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() as u64 > LIMIT || checked_handle(&reader.file, LIMIT)? != bytes.len() as u64 {
        return Err(failure("reader_length", "validation", None));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| ERROR.into())
}
pub(super) fn identity(marker: &Marker, provider: &str, id: &str) -> Result<(), String> {
    if !matches!(marker.version, 1 | 2)
        || marker.provider != provider
        || marker.job_id != id
        || !matches!(provider, "youtube" | "soop")
        || uuid::Uuid::parse_str(id).is_err()
        || uuid::Uuid::parse_str(&marker.attachment_id).is_err()
    {
        return Err(failure("identity", "validation", None));
    }
    Ok(())
}
fn publish<T: Serialize>(directory: &Path, name: &str, value: &T) -> Result<(), String> {
    let target = directory.join(name);
    let _parents = pin_parents(&target)?;
    if fs::symlink_metadata(directory.join(name)).is_ok() {
        return Err(failure("publish_target_exists", "validation", None));
    }
    let bytes =
        serde_json::to_vec(value).map_err(|_| failure("publish_serialize", "validation", None))?;
    if bytes.len() as u64 > LIMIT {
        return Err(failure("publish_size", "validation", None));
    }
    let temp = directory.join(format!("chat-status-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(0xC0010000)
            .share_mode(1)
            .custom_flags(0x00200000);
    }
    let mut file = options
        .open(&temp)
        .map_err(|e| io_failure("publish_temp_open", e))?;
    checked_handle(&file, LIMIT)?;
    file.write_all(&bytes)
        .map_err(|e| io_failure("publish_write", e))?;
    file.sync_all()
        .map_err(|e| io_failure("publish_sync_before_rename", e))?;
    checked_handle(&file, LIMIT)?;
    #[cfg(windows)]
    {
        // Rename the exact held file, without a source-path reopen or replacement.
        use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{FileRenameInfo, SetFileInformationByHandle},
        };
        let path: Vec<u16> = target.as_os_str().encode_wide().collect();
        if path.len() > 32767 {
            return Err(failure("publish_name_length", "validation", None));
        }
        let (buffer, size) = rename_buffer(&path);
        unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileRenameInfo,
                buffer.as_ptr().cast(),
                size,
            )
            .map_err(|e| windows_failure("publish_rename", e))?;
        }
    }
    #[cfg(not(windows))]
    {
        // No overwrite; Windows handle-pinning is the validated application path.
        fs::hard_link(&temp, &target).map_err(|e| io_failure("publish_link", e))?;
        fs::remove_file(&temp).map_err(|e| io_failure("publish_unlink", e))?;
    }
    file.sync_all()
        .map_err(|e| io_failure("publish_sync_after_rename", e))?;
    checked_handle(&file, LIMIT)?;
    Ok(())
}
pub(super) fn begin(directory: &Path, provider: &str, id: &str) -> Result<Marker, String> {
    begin_version(directory, provider, id, 1)
}
pub(super) fn begin_multipart(
    directory: &Path,
    provider: &str,
    id: &str,
) -> Result<Marker, String> {
    begin_version(directory, provider, id, 2)
}
pub(super) fn read_marker(directory: &Path) -> Result<Option<Marker>, String> {
    read(&directory.join(MARKER))
}
fn begin_version(
    directory: &Path,
    provider: &str,
    id: &str,
    version: u8,
) -> Result<Marker, String> {
    let marker = Marker {
        version,
        provider: provider.into(),
        job_id: id.into(),
        attachment_id: uuid::Uuid::new_v4().to_string(),
    };
    identity(&marker, provider, id)?;
    if fs::symlink_metadata(directory.join(FINAL)).is_ok() {
        return Err(failure("begin_final_exists", "validation", None));
    }
    publish(directory, MARKER, &marker)?;
    Ok(marker)
}
fn file_proofs(directory: &Path) -> Result<Vec<FileProof>, String> {
    let mut proofs = Vec::new();
    let mut total = 0u64;
    for name in ["chat.events.jsonl", "chat.source.jsonl"] {
        let path = directory.join(name);
        let reader = open_reader(&path, 128 * 1024 * 1024)?;
        let Some(mut reader) = reader else {
            if name == "chat.source.jsonl" {
                continue;
            }
            return Err(ERROR.into());
        };
        let length = checked_handle(&reader.file, 128 * 1024 * 1024)?;
        total = total.saturating_add(length);
        if total > 128 * 1024 * 1024 {
            return Err(ERROR.into());
        }
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let n = reader.file.read(&mut buffer).map_err(|_| ERROR)?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            if bytes > length {
                return Err(ERROR.into());
            }
            hash.update(&buffer[..n]);
        }
        if bytes != length || checked_handle(&reader.file, 128 * 1024 * 1024)? != length {
            return Err(ERROR.into());
        }
        proofs.push(FileProof {
            name: name.into(),
            bytes: length,
            sha256: format!("{:x}", hash.finalize()),
        });
    }
    Ok(proofs)
}
pub(super) fn finish(
    directory: &Path,
    marker: &Marker,
    reason: Option<Reason>,
    assets_incomplete: bool,
) -> Result<ChatArchiveSummary, String> {
    if marker.version != 1 {
        return Err(ERROR.into());
    }
    let existing: Marker = read(&directory.join(MARKER))?.ok_or(ERROR)?;
    if &existing != marker {
        return Err(ERROR.into());
    }
    identity(marker, &marker.provider, &marker.job_id)?;
    let partial = reason.is_some();
    let summary = ChatArchiveSummary {
        state: if partial {
            TextState::Partial
        } else {
            TextState::Complete
        },
        reasons: reason.into_iter().collect(),
        originals_retained: partial,
        assets_incomplete,
    };
    let files = if summary.state == TextState::Complete {
        file_proofs(directory)?
    } else {
        Vec::new()
    };
    publish(
        directory,
        FINAL,
        &Proof {
            marker: marker.clone(),
            summary: summary.clone(),
            files,
        },
    )?;
    Ok(summary)
}
fn validate_multipart_summary(marker: &Marker, proof: &MultipartProof) -> Result<(), String> {
    identity(marker, &marker.provider, &marker.job_id)?;
    if marker.version != 2
        || &proof.marker != marker
        || proof.summary.reasons.len() > 5
        || (proof.summary.state == TextState::Complete) != proof.summary.reasons.is_empty()
        || proof.summary.originals_retained != (proof.summary.state != TextState::Complete)
        || proof.catalog.bytes > chat_parts::CATALOG_BYTES
        || proof.catalog.parts == 0
        || proof.catalog.parts > chat_parts::MAX_PARTS
        || proof.catalog.total_bytes > chat_parts::MAX_TOTAL_BYTES
        || proof.catalog.sha256.len() != 64
        || !proof
            .catalog
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ERROR.into());
    }
    Ok(())
}
fn read_multipart_proof(
    directory: &Path,
    marker: &Marker,
    catalog: &chat_parts::CatalogProof,
) -> Result<Option<MultipartProof>, String> {
    let proof: Option<MultipartProof> = read(&directory.join(FINAL))?;
    if let Some(proof) = &proof {
        validate_multipart_summary(marker, proof)?;
        if &proof.catalog != catalog {
            return Err(ERROR.into());
        }
    }
    Ok(proof)
}
pub(super) fn multipart_replay_state(
    directory: &Path,
    marker: &Marker,
    catalog: &chat_parts::CatalogProof,
) -> Result<(bool, String), String> {
    match read_multipart_proof(directory, marker, catalog)? {
        Some(proof) => Ok((
            proof.summary.state == TextState::Complete,
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&proof).map_err(|_| ERROR)?)
            ),
        )),
        None => Ok((false, "writing".into())),
    }
}
fn publish_multipart(
    directory: &Path,
    marker: &Marker,
    catalog: &chat_parts::CatalogProof,
    reason: Option<Reason>,
    assets_incomplete: bool,
) -> Result<ChatArchiveSummary, String> {
    let partial = reason.is_some();
    let summary = ChatArchiveSummary {
        state: if partial {
            TextState::Partial
        } else {
            TextState::Complete
        },
        reasons: reason.into_iter().collect(),
        originals_retained: partial,
        assets_incomplete,
    };
    let proof = MultipartProof {
        marker: marker.clone(),
        summary: summary.clone(),
        catalog: catalog.clone(),
    };
    validate_multipart_summary(marker, &proof)?;
    publish(directory, FINAL, &proof)?;
    Ok(summary)
}
pub(super) fn finish_multipart(
    directory: &Path,
    marker: &Marker,
    closed: chat_parts::ClosedParts,
    reason: Option<Reason>,
    assets_incomplete: bool,
    cancel: &AtomicBool,
) -> Result<ChatArchiveSummary, String> {
    if marker.version != 2
        || !closed.matches(directory, marker)
        || read_marker(directory)?.as_ref() != Some(marker)
    {
        return Err(ERROR.into());
    }
    identity(marker, &marker.provider, &marker.job_id)?;
    let catalog = chat_parts::load_catalog(directory)?;
    let mut reason = reason.or(closed.reason());
    if !catalog.closed {
        reason.get_or_insert(Reason::Interrupted);
    }
    if cancel.load(Ordering::Acquire) {
        reason.get_or_insert(Reason::Interrupted);
    }
    if reason.is_none() {
        if let Err(error) = chat_parts::verify_catalog(directory, &catalog, cancel) {
            if error == "cancelled" {
                reason = Some(Reason::Interrupted);
            } else {
                return Err(error);
            }
        }
    }
    // A cancelled verifier still publishes a durable partial warning. It must
    // never permit originals cleanup merely because the video later verifies.
    if cancel.load(Ordering::Acquire) {
        reason.get_or_insert(Reason::Interrupted);
    }
    publish_multipart(directory, marker, &catalog.proof, reason, assets_incomplete)
}
pub(super) fn recovery(
    directory: &Path,
    provider: &str,
    id: &str,
) -> Result<ChatArchiveSummary, String> {
    recovery_with_cancel(directory, provider, id, &AtomicBool::new(false))
}
pub(super) fn recovery_with_cancel(
    directory: &Path,
    provider: &str,
    id: &str,
    cancel: &AtomicBool,
) -> Result<ChatArchiveSummary, String> {
    let marker: Option<Marker> = read(&directory.join(MARKER))?;
    if let Some(marker) = marker.as_ref().filter(|marker| marker.version == 2) {
        identity(marker, provider, id)?;
        let catalog = chat_parts::load_catalog(directory)?;
        if let Some(proof) = read_multipart_proof(directory, marker, &catalog.proof)? {
            if proof.summary.state == TextState::Complete {
                if !catalog.closed {
                    return Err(ERROR.into());
                }
                chat_parts::verify_catalog(directory, &catalog, cancel)?;
            }
            return Ok(proof.summary);
        }
        return publish_multipart(
            directory,
            marker,
            &catalog.proof,
            Some(Reason::Interrupted),
            false,
        );
    }
    let final_proof: Option<Proof> = read(&directory.join(FINAL))?;
    if let Some(proof) = final_proof {
        identity(&proof.marker, provider, id)?;
        if marker.as_ref() != Some(&proof.marker)
            || proof.summary.reasons.len() > 5
            || (proof.summary.state == TextState::Complete) != proof.summary.reasons.is_empty()
            || proof.summary.originals_retained != (proof.summary.state != TextState::Complete)
            || proof.files.len() > 2
        {
            return Err(ERROR.into());
        }
        if proof.summary.state == TextState::Complete {
            let current = file_proofs(directory)?;
            if current.len() != proof.files.len()
                || !current
                    .iter()
                    .zip(&proof.files)
                    .all(|(a, b)| a.name == b.name && a.bytes == b.bytes && a.sha256 == b.sha256)
            {
                return Err(ERROR.into());
            }
        } else if !proof.files.is_empty() {
            return Err(ERROR.into());
        }
        return Ok(proof.summary);
    }
    if let Some(marker) = marker {
        identity(&marker, provider, id)?;
        finish(directory, &marker, Some(Reason::Interrupted), false)
    } else {
        let marker = begin(directory, provider, id)?;
        let summary = ChatArchiveSummary {
            state: TextState::Unknown,
            reasons: vec![Reason::StatusUnknown],
            originals_retained: true,
            assets_incomplete: false,
        };
        publish(
            directory,
            FINAL,
            &Proof {
                marker,
                summary: summary.clone(),
                files: Vec::new(),
            },
        )?;
        Ok(summary)
    }
}
// Metadata-only read for completed library browsing; never repair/migrate legacy data.
pub(super) fn inspect(
    directory: &Path,
    provider: &str,
    id: &str,
) -> Result<Option<ChatArchiveSummary>, String> {
    let marker: Option<Marker> = read(&directory.join(MARKER))?;
    if let Some(marker) = marker.as_ref().filter(|marker| marker.version == 2) {
        identity(marker, provider, id)?;
        // Browsing reads bounded metadata only, never hashes media/chat shards.
        let proof: MultipartProof = read(&directory.join(FINAL))?.ok_or(ERROR)?;
        validate_multipart_summary(marker, &proof)?;
        return Ok(Some(proof.summary));
    }
    let proof: Option<Proof> = read(&directory.join(FINAL))?;
    match (marker, proof) {
        (None, None) => Ok(None),
        (Some(marker), Some(proof)) => {
            identity(&marker, provider, id)?;
            if marker != proof.marker
                || proof.summary.reasons.len() > 5
                || (proof.summary.state == TextState::Complete) != proof.summary.reasons.is_empty()
                || proof.summary.originals_retained != (proof.summary.state != TextState::Complete)
                || proof.files.len() > 2
            {
                return Err(ERROR.into());
            }
            Ok(Some(proof.summary))
        }
        _ => Err(ERROR.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn rename_path_terminator_is_allocated_for_all_alignment_residues_and_unicode() {
        use windows::Win32::Storage::FileSystem::FILE_RENAME_INFO;
        let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
        let mut residues = std::collections::HashSet::new();
        for suffix in ["a", "aa", "aaa", "한글ab"] {
            let path: Vec<u16> = format!("C:\\fixture\\{suffix}\\chat-status.json")
                .encode_utf16()
                .collect();
            let (buffer, size) = rename_buffer(&path);
            let old_size = offset + path.len() * 2;
            residues.insert(old_size % std::mem::size_of::<usize>());
            // These checks fail the old size calculation before any possible
            // access beyond its unterminated allocation.
            assert_eq!(size as usize, old_size + 2);
            assert!(buffer.len() * std::mem::size_of::<usize>() >= size as usize);
            unsafe {
                let info = &*buffer.as_ptr().cast::<FILE_RENAME_INFO>();
                assert_eq!(info.FileNameLength as usize, path.len() * 2);
                assert!(!info.Anonymous.ReplaceIfExists);
                assert!(info.RootDirectory.is_invalid());
                let name = std::slice::from_raw_parts(info.FileName.as_ptr(), path.len() + 1);
                assert_eq!(&name[..path.len()], path.as_slice());
                assert_eq!(name[path.len()], 0);
            }
        }
        assert_eq!(residues.len(), 4);
    }
    #[cfg(windows)]
    #[test]
    fn marker_and_terminal_publish_support_four_path_lengths_including_unicode() {
        let root = tempfile::tempdir().unwrap();
        for name in ["a", "aa", "aaa", "한글ab"] {
            let directory = root.path().join(name);
            fs::create_dir(&directory).unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let marker = begin(&directory, "youtube", &id).unwrap();
            fs::write(directory.join("chat.events.jsonl"), b"fixture\n").unwrap();
            let summary = finish(&directory, &marker, None, false).unwrap();
            assert_eq!(summary.state, TextState::Complete);
            assert_eq!(recovery(&directory, "youtube", &id).unwrap(), summary);
        }
    }
    #[test]
    fn diagnostics_are_capped_and_do_not_change_the_public_error() {
        let counter = AtomicUsize::new(0);
        for _ in 0..128 {
            assert!(admit_diagnostic(&counter));
        }
        assert!(!admit_diagnostic(&counter));
        assert_eq!(counter.load(Ordering::Relaxed), 128);
        for (domain, code, expected) in [
            ("win32", Some(32), "WIN32_32"),
            ("os", Some(5), "OS_5"),
            ("hresult", Some(0x80004005u32 as i32), "HRESULT_80004005"),
            ("os", None, "OS_UNAVAILABLE"),
            ("validation", None, "VALIDATION"),
        ] {
            let formatted = diagnostic_code(domain, code);
            assert_eq!(formatted, expected);
            // diagnostics::SafeFields::record_str accepts precisely this shape.
            assert!(formatted.len() <= 64);
            assert!(formatted
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'));
        }
        assert_eq!(
            io_failure("fixture_io", std::io::Error::other("private-path-secret")),
            ERROR
        );
        #[cfg(windows)]
        {
            assert_eq!(windows_code(0x80070020u32 as i32), ("win32", 32));
            assert_eq!(
                windows_code(0x80004005u32 as i32),
                ("hresult", 0x80004005u32 as i32)
            );
        }
    }
    #[test]
    fn complete_and_partial_recovery_keep_distinct_cleanup_proofs() {
        for partial in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let marker = begin(root.path(), "soop", &id).unwrap();
            fs::write(root.path().join("chat.events.jsonl"), b"fixture\n").unwrap();
            let summary = finish(
                root.path(),
                &marker,
                partial.then_some(Reason::ArchiveLimit),
                true,
            )
            .unwrap();
            assert_eq!(summary.originals_retained, partial);
            assert!(summary.assets_incomplete);
            assert_eq!(recovery(root.path(), "soop", &id).unwrap(), summary);
            fs::write(root.path().join("chat.events.jsonl"), b"changed\n").unwrap();
            assert_eq!(recovery(root.path(), "soop", &id).is_err(), !partial);
        }
    }
    #[test]
    fn interrupted_and_legacy_never_become_complete_on_retry() {
        for legacy in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            if !legacy {
                begin(root.path(), "youtube", &id).unwrap();
            }
            let first = recovery(root.path(), "youtube", &id).unwrap();
            assert!(first.originals_retained);
            assert_ne!(first.state, TextState::Complete);
            assert_eq!(recovery(root.path(), "youtube", &id).unwrap(), first);
        }
    }
    #[test]
    fn malformed_collision_and_failed_publication_preserve_marker() {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let marker = begin(root.path(), "soop", &id).unwrap();
        let before = fs::read(root.path().join(MARKER)).unwrap();
        fs::write(root.path().join(FINAL), b"unknown").unwrap();
        assert!(finish(root.path(), &marker, Some(Reason::WriteFailed), false).is_err());
        assert!(recovery(root.path(), "soop", &id).is_err());
        assert_eq!(fs::read(root.path().join(MARKER)).unwrap(), before);
        assert_eq!(fs::read(root.path().join(FINAL)).unwrap(), b"unknown");
        assert!(recovery(root.path(), "youtube", &id).is_err());
    }
    #[test]
    fn oversized_and_linked_status_never_authorize_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        begin(root.path(), "soop", &id).unwrap();
        fs::write(root.path().join(FINAL), vec![b' '; LIMIT as usize + 1]).unwrap();
        assert!(recovery(root.path(), "soop", &id).is_err());
        #[cfg(windows)]
        {
            fs::hard_link(root.path().join(MARKER), root.path().join("linked")).unwrap();
            assert!(recovery(root.path(), "soop", &id).is_err());
        }
    }
    #[test]
    fn legacy_browsing_is_read_only_and_malformed_status_remains_error() {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        assert!(inspect(root.path(), "soop", &id).unwrap().is_none());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        fs::write(root.path().join(MARKER), b"broken").unwrap();
        assert!(inspect(root.path(), "soop", &id).is_err());
        assert_eq!(fs::read(root.path().join(MARKER)).unwrap(), b"broken");
    }
    #[cfg(windows)]
    #[test]
    fn pinned_reader_rejects_concurrent_file_and_parent_replacement() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("owned");
        fs::create_dir(&directory).unwrap();
        let path = directory.join("status.json");
        fs::write(&path, b"original").unwrap();
        let mut reader = open_reader(&path, LIMIT).unwrap().unwrap();
        let target = path.clone();
        let parent = directory.clone();
        let renamed = root.path().join("other");
        let rejected = std::thread::spawn(move || {
            (
                fs::write(&target, b"changed").is_err(),
                fs::rename(&target, parent.join("moved.json")).is_err(),
                fs::rename(&parent, &renamed).is_err(),
            )
        })
        .join()
        .unwrap();
        assert_eq!(rejected, (true, true, true));
        let mut bytes = Vec::new();
        reader.file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"original");
    }
}
