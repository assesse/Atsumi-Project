//! YouTube-only durable raw storage, verified local preparation and publication.
#![allow(dead_code)]
mod codec_facts;
#[cfg(debug_assertions)]
mod automatic_qa;
#[cfg(debug_assertions)]
pub use automatic_qa::{inspect_automatic_raw_qa, RawAutomaticQaEvidence, RawAutomaticQaError};
mod files;
mod finalize;
mod index;
mod managed_probe;
mod packet_facts;
mod packet_probe;
mod packet_tape;
mod production;
pub(in crate::platform_media) mod raw_anchor;
pub(in crate::platform_media) use raw_anchor::{BoundaryObservation, RawTimelineFacts, replay_origin as raw_replay_origin};
mod reconstruct;
mod recovery;
mod schema;
mod stream_facts;
mod vp9_admission;
pub(in crate::platform_media) use production::{has_raw_proof, prepare_recording, recovery_identity};
pub(in crate::platform_media) use index::{
    inspect_aac_initialization, inspect_vp9_initialization, RawInitialization,
};
pub(in crate::platform_media) use finalize::{
    Fault as PublicationFault, PublicationExclusion, PublicationHandoff,
    PublicationSourceFacts, PublicationState, VerifiedCandidate,
};
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(in crate::platform_media) fn authored_raw_pair() -> [Vec<u8>; 2] { index::authored_pair() }

