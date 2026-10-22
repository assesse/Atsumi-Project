//! Consumes one verified candidate. This is local file installation, not capture
//! admission, library mutation, source cleanup or a reusable path authority.
use super::*;
use std::{cell::Cell, time::UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::platform_media) enum PublicationState { Sealed, Interrupted }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::platform_media) struct PublicationExclusion {
    pub bytes: u64,
    pub units: u64,
    pub startup_coverage: bool,
    pub audio_coverage_tail: bool,
    pub outside_selected_video: bool,
    pub incomplete_structure: bool,
    pub incomplete_append: bool,
}
impl PublicationExclusion {
    fn project(value: Option<selection::Excluded>) -> Result<Self> {
        let Some(value) = value else { return Ok(Self::default()); };
        let bytes = value.end.checked_sub(value.start).ok_or(Fault::Bounds)?;
        if bytes > MAX_FILE || value.complete_units > 100_000 { return Err(Fault::Bounds); }
        let reasons = value.reasons;
        Ok(Self { bytes, units: value.complete_units,
            startup_coverage: reasons.startup_coverage,
            audio_coverage_tail: reasons.audio_coverage_tail,
            outside_selected_video: reasons.outside_selected_video,
            incomplete_structure: reasons.incomplete_structure,
            incomplete_append: reasons.incomplete_append })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::platform_media) struct PublicationSourceFacts {
    pub state: PublicationState,
    pub seal_cause: Option<SealCause>,
    pub selected: bool,
    pub incomplete_append: bool,
    pub torn_journal: bool,
    pub uncommitted_bytes: [u64; 2],
    pub partial_tail: [bool; 2],
    pub leading: [PublicationExclusion; 2],
    pub trailing: [PublicationExclusion; 2],
    pub audio_excluded_packets: [u64; 2],
}
impl PublicationSourceFacts {
    fn project(scope: &SourceScope, selected: &Option<SelectedEvidence>) -> Result<Self> {
        let state = match scope.state {
            PrefixState::Sealed => PublicationState::Sealed,
            PrefixState::Interrupted => PublicationState::Interrupted,
            _ => return Err(Fault::UnsupportedAdmission),
        };
        if scope.seal_cause == Some(SealCause::Cancelled)
            || scope.uncommitted_bytes.iter().any(|n| *n > MAX_FILE) {
            return Err(Fault::UnsupportedAdmission);
        }
        let mut value = Self { state, seal_cause: scope.seal_cause, selected: selected.is_some(),
            incomplete_append: scope.incomplete_append, torn_journal: scope.torn_journal,
            uncommitted_bytes: scope.uncommitted_bytes, partial_tail: [false; 2],
            leading: [PublicationExclusion::default(); 2], trailing: [PublicationExclusion::default(); 2],
            audio_excluded_packets: [0; 2] };
        if let Some(selected) = selected {
            for (at, track) in selected.facts.tracks.iter().enumerate() {
                value.partial_tail[at] = match track.original_outcome.terminal {
                    index::Terminal::Complete => false,
                    index::Terminal::PartialTail { .. } => true,
                    index::Terminal::Failed(_) => return Err(Fault::UnsupportedAdmission),
                };
                value.leading[at] = PublicationExclusion::project(track.leading)?;
                value.trailing[at] = PublicationExclusion::project(track.trailing)?;
            }
            value.audio_excluded_packets = [selected.audio.leading(), selected.audio.trailing()]
                .map(|fold| fold.map_or(0, |fold| fold.count));
            if value.audio_excluded_packets.iter().any(|n| *n > MAX_PACKETS) { return Err(Fault::Bounds); }
        }
        Ok(value)
    }
}

/// Reporting snapshot only. Copying this cannot construct a handoff.
#[derive(Clone, Debug)]
pub(in crate::platform_media) struct PublicationFacts {
    pub identity: SpoolIdentity,
    pub bytes: u64,
    pub sha256: [u8; 32],
    pub duration: f64,
    pub packets: [u64; 2],
    pub source: PublicationSourceFacts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase { Prepared, Installed, Verified, Failed }

struct RenameFile {
    file: File,
    path: PathBuf,
    key: FileKey,
    length: u64,
    hash: [u8; 32],
    directories: Arc<Directories>,
}
impl RenameFile {
    fn acquire(held: HeldInput, cancel: &AtomicBool, deadline: Instant) -> Result<Self> {
        Self::acquire_after_witness(held, cancel, deadline, || Ok(()))
    }
    fn acquire_after_witness(held: HeldInput, cancel: &AtomicBool, deadline: Instant,
        after_release: impl FnOnce() -> Result<()>) -> Result<Self> {
        held.check(cancel, deadline)?;
        if held.kind != OwnedKind::Candidate { return Err(Fault::Identity); }
        let (key, length) = file_key(&held.file.file)?;
        let witness = open_publication(&held.path, Access::Witness)?;
        if file_key(&witness)? != (key, length) { return Err(Fault::Changed); }
        let HeldInput { path, file, directories, hash, .. } = held;
        // The witness denies writes while the old no-delete holding is released.
        // A rename in this narrow interval is rejected by exact identity below.
        drop(file);
        after_release()?;
        checkpoint(cancel, deadline)?;
        directories.check().map_err(Fault::Source)?;
        let file = open_publication(&path, Access::Owner)?;
        if file_key(&file)? != (key, length) || file_key(&witness)? != (key, length) {
            return Err(Fault::Changed);
        }
        let value = Self { file, path, key, length, hash, directories };
        value.check(cancel, deadline)?;
        drop(witness);
        Ok(value)
    }
    fn check(&self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        checkpoint(cancel, deadline)?;
        self.directories.check().map_err(Fault::Source)?;
        let inspection = open_publication(&self.path, Access::Inspect)?;
        if file_key(&self.file)? != (self.key, self.length)
            || file_key(&inspection)? != (self.key, self.length) { return Err(Fault::Changed); }
        checkpoint(cancel, deadline)
    }
    fn install(&mut self, target: &Path, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        self.check(cancel, deadline)?;
        if target.parent() != self.path.parent() || target.file_name().and_then(|s| s.to_str()) != Some("media.mp4") {
            return Err(Fault::Path);
        }
        rename_exact(&self.file, target)?;
        // Update even if the following cancellation/metadata check fails. Never
        // roll back, remove or look for another object after a successful rename.
        self.path = target.to_owned();
        self.check(cancel, deadline)
    }
    fn verify(&self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        self.check(cancel, deadline)?;
        if hash_file(&self.file, self.length, cancel, deadline)? != self.hash { return Err(Fault::Changed); }
        self.check(cancel, deadline)
    }
}

#[derive(Clone, Copy)]
enum Access { Witness, Owner, Inspect, Existing }
fn open_publication(path: &Path, access: Access) -> Result<File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let metadata = fs::symlink_metadata(path).map_err(|_| Fault::Path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() { return Err(Fault::Path); }
        let mut options = OpenOptions::new();
        // FILE_SHARE_READ=1, FILE_SHARE_DELETE=4, DELETE=0x10000.
        options.read(true).share_mode(if matches!(access, Access::Owner | Access::Existing) { 1 } else { 5 })
            .custom_flags(0x00200000);
        if matches!(access, Access::Owner) { options.access_mode(0x80000000 | 0x10000); }
        if matches!(access, Access::Inspect) { options.access_mode(0x80); }
        let file = options.open(path).map_err(|_| Fault::Path)?;
        file_key(&file)?;
        Ok(file)
    }
    #[cfg(not(windows))]
    { let _ = (path, access); Err(Fault::UnsupportedAdmission) }
}
fn rename_exact(file: &File, target: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
        use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::{FileRenameInfo, SetFileInformationByHandle, FILE_RENAME_INFO}};
        let name: Vec<u16> = target.as_os_str().encode_wide().collect();
        if name.is_empty() || name.len() > 32767 || name.contains(&0) { return Err(Fault::Path); }
        let size = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + (name.len() + 1) * 2;
        let capacity = size.max(std::mem::size_of::<FILE_RENAME_INFO>());
        let mut buffer = vec![0usize; capacity.div_ceil(std::mem::size_of::<usize>())];
        unsafe {
            let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
            (*info).Anonymous.ReplaceIfExists = false;
            (*info).RootDirectory = HANDLE::default();
            (*info).FileNameLength = (name.len() * 2) as u32;
            let to = (*info).FileName.as_mut_ptr();
            std::ptr::copy_nonoverlapping(name.as_ptr(), to, name.len());
            to.add(name.len()).write(0);
            SetFileInformationByHandle(HANDLE(file.as_raw_handle()), FileRenameInfo, buffer.as_ptr().cast(), size as u32)
                .map_err(|_| Fault::Io)?;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    { let _ = (file, target); Err(Fault::UnsupportedAdmission) }
}

pub(in crate::platform_media) struct PublicationHandoff {
    source: FinalizerSource,
    selected: Option<SelectedEvidence>,
    file: RenameFile,
    job: PathBuf,
    facts: PublicationFacts,
    phase: Cell<Phase>,
    _attempt: Arc<Directories>,
}
impl VerifiedCandidate {
    pub(in crate::platform_media) fn into_publication(self, root: &Path, expected: &SpoolIdentity,
        cancel: &AtomicBool, deadline: Instant) -> Result<PublicationHandoff> {
        checkpoint(cancel, deadline)?;
        expected.validate().map_err(Fault::Source)?;
        self.check_source(cancel)?;
        if &self.scope.identity != expected { return Err(Fault::Identity); }
        let job = crate::platform_media::storage::job_dir(root, "youtube", &expected.job_id).map_err(|_| Fault::Path)?;
        if self.candidate.path.parent() != Some(job.as_path())
            || !self.candidate.path.file_name().and_then(|n| n.to_str()).is_some_and(valid_merge_name) {
            return Err(Fault::Identity);
        }
        let source = PublicationSourceFacts::project(&self.scope, &self.selected)?;
        let rational = candidate_duration([self.duration; 2])?;
        let duration = rational.numerator as f64 / rational.denominator as f64;
        let packets = self.comparisons.each_ref().map(|v| v.count);
        if !duration.is_finite() || duration <= 0.0 || duration > MAX_TIME.as_secs_f64()
            || packets.iter().any(|n| *n == 0 || *n > MAX_PACKETS) { return Err(Fault::Bounds); }
        self.candidate.rehash(cancel, deadline)?;
        let facts = PublicationFacts { identity: expected.clone(), bytes: self.candidate.length,
            sha256: self.candidate.hash, duration, packets, source };
        let Self { source, selected, candidate, _attempt, .. } = self;
        let file = RenameFile::acquire(candidate, cancel, deadline)?;
        let value = PublicationHandoff { source, selected, file, job, facts,
            phase: Cell::new(Phase::Prepared), _attempt };
        value.quick(cancel, deadline)?;
        Ok(value)
    }
}
fn valid_merge_name(name: &str) -> bool {
    name.strip_prefix("merge-").and_then(|n| n.strip_suffix(".mp4"))
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok_and(|parsed| parsed.to_string() == id))
}
impl PublicationHandoff {
    pub(in crate::platform_media) fn facts(&self) -> &PublicationFacts { &self.facts }
    pub(in crate::platform_media) fn matches_scope(&self, root: &Path, expected: &SpoolIdentity) -> bool {
        &self.facts.identity == expected
            && crate::platform_media::storage::job_dir(root, "youtube", &expected.job_id).is_ok_and(|job| job == self.job)
    }
    fn quick(&self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        checkpoint(cancel, deadline)?;
        check_source_evidence(&self.source, &self.selected, cancel)?;
        self.file.check(cancel, deadline)
    }
    fn terminal<T>(&self, result: Result<T>) -> Result<T> {
        if result.is_err() { self.phase.set(Phase::Failed); }
        result
    }
    /// Explicit raw recovery only. Absence is the sole nonterminal fallback;
    /// an existing target still requires verify_installed's complete hash.
    pub(in crate::platform_media) fn adopt_existing_if_present(&mut self, cancel: &AtomicBool, deadline: Instant) -> Result<bool> {
        let result = (|| {
            if self.phase.replace(Phase::Failed) != Phase::Prepared { return Err(Fault::Terminal); }
            self.quick(cancel, deadline)?;
            let path = self.job.join("media.mp4");
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.quick(cancel, deadline)?;
                    self.phase.set(Phase::Prepared);
                    return Ok(false);
                }
                Err(_) => return Err(Fault::Path),
                Ok(_) => (),
            }
            // READ/share-READ holds the fixed existing object without write or
            // delete permission. The freshly verified merge remains on disk.
            let file = open_publication(&path, Access::Existing)?;
            let (key, length) = file_key(&file)?;
            if length != self.facts.bytes { return Err(Fault::Changed); }
            let existing = RenameFile { file, path, key, length,
                hash: self.facts.sha256, directories: self.file.directories.clone() };
            existing.check(cancel, deadline)?;
            self.quick(cancel, deadline)?;
            self.file = existing;
            self.quick(cancel, deadline)?;
            self.phase.set(Phase::Installed);
            Ok(true)
        })();
        self.terminal(result)
    }
    pub(in crate::platform_media) fn install_no_replace(&mut self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        let result = (|| {
            if self.phase.replace(Phase::Failed) != Phase::Prepared { return Err(Fault::Terminal); }
            self.quick(cancel, deadline)?;
            self.file.install(&self.job.join("media.mp4"), cancel, deadline)?;
            self.quick(cancel, deadline)?;
            self.phase.set(Phase::Installed);
            Ok(())
        })();
        self.terminal(result)
    }
    pub(in crate::platform_media) fn verify_installed(&mut self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        let result = (|| {
            if self.phase.replace(Phase::Failed) != Phase::Installed { return Err(Fault::Terminal); }
            self.quick(cancel, deadline)?;
            self.file.verify(cancel, deadline)?;
            self.quick(cancel, deadline)?;
            self.phase.set(Phase::Verified);
            Ok(())
        })();
        self.terminal(result)
    }
    pub(in crate::platform_media) fn check_for_commit(&self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        let result = if self.phase.get() == Phase::Verified { self.quick(cancel, deadline) } else { Err(Fault::Terminal) };
        self.terminal(result)
    }
    pub(in crate::platform_media) fn with_final_path<T>(&self, cancel: &AtomicBool, deadline: Instant,
        work: impl FnOnce(&Path, &PublicationFacts) -> T) -> Result<T> {
        self.check_for_commit(cancel, deadline)?;
        let value = work(&self.file.path, &self.facts);
        self.check_for_commit(cancel, deadline)?;
        Ok(value)
    }
    pub(in crate::platform_media) fn modified_millis(&self, cancel: &AtomicBool, deadline: Instant) -> Result<u64> {
        self.check_for_commit(cancel, deadline)?;
        let value = self.file.file.metadata().map_err(|_| Fault::Io)?.modified().map_err(|_| Fault::Io)?
            .duration_since(UNIX_EPOCH).map_err(|_| Fault::Bounds)?.as_millis();
        self.check_for_commit(cancel, deadline)?;
        u64::try_from(value).map_err(|_| Fault::Bounds)
    }
}

#[cfg(test)]
mod tests;
