//! Versioned, exact-owned chat shards. No network, caller-supplied filenames or cleanup.
use super::{
    chat_status::{self, Marker, Reason},
    storage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub(super) const PART_BYTES: u64 = 8 * 1024 * 1024;
pub(super) const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub(super) const MAX_PARTS: usize = 512;
pub(super) const CATALOG_BYTES: u64 = 256 * 1024;
const LINE_BYTES: usize = 512 * 1024;
const RESERVE: u64 = 1024 * 1024 * 1024;
const JOURNAL: &str = "chat-parts.jsonl";
const ERROR: &str = chat_status::ERROR;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Stream {
    Events,
    Source,
}
impl Stream {
    fn slot(self) -> usize {
        match self {
            Self::Events => 0,
            Self::Source => 1,
        }
    }
}
pub(super) fn part_name(stream: Stream, sequence: u32) -> Result<String, String> {
    if sequence as usize >= MAX_PARTS {
        return Err(ERROR.into());
    }
    let stream = match stream {
        Stream::Events => "events",
        Stream::Source => "source",
    };
    Ok(if sequence == 0 {
        format!("chat.{stream}.jsonl")
    } else {
        format!("chat.{stream}.{sequence:06}.jsonl")
    })
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
enum Record {
    Header {
        marker: Marker,
    },
    Open {
        stream: Stream,
        sequence: u32,
    },
    Seal {
        stream: Stream,
        sequence: u32,
        bytes: u64,
        lines: u64,
        sha256: String,
    },
}
#[derive(Clone, Copy)]
struct Bounds {
    part: u64,
    total: u64,
    parts: usize,
    catalog: u64,
}
const BOUNDS: Bounds = Bounds {
    part: PART_BYTES,
    total: MAX_TOTAL_BYTES,
    parts: MAX_PARTS,
    catalog: CATALOG_BYTES,
};
struct ActivePart {
    file: File,
    sequence: u32,
    bytes: u64,
    lines: u64,
    hash: Sha256,
    failed: bool,
}
pub(super) struct Writer {
    directory: PathBuf,
    marker: Marker,
    journal: File,
    journal_bytes: u64,
    active: [Option<ActivePart>; 2],
    next: [u32; 2],
    count: usize,
    total: u64,
    reason: Option<Reason>,
    last_sync: Instant,
    space_budget: u64,
    last_space_check: Instant,
    bounds: Bounds,
    _parents: Vec<File>,
}
pub(super) struct ClosedParts {
    directory: PathBuf,
    marker: Marker,
    pub reason: Option<Reason>,
}
impl ClosedParts {
    pub(super) fn reason(&self) -> Option<Reason> {
        self.reason.clone()
    }
    pub(super) fn matches(&self, directory: &Path, marker: &Marker) -> bool {
        self.directory == directory && &self.marker == marker
    }
}
fn create(path: &Path) -> Result<File, String> {
    storage::safe_path(path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(path).map_err(|_| ERROR)?;
    chat_status::checked_handle(&file, CATALOG_BYTES.max(PART_BYTES))?;
    Ok(file)
}
impl Writer {
    pub(super) fn create(directory: &Path, marker: &Marker) -> Result<Self, String> {
        Self::with_bounds(directory, marker, BOUNDS)
    }
    fn with_bounds(directory: &Path, marker: &Marker, bounds: Bounds) -> Result<Self, String> {
        if marker.version != 2 || chat_status::read_marker(directory)?.as_ref() != Some(marker) {
            return Err(ERROR.into());
        }
        let parents = chat_status::pin_parents(&directory.join(JOURNAL))?;
        let journal = create(&directory.join(JOURNAL))?;
        let mut writer = Self {
            directory: directory.into(),
            marker: marker.clone(),
            journal,
            journal_bytes: 0,
            active: [None, None],
            next: [0, 0],
            count: 0,
            total: 0,
            reason: None,
            last_sync: Instant::now(),
            space_budget: 0,
            last_space_check: Instant::now(),
            bounds,
            _parents: parents,
        };
        writer
            .record(&Record::Header {
                marker: marker.clone(),
            })
            .map_err(|_| ERROR)?;
        writer.open_part(Stream::Events).map_err(|_| ERROR)?;
        Ok(writer)
    }
    fn latch(&mut self, reason: Reason) -> Reason {
        self.reason.get_or_insert(reason).clone()
    }
    fn record(&mut self, record: &Record) -> Result<(), Reason> {
        let bytes = serde_json::to_vec(record).map_err(|_| Reason::WriteFailed)?;
        if self.journal_bytes.saturating_add(bytes.len() as u64 + 1) > self.bounds.catalog {
            return Err(Reason::ArchiveLimit);
        }
        self.journal
            .write_all(&bytes)
            .and_then(|_| self.journal.write_all(b"\n"))
            .map_err(|_| Reason::WriteFailed)?;
        self.journal.sync_all().map_err(|_| Reason::FlushFailed)?;
        self.journal_bytes += bytes.len() as u64 + 1;
        Ok(())
    }
    fn open_part(&mut self, stream: Stream) -> Result<(), Reason> {
        if self.count >= self.bounds.parts {
            return Err(Reason::ArchiveLimit);
        }
        let sequence = self.next[stream.slot()];
        let name = part_name(stream, sequence).map_err(|_| Reason::ArchiveLimit)?;
        let file = create(&self.directory.join(name)).map_err(|_| Reason::WriteFailed)?;
        // A new file accepts no content until its exact open record is durable.
        self.record(&Record::Open { stream, sequence })?;
        self.active[stream.slot()] = Some(ActivePart {
            file,
            sequence,
            bytes: 0,
            lines: 0,
            hash: Sha256::new(),
            failed: false,
        });
        self.next[stream.slot()] += 1;
        self.count += 1;
        Ok(())
    }
    fn seal(&mut self, stream: Stream) -> Result<(), Reason> {
        let Some(part) = self.active[stream.slot()].take() else {
            return Ok(());
        };
        let sync = part.file.sync_all();
        if sync.is_err() {
            return Err(Reason::FlushFailed);
        }
        if part.failed {
            return Err(Reason::WriteFailed);
        }
        if chat_status::checked_handle(&part.file, self.bounds.part)
            .map_err(|_| Reason::WriteFailed)?
            != part.bytes
        {
            return Err(Reason::WriteFailed);
        }
        let seal = Record::Seal {
            stream,
            sequence: part.sequence,
            bytes: part.bytes,
            lines: part.lines,
            sha256: format!("{:x}", part.hash.finalize()),
        };
        // Hash reflects accepted writes; full disk verification is outside the live lock.
        self.record(&seal)
    }
    pub(super) fn append(&mut self, stream: Stream, bytes: &[u8]) -> Result<(), Reason> {
        if let Some(reason) = &self.reason {
            return Err(reason.clone());
        }
        let result = self.append_inner(stream, bytes);
        result.map_err(|reason| self.latch(reason))
    }
    fn append_inner(&mut self, stream: Stream, bytes: &[u8]) -> Result<(), Reason> {
        let length = bytes.len() as u64 + 1;
        let line_limit = if stream == Stream::Events {
            64 * 1024
        } else {
            LINE_BYTES
        };
        if length > line_limit as u64
            || length > self.bounds.part
            || self.total.saturating_add(length) > self.bounds.total
        {
            return Err(Reason::ArchiveLimit);
        }
        // A fresh observation grants at most one part of write credit. This
        // retains the reserve without a filesystem syscall for every message.
        if self.space_budget < length || self.last_space_check.elapsed() >= Duration::from_secs(2) {
            self.space_budget = fs2::available_space(&self.directory)
                .map_err(|_| Reason::WriteFailed)?
                .saturating_sub(RESERVE)
                .min(self.bounds.part);
            self.last_space_check = Instant::now();
        }
        if self.space_budget < length {
            return Err(Reason::WriteFailed);
        }
        if self.active[stream.slot()]
            .as_ref()
            .is_some_and(|part| part.bytes + length > self.bounds.part)
        {
            self.seal(stream)?;
        }
        if self.active[stream.slot()].is_none() {
            self.open_part(stream)?;
        }
        let part = self.active[stream.slot()]
            .as_mut()
            .ok_or(Reason::WriteFailed)?;
        if part
            .file
            .write_all(bytes)
            .and_then(|_| part.file.write_all(b"\n"))
            .is_err()
        {
            part.failed = true;
            return Err(Reason::WriteFailed);
        }
        part.hash.update(bytes);
        part.hash.update(b"\n");
        part.bytes += length;
        part.lines += 1;
        self.total += length;
        self.space_budget -= length;
        if self.last_sync.elapsed() >= Duration::from_secs(2) {
            self.sync()?;
        }
        Ok(())
    }
    pub(super) fn sync(&mut self) -> Result<(), Reason> {
        let mut failure = None;
        for part in self.active.iter().flatten() {
            if part.file.sync_all().is_err() {
                failure = Some(Reason::FlushFailed);
            }
        }
        if self.journal.sync_all().is_err() {
            failure = Some(Reason::FlushFailed);
        }
        self.last_sync = Instant::now();
        match failure {
            Some(reason) => Err(self.latch(reason)),
            None => Ok(()),
        }
    }
    pub(super) fn close(mut self) -> ClosedParts {
        // Do not short-circuit a prior failure: every live handle gets its final sync.
        for stream in [Stream::Events, Stream::Source] {
            if let Err(reason) = self.seal(stream) {
                self.latch(reason);
            }
        }
        if self.journal.sync_all().is_err() {
            self.latch(Reason::FlushFailed);
        }
        ClosedParts {
            directory: self.directory.clone(),
            marker: self.marker.clone(),
            reason: self.reason.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ReplayPart {
    pub sequence: u32,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone)]
pub(super) struct ReplaySource {
    pub generation: String,
    pub attachment_id: String,
    pub complete: bool,
    pub events: Vec<ReplayPart>,
    pub total_event_bytes: u64,
    directory: PathBuf,
    parents: Arc<Vec<File>>,
}
pub(super) struct OwnedPart {
    pub file: File,
    _parents: Arc<Vec<File>>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CatalogProof {
    pub bytes: u64,
    pub sha256: String,
    pub parts: usize,
    pub total_bytes: u64,
}
pub(super) struct Catalog {
    pub proof: CatalogProof,
    pub closed: bool,
    pub events: Vec<ReplayPart>,
    source: Vec<ReplayPart>,
}
pub(super) fn load_catalog(directory: &Path) -> Result<Catalog, String> {
    let marker = chat_status::read_marker(directory)?.ok_or(ERROR)?;
    chat_status::identity(&marker, &marker.provider, &marker.job_id)?;
    if marker.version != 2 {
        return Err(ERROR.into());
    }
    let mut reader =
        chat_status::open_reader(&directory.join(JOURNAL), CATALOG_BYTES)?.ok_or(ERROR)?;
    let mut bytes = Vec::new();
    (&mut reader.file)
        .take(CATALOG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() as u64 > CATALOG_BYTES
        || chat_status::checked_handle(&reader.file, CATALOG_BYTES)? != bytes.len() as u64
    {
        return Err(ERROR.into());
    }
    let mut active = [None::<u32>, None];
    let mut next = [0u32, 0];
    let mut parts = [Vec::<ReplayPart>::new(), Vec::new()];
    let mut count = 0usize;
    let mut total = 0u64;
    let mut sound = true;
    let mut header = false;
    for (ordinal, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
        if !line.ends_with(b"\n") {
            sound = false;
            break;
        }
        let record: Record = serde_json::from_slice(line).map_err(|_| ERROR)?;
        if ordinal == 0 && !matches!(&record, Record::Header { .. }) {
            return Err(ERROR.into());
        }
        match record {
            Record::Header { marker: stored } => {
                if ordinal != 0 || stored != marker {
                    return Err(ERROR.into());
                }
                header = true;
            }
            Record::Open { stream, sequence } => {
                let slot = stream.slot();
                if active[slot].is_some() || sequence != next[slot] || count >= MAX_PARTS {
                    return Err(ERROR.into());
                }
                part_name(stream, sequence)?;
                active[slot] = Some(sequence);
                next[slot] += 1;
                count += 1;
            }
            Record::Seal {
                stream,
                sequence,
                bytes,
                lines,
                sha256,
            } => {
                let slot = stream.slot();
                if active[slot] != Some(sequence)
                    || bytes > PART_BYTES
                    || lines > bytes
                    || (bytes == 0) != (lines == 0)
                    || sha256.len() != 64
                    || !sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(ERROR.into());
                }
                total = total.checked_add(bytes).ok_or(ERROR)?;
                if total > MAX_TOTAL_BYTES {
                    return Err(ERROR.into());
                }
                parts[slot].push(ReplayPart {
                    sequence,
                    bytes,
                    sha256,
                });
                active[slot] = None;
            }
        }
    }
    if !header || next[0] == 0 {
        return Err(ERROR.into());
    }
    let [events, source] = parts;
    Ok(Catalog {
        proof: CatalogProof {
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            parts: count,
            total_bytes: total,
        },
        closed: sound && active.iter().all(Option::is_none),
        events,
        source,
    })
}
pub(super) fn verify_catalog(
    directory: &Path,
    catalog: &Catalog,
    cancel: &AtomicBool,
) -> Result<(), String> {
    for (stream, parts) in [
        (Stream::Events, &catalog.events),
        (Stream::Source, &catalog.source),
    ] {
        for part in parts {
            if cancel.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            let mut reader = chat_status::open_reader(
                &directory.join(part_name(stream, part.sequence)?),
                PART_BYTES,
            )?
            .ok_or(ERROR)?;
            if chat_status::checked_handle(&reader.file, PART_BYTES)? != part.bytes {
                return Err(ERROR.into());
            }
            let mut hash = Sha256::new();
            let mut read = 0u64;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Err("cancelled".into());
                }
                let length = reader.file.read(&mut buffer).map_err(|_| ERROR)?;
                if length == 0 {
                    break;
                }
                read += length as u64;
                if read > part.bytes {
                    return Err(ERROR.into());
                }
                hash.update(&buffer[..length]);
            }
            if read != part.bytes || format!("{:x}", hash.finalize()) != part.sha256 {
                return Err(ERROR.into());
            }
        }
    }
    if load_catalog(directory)?.proof != catalog.proof {
        return Err(ERROR.into());
    }
    Ok(())
}
pub(super) fn replay_source(
    directory: &Path,
    provider: &str,
    job_id: &str,
) -> Result<Option<ReplaySource>, String> {
    let parents = Arc::new(chat_status::pin_parents(&directory.join(JOURNAL))?);
    let catalog_exists = match std::fs::symlink_metadata(directory.join(JOURNAL)) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return Err(ERROR.into()),
    };
    let Some(marker) = chat_status::read_marker(directory)? else {
        if catalog_exists {
            return Err(ERROR.into());
        }
        return Ok(None);
    };
    if marker.version == 1 {
        if catalog_exists {
            return Err(ERROR.into());
        }
        return Ok(None);
    }
    chat_status::identity(&marker, provider, job_id)?;
    let catalog = load_catalog(directory)?;
    let (complete, terminal) =
        chat_status::multipart_replay_state(directory, &marker, &catalog.proof)?;
    if complete && !catalog.closed {
        return Err(ERROR.into());
    }
    let total_event_bytes = catalog.events.iter().map(|part| part.bytes).sum();
    Ok(Some(ReplaySource {
        generation: format!(
            "{}:{}:{terminal}",
            marker.attachment_id, catalog.proof.sha256
        ),
        attachment_id: marker.attachment_id,
        complete,
        events: catalog.events,
        total_event_bytes,
        directory: directory.into(),
        parents,
    }))
}
pub(super) fn open_replay_part(source: &ReplaySource, index: usize) -> Result<OwnedPart, String> {
    let part = source.events.get(index).ok_or(ERROR)?;
    let path = source
        .directory
        .join(part_name(Stream::Events, part.sequence)?);
    storage::safe_path(&path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(path).map_err(|_| ERROR)?;
    if chat_status::checked_handle(&file, PART_BYTES)? != part.bytes {
        return Err(ERROR.into());
    }
    Ok(OwnedPart {
        file,
        _parents: source.parents.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Read};
    fn setup(bounds: Bounds) -> (tempfile::TempDir, Marker, Writer) {
        let directory = tempfile::tempdir().unwrap();
        let marker = chat_status::begin_multipart(
            directory.path(),
            "soop",
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        let writer = Writer::with_bounds(directory.path(), &marker, bounds).unwrap();
        (directory, marker, writer)
    }
    fn small() -> Bounds {
        Bounds {
            part: 12,
            total: 200,
            parts: 20,
            ..BOUNDS
        }
    }
    #[test]
    fn rotate_whole_lines_both_streams_and_prove_exact_parts() {
        let (directory, marker, mut writer) = setup(small());
        for n in 0..7 {
            writer
                .append(Stream::Events, format!("row{n}").as_bytes())
                .unwrap();
            writer
                .append(Stream::Source, format!("raw{n}").as_bytes())
                .unwrap();
        }
        let closed = writer.close();
        assert!(closed.reason().is_none());
        let summary = chat_status::finish_multipart(
            directory.path(),
            &marker,
            closed,
            None,
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(summary.state, chat_status::TextState::Complete);
        let source = replay_source(directory.path(), "soop", &marker.job_id)
            .unwrap()
            .unwrap();
        assert!(source.complete);
        assert_eq!(source.events.len(), 4);
        assert_eq!(source.total_event_bytes, 35);
        let mut collected = String::new();
        for index in 0..source.events.len() {
            open_replay_part(&source, index)
                .unwrap()
                .file
                .read_to_string(&mut collected)
                .unwrap();
        }
        assert_eq!(
            collected,
            (0..7).map(|n| format!("row{n}\n")).collect::<String>()
        );
        assert_eq!(
            chat_status::recovery(directory.path(), "soop", &marker.job_id).unwrap(),
            summary
        );
    }
    #[test]
    fn all_caps_are_sticky_and_close_preserves_successful_seals() {
        for bounds in [
            Bounds {
                total: 9,
                ..small()
            },
            Bounds {
                parts: 1,
                ..small()
            },
            Bounds {
                catalog: 400,
                ..small()
            },
        ] {
            let (directory, marker, mut writer) = setup(bounds);
            let mut failed = false;
            for _ in 0..10 {
                if writer.append(Stream::Events, b"line").is_err() {
                    failed = true;
                    break;
                }
            }
            assert!(failed);
            assert!(writer.append(Stream::Source, b"raw").is_err());
            let closed = writer.close();
            assert!(closed.reason().is_some());
            let summary = chat_status::finish_multipart(
                directory.path(),
                &marker,
                closed,
                None,
                false,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(summary.state, chat_status::TextState::Partial);
            assert!(summary.originals_retained);
            assert!(directory.path().join("chat.events.jsonl").is_file());
        }
    }
    #[test]
    fn crash_exposes_sealed_prefix_only_and_recovery_never_resumes() {
        let (directory, marker, mut writer) = setup(small());
        writer.append(Stream::Events, b"first").unwrap();
        writer.append(Stream::Events, b"second").unwrap();
        drop(writer);
        let source = replay_source(directory.path(), "soop", &marker.job_id)
            .unwrap()
            .unwrap();
        assert!(!source.complete);
        assert_eq!(source.events.len(), 1);
        assert_eq!(source.total_event_bytes, 6);
        let current = fs::read(directory.path().join("chat.events.000001.jsonl")).unwrap();
        let summary = chat_status::recovery(directory.path(), "soop", &marker.job_id).unwrap();
        assert_eq!(summary.reasons, vec![Reason::Interrupted]);
        assert!(Writer::create(directory.path(), &marker).is_err());
        assert_eq!(
            fs::read(directory.path().join("chat.events.000001.jsonl")).unwrap(),
            current
        );
    }
    #[test]
    fn cancellation_publishes_partial_and_cannot_authorize_cleanup() {
        let (directory, marker, mut writer) = setup(small());
        writer.append(Stream::Events, b"safe").unwrap();
        let summary = chat_status::finish_multipart(
            directory.path(),
            &marker,
            writer.close(),
            None,
            false,
            &AtomicBool::new(true),
        )
        .unwrap();
        assert_eq!(summary.reasons, vec![Reason::Interrupted]);
        assert!(summary.originals_retained);
        assert!(
            !replay_source(directory.path(), "soop", &marker.job_id)
                .unwrap()
                .unwrap()
                .complete
        );
    }
    #[test]
    fn mismatched_identity_corrupt_proof_and_part_changes_fail_closed() {
        let (directory, marker, mut writer) = setup(small());
        writer.append(Stream::Events, b"safe").unwrap();
        chat_status::finish_multipart(
            directory.path(),
            &marker,
            writer.close(),
            None,
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(replay_source(directory.path(), "youtube", &marker.job_id).is_err());
        fs::write(directory.path().join("chat.events.jsonl"), b"evil\n").unwrap();
        assert!(chat_status::recovery(directory.path(), "soop", &marker.job_id).is_err());
        let proof = directory.path().join("chat-status.final.json");
        fs::write(&proof, b"{}").unwrap();
        assert!(replay_source(directory.path(), "soop", &marker.job_id).is_err());
        assert_eq!(fs::read(proof).unwrap(), b"{}");
    }
    #[test]
    fn malformed_tail_is_partial_but_duplicates_and_unknown_names_are_rejected() {
        let (directory, marker, mut writer) = setup(small());
        writer.append(Stream::Events, b"safe").unwrap();
        let _ = writer.close();
        let path = directory.path().join(JOURNAL);
        let before = fs::read(&path).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{incomplete")
            .unwrap();
        let source = replay_source(directory.path(), "soop", &marker.job_id)
            .unwrap()
            .unwrap();
        assert!(!source.complete);
        assert_eq!(source.events.len(), 1);
        fs::write(&path, &before).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"op\":\"open\",\"stream\":\"events\",\"sequence\":0}\n")
            .unwrap();
        assert!(replay_source(directory.path(), "soop", &marker.job_id).is_err());
        assert!(part_name(Stream::Events, 512).is_err());
        fs::write(directory.path().join("unknown.jsonl"), b"untouched").unwrap();
        assert_eq!(
            fs::read(directory.path().join("unknown.jsonl")).unwrap(),
            b"untouched"
        );
    }
    #[test]
    fn hardlinked_catalog_or_parts_are_never_adopted() {
        let (directory, marker, mut writer) = setup(small());
        writer.append(Stream::Events, b"safe").unwrap();
        let _ = writer.close();
        let source = replay_source(directory.path(), "soop", &marker.job_id)
            .unwrap()
            .unwrap();
        fs::hard_link(
            directory.path().join("chat.events.jsonl"),
            directory.path().join("extra"),
        )
        .unwrap();
        assert!(open_replay_part(&source, 0).is_err());
        fs::hard_link(
            directory.path().join(JOURNAL),
            directory.path().join("extra-catalog"),
        )
        .unwrap();
        assert!(replay_source(directory.path(), "soop", &marker.job_id).is_err());
    }
    #[test]
    fn actual_eight_mib_boundary_is_not_split_and_disk_hashes_match() {
        let (directory, marker, mut writer) = setup(BOUNDS);
        let row = vec![b'x'; LINE_BYTES - 1];
        for _ in 0..16 {
            writer.append(Stream::Source, &row).unwrap();
        }
        writer.append(Stream::Source, b"y").unwrap();
        let closed = writer.close();
        let catalog = load_catalog(directory.path()).unwrap();
        assert_eq!(catalog.source.len(), 2);
        assert_eq!(catalog.source[0].bytes, PART_BYTES);
        assert_eq!(catalog.source[1].bytes, 2);
        assert_eq!(
            catalog.source[0].sha256,
            format!(
                "{:x}",
                Sha256::digest(fs::read(directory.path().join("chat.source.jsonl")).unwrap())
            )
        );
        chat_status::finish_multipart(
            directory.path(),
            &marker,
            closed,
            None,
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
    }
    #[test]
    fn a_foreign_catalog_or_complete_invalid_record_cannot_be_a_partial_fallback() {
        let (a, marker, writer) = setup(small());
        let _ = writer.close();
        let (b, _, writer) = setup(small());
        let _ = writer.close();
        let journal = a.path().join(JOURNAL);
        let original = fs::read(&journal).unwrap();
        fs::write(&journal, fs::read(b.path().join(JOURNAL)).unwrap()).unwrap();
        assert!(replay_source(a.path(), "soop", &marker.job_id).is_err());
        fs::write(&journal, original).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&journal)
            .unwrap()
            .write_all(b"{invalid}\n")
            .unwrap();
        assert!(replay_source(a.path(), "soop", &marker.job_id).is_err());
        assert!(chat_status::recovery(a.path(), "soop", &marker.job_id).is_err());
    }
}