use files::{Directories, GuardedFile};
use schema::{hash, line, Partial, Proof, Receipt, Row, Seal, JOURNAL, PROOF, RAW};
pub(crate) use schema::{
    ClockDescriptor, Mode, Owner, SealCause, SpoolIdentity, Track, TrackExtent, Window,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Identity,
    Proof,
    Order,
    Bounds,
    Cancelled,
    Expired,
    Space,
    Path,
    Io,
    Corrupt,
    Sealed,
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Limits {
    chunk: u64,
    append: u64,
    raw: u64,
    elapsed_ms: u64,
    receipts: u64,
    journal: u64,
}
const LIMITS: Limits = Limits {
    chunk: 131072,
    append: 33554432,
    raw: 137438953472,
    elapsed_ms: 86400000,
    receipts: 1100000,
    journal: 1073741824,
};
const RESERVE: u64 = 1073741824;
const SEAL_ROOM: u64 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Create(u8),
    InitialRawSync(u8),
    ProofWrite,
    ProofSync,
    HeaderWrite,
    HeaderSync,
    BeforeRaw,
    RawWrite,
    RawSync,
    ReceiptWrite,
    ReceiptSync,
    BeforeAck,
    BeforeSeal,
    SealWrite,
    SealSync,
}
#[derive(Default)]
struct Runtime {
    #[cfg(test)]
    fail: Option<Step>,
    #[cfg(test)]
    cancel_at: Option<Step>,
    #[cfg(test)]
    expire_at: Option<Step>,
    #[cfg(test)]
    elapsed: Option<u64>,
    #[cfg(test)]
    available: Option<u64>,
    #[cfg(test)]
    steps: usize,
}
impl Runtime {
    fn step(&mut self, step: Step, cancel: &AtomicBool, limits: Limits) -> Result<()> {
        #[cfg(test)]
        {
            self.steps += 1;
            if self.cancel_at == Some(step) {
                cancel.store(true, Ordering::Release);
            }
            if self.expire_at == Some(step) {
                self.elapsed = Some(limits.elapsed_ms);
            }
            if self.fail == Some(step) {
                return Err(Error::Io);
            }
        }
        let _ = (step, cancel, limits);
        Ok(())
    }
    fn elapsed(&self, start: Instant) -> u64 {
        #[cfg(test)]
        if let Some(value) = self.elapsed {
            return value;
        }
        start.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    fn space(&self, directory: &Path, required: u64) -> Result<()> {
        #[cfg(test)]
        if let Some(value) = self.available {
            return if value >= required {
                Ok(())
            } else {
                Err(Error::Space)
            };
        }
        if fs2::available_space(directory).map_err(|_| Error::Space)? >= required {
            Ok(())
        } else {
            Err(Error::Space)
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Chunk {
    pub track: Track,
    pub append_index: u64,
    pub chunk_index: u32,
    pub final_chunk: bool,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ack {
    pub sequence: u64,
    pub track: Track,
    pub append_index: u64,
    pub chunk_index: u32,
    pub committed_end: u64,
    pub duplicate: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DurabilitySeal {
    pub cause: SealCause,
    pub tracks: [TrackExtent; 2],
    pub receipts: u64,
    pub incomplete_append: bool,
}

#[derive(Clone)]
struct Progress {
    lengths: [u64; 2],
    next: [u64; 2],
    hashes: [Sha256; 2],
    partial: Option<Partial>,
    receipts: u64,
    elapsed: u64,
}
impl Default for Progress {
    fn default() -> Self {
        Self {
            lengths: [0; 2],
            next: [0; 2],
            hashes: [Sha256::new(), Sha256::new()],
            partial: None,
            receipts: 0,
            elapsed: 0,
        }
    }
}
impl Progress {
    fn validate(&self, row: &Receipt, limits: Limits) -> Result<()> {
        let track = row.track.index();
        let len = row.length as u64;
        if row.sequence != self.receipts
            || row.append_index != self.next[track]
            || row.offset != self.lengths[track]
            || len == 0
            || len > limits.chunk
            || (!row.final_chunk && len != limits.chunk)
            || row.elapsed_ms < self.elapsed
            || row.elapsed_ms >= limits.elapsed_ms
            || !schema::digest_valid(&row.payload_sha256)
            || !schema::digest_valid(&row.previous_record_sha256)
        {
            return Err(Error::Order);
        }
        if self.receipts >= limits.receipts
            || self.lengths[0]
                .checked_add(self.lengths[1])
                .and_then(|n| n.checked_add(len))
                .is_none_or(|n| n > limits.raw)
        {
            return Err(Error::Bounds);
        }
        let previous = match &self.partial {
            Some(p)
                if p.track == row.track
                    && p.append_index == row.append_index
                    && p.next_chunk == row.chunk_index =>
            {
                p.bytes
            }
            None if row.chunk_index == 0 => 0,
            _ => return Err(Error::Order),
        };
        if previous.checked_add(len).is_none_or(|n| n > limits.append)
            || row.chunk_index.checked_add(1).is_none()
            || (row.final_chunk && row.append_index.checked_add(1).is_none())
        {
            return Err(Error::Bounds);
        }
        Ok(())
    }
    fn commit(&mut self, row: &Receipt, bytes: &[u8]) {
        let track = row.track.index();
        let bytes_before = self.partial.as_ref().map_or(0, |p| p.bytes);
        self.lengths[track] += row.length as u64;
        self.hashes[track].update(bytes);
        self.receipts += 1;
        self.elapsed = row.elapsed_ms;
        if row.final_chunk {
            self.next[track] += 1;
            self.partial = None;
        } else {
            self.partial = Some(Partial {
                track: row.track,
                append_index: row.append_index,
                next_chunk: row.chunk_index + 1,
                bytes: bytes_before + row.length as u64,
            });
        }
    }
    fn extents(&self) -> [TrackExtent; 2] {
        std::array::from_fn(|i| TrackExtent {
            bytes: self.lengths[i],
            sha256: format!("{:x}", self.hashes[i].clone().finalize()),
            next_append: self.next[i],
        })
    }
}

pub(crate) struct SpoolWriter {
    identity: SpoolIdentity,
    directory: PathBuf,
    directories: Directories,
    proof: GuardedFile,
    raw: [GuardedFile; 2],
    journal: GuardedFile,
    progress: Progress,
    chain: String,
    journal_bytes: u64,
    last: Option<Receipt>,
    cancel: Arc<AtomicBool>,
    start: Instant,
    limits: Limits,
    terminal: Option<Error>,
    runtime: Runtime,
}
impl SpoolWriter {
    pub(crate) fn create_new(
        root: &Path,
        identity: SpoolIdentity,
        clock: ClockDescriptor,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self> {
        Self::create(root, identity, clock, cancel, LIMITS, Runtime::default())
    }
    fn create(
        root: &Path,
        identity: SpoolIdentity,
        clock: ClockDescriptor,
        cancel: Arc<AtomicBool>,
        limits: Limits,
        mut runtime: Runtime,
    ) -> Result<Self> {
        identity.validate()?;
        clock.validate()?;
        if cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        let directory =
            super::storage::job_dir(root, "youtube", &identity.job_id).map_err(|_| Error::Path)?;
        let parent = directory.parent().ok_or(Error::Path)?;
        let mut directories = Directories::hold(parent)?;
        runtime.space(parent, RESERVE + 8192 + 2 * SEAL_ROOM)?;
        directories.create_child(&directory)?;
        runtime.step(Step::Create(0), &cancel, limits)?;
        let mut proof = GuardedFile::open(&directory.join(PROOF), true)?;
        runtime.step(Step::Create(1), &cancel, limits)?;
        let video = GuardedFile::open(&directory.join(RAW[0]), true)?;
        runtime.step(Step::Create(2), &cancel, limits)?;
        let audio = GuardedFile::open(&directory.join(RAW[1]), true)?;
        runtime.step(Step::Create(3), &cancel, limits)?;
        let raw = [video, audio];
        let mut journal = GuardedFile::open(&directory.join(JOURNAL), true)?;
        runtime.step(Step::Create(4), &cancel, limits)?;
        let bytes = serde_json::to_vec(&Proof {
            version: 1,
            provider: schema::Provider::Youtube,
            branch: schema::Branch::RawV1,
            identity: identity.clone(),
            clock,
            video: schema::VideoFile::Video,
            audio: schema::AudioFile::Audio,
            journal: schema::JournalFile::Journal,
            limits,
        })
        .map_err(|_| Error::Proof)?;
        if bytes.len() > 8192 {
            return Err(Error::Proof);
        }
        runtime.space(&directory, RESERVE + bytes.len() as u64 + 2 * SEAL_ROOM)?;
        proof.file.write_all(&bytes).map_err(|_| Error::Io)?;
        runtime.step(Step::ProofWrite, &cancel, limits)?;
        proof.file.sync_all().map_err(|_| Error::Io)?;
        runtime.step(Step::ProofSync, &cancel, limits)?;
        for (i, file) in raw.iter().enumerate() {
            file.file.sync_all().map_err(|_| Error::Io)?;
            runtime.step(Step::InitialRawSync(i as u8), &cancel, limits)?;
        }
        let header = line(&Row::Header {
            version: 1,
            proof_sha256: hash(&bytes),
        })?;
        runtime.space(&directory, RESERVE + header.len() as u64 + SEAL_ROOM)?;
        journal.file.write_all(&header).map_err(|_| Error::Io)?;
        runtime.step(Step::HeaderWrite, &cancel, limits)?;
        journal.file.sync_all().map_err(|_| Error::Io)?;
        runtime.step(Step::HeaderSync, &cancel, limits)?;
        let writer = Self {
            identity,
            directory,
            directories,
            proof,
            raw,
            journal,
            progress: Progress::default(),
            chain: hash(&header),
            journal_bytes: header.len() as u64,
            last: None,
            cancel,
            start: Instant::now(),
            limits,
            terminal: None,
            runtime,
        };
        writer.guards()?;
        writer.live()?;
        Ok(writer)
    }
    fn live(&self) -> Result<()> {
        if let Some(error) = self.terminal {
            return Err(error);
        }
        if self.cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        if self.runtime.elapsed(self.start) >= self.limits.elapsed_ms {
            return Err(Error::Expired);
        }
        Ok(())
    }
    fn guards(&self) -> Result<()> {
        self.directories.check()?;
        self.proof.check()?;
        for (i, file) in self.raw.iter().enumerate() {
            if file.check()? != self.progress.lengths[i] {
                return Err(Error::Corrupt);
            }
        }
        if self.journal.check()? != self.journal_bytes {
            return Err(Error::Corrupt);
        }
        Ok(())
    }
    pub(crate) fn append(&mut self, identity: &SpoolIdentity, chunk: Chunk) -> Result<Ack> {
        if identity != &self.identity {
            return Err(Error::Identity);
        }
        let result = self.append_inner(chunk);
        if let Err(error) = result {
            self.terminal.get_or_insert(error);
        }
        result
    }
    fn append_inner(&mut self, chunk: Chunk) -> Result<Ack> {
        self.live()?;
        self.guards()?;
        if chunk.bytes.is_empty() || chunk.bytes.len() as u64 > self.limits.chunk {
            return Err(Error::Bounds);
        }
        let payload_hash = hash(&chunk.bytes);
        let ack = |receipt: &Receipt, duplicate| Ack {
            sequence: receipt.sequence,
            track: receipt.track,
            append_index: receipt.append_index,
            chunk_index: receipt.chunk_index,
            committed_end: receipt.offset + receipt.length as u64,
            duplicate,
        };
        if let Some(last) = &self.last {
            if last.track == chunk.track
                && last.append_index == chunk.append_index
                && last.chunk_index == chunk.chunk_index
                && last.final_chunk == chunk.final_chunk
                && last.length as usize == chunk.bytes.len()
                && last.payload_sha256 == payload_hash
            {
                self.live()?;
                return Ok(ack(last, true));
            }
        }
        let row = Receipt {
            sequence: self.progress.receipts,
            track: chunk.track,
            append_index: chunk.append_index,
            chunk_index: chunk.chunk_index,
            offset: self.progress.lengths[chunk.track.index()],
            length: chunk.bytes.len() as u32,
            final_chunk: chunk.final_chunk,
            payload_sha256: payload_hash,
            elapsed_ms: self.runtime.elapsed(self.start),
            previous_record_sha256: self.chain.clone(),
        };
        self.progress.validate(&row, self.limits)?;
        let encoded = line(&Row::Chunk(row.clone()))?;
        let next_journal = self
            .journal_bytes
            .checked_add(encoded.len() as u64)
            .filter(|n| {
                n.checked_add(SEAL_ROOM)
                    .is_some_and(|n| n <= self.limits.journal)
            })
            .ok_or(Error::Bounds)?;
        let required = RESERVE
            .checked_add(SEAL_ROOM)
            .and_then(|n| n.checked_add(encoded.len() as u64))
            .and_then(|n| n.checked_add(chunk.bytes.len() as u64))
            .ok_or(Error::Bounds)?;
        self.runtime.space(&self.directory, required)?;
        self.runtime
            .step(Step::BeforeRaw, &self.cancel, self.limits)?;
        self.live()?;
        let raw = &mut self.raw[chunk.track.index()];
        raw.file.write_all(&chunk.bytes).map_err(|_| Error::Io)?;
        self.runtime
            .step(Step::RawWrite, &self.cancel, self.limits)?;
        raw.file.sync_all().map_err(|_| Error::Io)?;
        self.runtime
            .step(Step::RawSync, &self.cancel, self.limits)?;
        self.live()?;
        self.directories.check()?;
        self.proof.check()?;
        if self.raw[chunk.track.index()].check()? != row.offset + row.length as u64 {
            return Err(Error::Corrupt);
        }
        if self.journal.check()? != self.journal_bytes {
            return Err(Error::Corrupt);
        }
        self.runtime
            .space(&self.directory, RESERVE + SEAL_ROOM + encoded.len() as u64)?;
        self.journal
            .file
            .write_all(&encoded)
            .map_err(|_| Error::Io)?;
        self.runtime
            .step(Step::ReceiptWrite, &self.cancel, self.limits)?;
        self.journal.file.sync_all().map_err(|_| Error::Io)?;
        self.runtime
            .step(Step::ReceiptSync, &self.cancel, self.limits)?;
        self.progress.commit(&row, &chunk.bytes);
        self.chain = hash(&encoded);
        self.journal_bytes = next_journal;
        self.last = Some(row.clone());
        self.runtime
            .step(Step::BeforeAck, &self.cancel, self.limits)?;
        self.guards()?;
        self.live()?;
        Ok(ack(&row, false))
    }
    pub(crate) fn seal(
        &mut self,
        identity: &SpoolIdentity,
        cause: SealCause,
    ) -> Result<DurabilitySeal> {
        if identity != &self.identity {
            return Err(Error::Identity);
        }
        if let Some(error) = self.terminal {
            return Err(error);
        }
        // A healthy revoked writer can persist a cancelled seal, but no more media.
        self.terminal = Some(Error::Sealed);
        let result = self.seal_inner(cause);
        if let Err(error) = result {
            self.terminal = Some(error);
        }
        result
    }
    fn seal_inner(&mut self, cause: SealCause) -> Result<DurabilitySeal> {
        self.guards()?;
        self.runtime
            .step(Step::BeforeSeal, &self.cancel, self.limits)?;
        for file in &self.raw {
            file.file.sync_all().map_err(|_| Error::Io)?;
        }
        // Revoke/expiry can arrive during the final raw flush, before row encoding.
        let cause = if self.cancel.load(Ordering::Acquire) {
            SealCause::Cancelled
        } else if self.runtime.elapsed(self.start) >= self.limits.elapsed_ms {
            SealCause::Limit
        } else if self.progress.partial.is_some() && cause == SealCause::Stopped {
            SealCause::Interrupted
        } else {
            cause
        };
        let seal = Seal {
            cause,
            receipts: self.progress.receipts,
            tracks: self.progress.extents(),
            partial: self.progress.partial.clone(),
            previous_record_sha256: self.chain.clone(),
        };
        let bytes = line(&Row::Seal(seal.clone()))?;
        let length = self
            .journal_bytes
            .checked_add(bytes.len() as u64)
            .filter(|n| *n <= self.limits.journal)
            .ok_or(Error::Bounds)?;
        self.runtime
            .space(&self.directory, RESERVE + bytes.len() as u64)?;
        self.journal.file.write_all(&bytes).map_err(|_| Error::Io)?;
        self.runtime
            .step(Step::SealWrite, &self.cancel, self.limits)?;
        self.journal.file.sync_all().map_err(|_| Error::Io)?;
        self.runtime
            .step(Step::SealSync, &self.cancel, self.limits)?;
        self.journal_bytes = length;
        self.guards()?;
        // An already durable row is never rewritten. A later revoke is reported
        // as committed-but-unacknowledged rather than a successful stop reply.
        if self.cancel.load(Ordering::Acquire) && cause != SealCause::Cancelled {
            return Err(Error::Cancelled);
        }
        if self.runtime.elapsed(self.start) >= self.limits.elapsed_ms
            && !matches!(cause, SealCause::Cancelled | SealCause::Limit)
        {
            return Err(Error::Expired);
        }
        Ok(DurabilitySeal {
            cause,
            tracks: seal.tracks,
            receipts: seal.receipts,
            incomplete_append: seal.partial.is_some(),
        })
    }
}

#[cfg(test)]
use recovery::{CommittedPrefix, PrefixState};
