//! Dormant one-unit packet evidence. No capture, remux, publication or cleanup.
//! Reported stream timebase/scalars are limited evidence, not full codec validation.
use super::{
    codec_facts,
    index::{Configuration, Format, Unit},
    managed_probe::{self, ManagedProbe},
    packet_facts::{self, FoldResult, TimeBase},
    recovery::{PrefixState, SelectedUnit, SpanFacts, UnitSpan},
    stream_facts::{ProbeFacts, ProjectionDecoder, StreamFacts},
    ClockDescriptor, SealCause, SpoolIdentity, Track, TrackExtent,
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

const CHUNK: usize = 131_072;
const STDOUT_CHUNK: usize = 16_384;
const STDERR_CAP: usize = 65_536;
const INPUT_CAP: u64 = 128 * 1024 * 1024 * 1024;
const BUDGET: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(50);
const ENTRIES: &str = "packet=stream_index,pts,dts,duration,size,flags,data_hash:packet_side_data=side_data_type,skip_samples,discard_padding,skip_reason,discard_reason:stream=index,codec_name,codec_type,time_base,width,height,sample_rate,channels:stream_tags=:stream_disposition=:stream_side_data=";
const PROJECTION: &str = "compact=p=1:nk=0:e=c:sv=fail";
const METADATA_BUDGET: Duration = Duration::from_secs(30);
const METADATA_CAP: usize = 65_536;
const METADATA_ENTRIES: &str = "stream=index,codec_name,codec_type,codec_tag_string,profile,width,height,pix_fmt,sample_aspect_ratio,color_range,color_space,color_transfer,color_primaries,chroma_location,field_order,sample_rate,channels,channel_layout,initial_padding,extradata_size,extradata_hash,time_base:stream_side_data=side_data_type:stream_tags=:stream_disposition=";

#[cfg(all(test, windows))]
mod continuous_join_test;
#[cfg(all(test, windows))]
mod metadata_input_test;
mod owned;
pub(super) use owned::{
    decode_owned, metadata_owned, probe_owned, probe_selected_owned, remux_owned, remux_selected_owned,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fault {
    SourceInvalid,
    SourceChanged,
    UnsupportedInput,
    ToolUnavailable,
    Containment,
    Spawn,
    InputIo,
    InputShort,
    InputHash,
    OutputIo,
    OutputLimit,
    Packet(packet_facts::Fault),
    Codec(codec_facts::Fault),
    StderrOutput,
    StderrLimit,
    ChildFailed,
    WorkerFailed,
    Deadline,
    Cancelled,
}
type Result<T> = std::result::Result<T, Fault>;

/// Immutable copied facts, not a source lease or media-ready certificate.
#[derive(Clone, Debug)]
pub(super) struct SourceFacts {
    pub identity: SpoolIdentity,
    pub clock: ClockDescriptor,
    pub tracks: [TrackExtent; 2],
    pub receipts: u64,
    pub state: PrefixState,
    pub seal_cause: Option<SealCause>,
    pub incomplete_append: bool,
    pub torn_journal: bool,
    pub uncommitted_bytes: [u64; 2],
    pub track: Track,
    pub unit: Unit,
}
impl SourceFacts {
    fn copy(f: SpanFacts<'_>) -> Result<Self> {
        let expected = match f.track {
            Track::Video => Format::Vp9Webm,
            Track::Audio => Format::AacMp4,
        };
        if !matches!(f.state, PrefixState::Sealed | PrefixState::Interrupted)
            || f.unit.facts.configuration.format != expected
            || f.unit.facts.start >= f.unit.facts.init_end
            || f.unit.facts.init_end >= f.unit.facts.end
            || f.unit.facts.end > f.tracks[f.track.index()].bytes
        {
            return Err(Fault::SourceInvalid);
        }
        Ok(Self {
            identity: f.identity.clone(),
            clock: f.clock.clone(),
            tracks: f.tracks.clone(),
            receipts: f.receipts,
            state: f.state,
            seal_cause: f.seal_cause,
            incomplete_append: f.incomplete_append,
            torn_journal: f.torn_journal,
            uncommitted_bytes: f.uncommitted_bytes,
            track: f.track,
            unit: f.unit.clone(),
        })
    }
}
#[derive(Debug)]
pub(super) struct PacketProbeEvidence {
    pub source: SourceFacts,
    pub input_sha256: [u8; 32],
    pub packets: FoldResult,
    pub stream: StreamFacts,
    pub projection_version: u8,
    pub tool_package: &'static str,
    pub tool_archive_sha256: &'static str,
}

#[derive(Debug)]
pub(super) struct CodecProbeEvidence {
    pub source: SourceFacts,
    pub input_sha256: [u8; 32],
    pub codec: codec_facts::SourceFacts,
    pub tool_package: &'static str,
    pub tool_archive_sha256: &'static str,
}

fn source_error(e: super::Error) -> Fault {
    if e == super::Error::Cancelled {
        Fault::Cancelled
    } else {
        Fault::SourceChanged
    }
}
fn tool_error(e: managed_probe::Fault) -> Fault {
    match e {
        managed_probe::Fault::Cancelled => Fault::Cancelled,
        managed_probe::Fault::Deadline => Fault::Deadline,
        _ => Fault::ToolUnavailable,
    }
}
fn checkpoint(cancel: &AtomicBool, deadline: Instant) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err(Fault::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(Fault::Deadline);
    }
    Ok(())
}

pub(super) fn probe(
    selected: &mut SelectedUnit,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
) -> Result<PacketProbeEvidence> {
    let deadline = Instant::now() + BUDGET;
    checkpoint(cancel, deadline)?;
    #[cfg(not(windows))]
    {
        let _ = (selected, tool);
        Err(Fault::UnsupportedInput)
    }
    #[cfg(windows)]
    {
        let evidence = selected
            .with_span(cancel, |span| {
                probe_span(span, tool, cancel, deadline, |_| Ok(()))
            })
            .map_err(source_error)??;
        // The enclosing source capability can revoke apparent per-unit success.
        tool.check(cancel, deadline).map_err(tool_error)?;
        checkpoint(cancel, deadline)?;
        Ok(evidence)
    }
}

/// A sequential visit lends the same native capability as SelectedUnit. The
/// consumer must not commit its output until this AND the enclosing visit pass.
/// The caller's overall deadline is never extended by starting another unit.
pub(super) fn probe_span(
    mut span: UnitSpan<'_>,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
    consume: impl FnMut(packet_facts::PacketFact) -> std::result::Result<(), packet_facts::Fault> + Send,
) -> Result<PacketProbeEvidence> {
    let deadline = deadline.min(Instant::now() + BUDGET);
    checkpoint(cancel, deadline)?;
    #[cfg(not(windows))]
    {
        let _ = (span, tool, consume);
        Err(Fault::UnsupportedInput)
    }
    #[cfg(windows)]
    {
        let source = SourceFacts::copy(span.facts())?;
        let range = Range::new(source.unit.facts.start, source.unit.facts.end)?;
        let mut reader = span.take_reader().map_err(source_error)?;
        let command = command(tool, source.unit.facts.configuration.format);
        let result = run_with_callback(
            command,
            &mut reader,
            range,
            source.track,
            0,
            cancel,
            deadline,
            || {
                checkpoint(cancel, deadline)?;
                span.check().map_err(source_error)?;
                tool.check(cancel, deadline).map_err(tool_error)?;
                checkpoint(cancel, deadline)
            },
            consume,
        )?;
        validate_stream(
            source.track,
            &source.unit.facts.configuration,
            &result.stream,
        )?;
        span.check().map_err(source_error)?;
        tool.check(cancel, deadline).map_err(tool_error)?;
        checkpoint(cancel, deadline)?;
        Ok(PacketProbeEvidence {
            source,
            input_sha256: result.hash,
            packets: result.packets,
            stream: result.stream,
            projection_version: 2,
            tool_package: managed_probe::PACKAGE,
            tool_archive_sha256: managed_probe::ARCHIVE_SHA256,
        })
    }
}

/// Whole-stream observations from one exact, held source unit. As with packet
/// evidence, the enclosing sequential visit must still complete successfully.
pub(super) fn probe_codec_span(
    mut span: UnitSpan<'_>,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<CodecProbeEvidence> {
    let deadline = deadline.min(Instant::now() + METADATA_BUDGET);
    checkpoint(cancel, deadline)?;
    #[cfg(not(windows))]
    {
        let _ = (span, tool);
        Err(Fault::UnsupportedInput)
    }
    #[cfg(windows)]
    {
        let source = SourceFacts::copy(span.facts())?;
        let range = Range::new(source.unit.facts.start, source.unit.facts.end)?;
        let mut reader = span.take_reader().map_err(source_error)?;
        let (input_sha256, bytes) = run_output(
            metadata_command(tool, source.unit.facts.configuration.format),
            &mut reader,
            range,
            cancel,
            deadline,
            || {
                checkpoint(cancel, deadline)?;
                span.check().map_err(source_error)?;
                tool.check(cancel, deadline).map_err(tool_error)?;
                checkpoint(cancel, deadline)
            },
            |output, state| metadata_bytes(output, state),
        )?;
        let codec =
            codec_facts::source(&bytes, &source.unit.facts.configuration).map_err(Fault::Codec)?;
        span.check().map_err(source_error)?;
        tool.check(cancel, deadline).map_err(tool_error)?;
        checkpoint(cancel, deadline)?;
        Ok(CodecProbeEvidence {
            source,
            input_sha256,
            codec,
            tool_package: managed_probe::PACKAGE,
            tool_archive_sha256: managed_probe::ARCHIVE_SHA256,
        })
    }
}

/// Cross-check only the reported source-local scalars. This neither compares
/// opaque semantic/extradata hashes nor clears codec_packet_validation_pending.
fn validate_stream(
    track: Track,
    configuration: &Configuration,
    stream: &StreamFacts,
) -> Result<()> {
    if stream.track != track {
        return Err(Fault::UnsupportedInput);
    }
    let scale = u32::try_from(configuration.timebase).map_err(|_| Fault::UnsupportedInput)?;
    let expected = match (track, configuration.format) {
        (Track::Video, Format::Vp9Webm)
            if stream.width == configuration.width && stream.height == configuration.height =>
        {
            TimeBase::new(scale, 1_000_000_000)
        }
        (Track::Audio, Format::AacMp4)
            if stream.sample_rate == configuration.sample_rate
                && stream.channels == configuration.channels =>
        {
            TimeBase::new(1, scale)
        }
        _ => return Err(Fault::UnsupportedInput),
    }
    .map_err(|_| Fault::UnsupportedInput)?;
    if stream.time_base != expected {
        return Err(Fault::UnsupportedInput);
    }
    Ok(())
}

fn input_arguments(format: Format) -> Vec<&'static str> {
    let mut args = vec![
        "-v",
        "error",
        "-hide_banner",
        "-f",
        match format {
            Format::Vp9Webm => "matroska",
            Format::AacMp4 => "mov",
        },
        "-protocol_whitelist",
        "pipe",
        "-blocksize",
        "131072",
    ];
    if format == Format::AacMp4 {
        args.extend(["-enable_drefs", "0", "-use_absolute_path", "0"]);
    }
    args.extend(["-i", "pipe:0"]);
    args
}
fn arguments(format: Format) -> Vec<&'static str> {
    let mut args = input_arguments(format);
    args.extend([
        "-show_packets",
        "-show_data_hash",
        "sha256",
        "-show_entries",
        ENTRIES,
        "-of",
        PROJECTION,
    ]);
    args
}
fn metadata_arguments(format: Format) -> Vec<&'static str> {
    let mut args = input_arguments(format);
    // Metadata alone may exit before stdin is drained. The guarded producer
    // requires every selected byte delivered; count without exposing packets.
    args.extend([
        "-count_packets",
        "-show_data_hash",
        "sha256",
        "-show_entries",
        METADATA_ENTRIES,
        "-of",
        "json",
    ]);
    args
}
fn metadata_command(tool: &ManagedProbe, format: Format) -> Command {
    let mut command = Command::new(tool.executable());
    command
        .current_dir(tool.cwd())
        .env_clear()
        .envs(tool.environment())
        .args(metadata_arguments(format));
    command
}
fn command(tool: &ManagedProbe, format: Format) -> Command {
    // Kept private: the only production caller supplies the held native lease.
    configured_command(tool.executable(), tool.cwd(), tool.environment(), format)
}
fn configured_command(
    executable: &std::path::Path,
    cwd: &std::path::Path,
    environment: [(&'static str, &std::ffi::OsStr); 2],
    format: Format,
) -> Command {
    let mut command = Command::new(executable);
    command
        .current_dir(cwd)
        .env_clear()
        .envs(environment)
        .args(arguments(format));
    command
}

#[derive(Clone, Copy)]
struct Range {
    start: u64,
    end: u64,
}
impl Range {
    fn new(start: u64, end: u64) -> Result<Self> {
        if start >= end || end > INPUT_CAP {
            Err(Fault::SourceInvalid)
        } else {
            Ok(Self { start, end })
        }
    }
    fn len(self) -> u64 {
        self.end - self.start
    }
}
struct Control<'a> {
    cancel: &'a AtomicBool,
    deadline: Instant,
    failure: Mutex<Option<Fault>>,
}
impl<'a> Control<'a> {
    fn new(cancel: &'a AtomicBool, deadline: Instant) -> Self {
        Self {
            cancel,
            deadline,
            failure: Mutex::new(None),
        }
    }
    fn fail(&self, error: Fault) -> Fault {
        match self.failure.lock() {
            Ok(mut first) => *first.get_or_insert(error),
            Err(_) => Fault::WorkerFailed,
        }
    }
    fn check(&self) -> Result<()> {
        if let Some(error) = *self.failure.lock().map_err(|_| Fault::WorkerFailed)? {
            return Err(error);
        }
        checkpoint(self.cancel, self.deadline).map_err(|e| self.fail(e))
    }
    fn worker<T>(&self, work: impl FnOnce() -> Result<T>) -> Result<T> {
        // Scoped workers cannot unwind past ownership cleanup or leave silent EOF.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
            .unwrap_or(Err(Fault::WorkerFailed))
            .map_err(|e| self.fail(e))
    }
}

fn hash_range<R: Read + Seek>(
    reader: &mut R,
    range: Range,
    check: impl Fn() -> Result<()>,
) -> Result<[u8; 32]> {
    check()?;
    reader
        .seek(SeekFrom::Start(range.start))
        .map_err(|_| Fault::InputIo)?;
    let mut remaining = range.len();
    let mut buffer = [0; CHUNK];
    let mut hash = Sha256::new();
    while remaining > 0 {
        check()?;
        let cap = remaining.min(CHUNK as u64) as usize;
        let n = match reader.read(&mut buffer[..cap]) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|_| Fault::InputIo)?,
        };
        if n == 0 {
            return Err(Fault::InputShort);
        }
        hash.update(&buffer[..n]);
        remaining -= n as u64;
    }
    check()?;
    Ok(hash.finalize().into())
}
fn transfer<R: Read + Seek, W: Write>(
    reader: &mut R,
    mut writer: W,
    range: Range,
    control: &Control<'_>,
) -> Result<[u8; 32]> {
    control.check()?;
    reader
        .seek(SeekFrom::Start(range.start))
        .map_err(|_| Fault::InputIo)?;
    let mut remaining = range.len();
    let mut buffer = [0; CHUNK];
    let mut hash = Sha256::new();
    while remaining > 0 {
        control.check()?;
        let cap = remaining.min(CHUNK as u64) as usize;
        let n = match reader.read(&mut buffer[..cap]) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|_| Fault::InputIo)?,
        };
        if n == 0 {
            return Err(Fault::InputShort);
        }
        let mut sent = 0;
        while sent < n {
            control.check()?;
            let written = match writer.write(&buffer[sent..n]) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result.map_err(|_| Fault::InputIo)?,
            };
            if written == 0 {
                return Err(Fault::InputShort);
            }
            hash.update(&buffer[sent..sent + written]);
            sent += written;
        }
        remaining -= n as u64;
    }
    control.check()?;
    // No buffered wrapper: every counted byte was written to the OS pipe.
    drop(writer);
    Ok(hash.finalize().into())
}
fn packets<R: Read>(mut reader: R, track: Track, control: &Control<'_>) -> Result<ProbeFacts> {
    packets_with_callback(&mut reader, track, 0, control, |_| Ok(()))
}

fn packets_with_callback<R: Read>(
    mut reader: R,
    track: Track,
    stream_index: u8,
    control: &Control<'_>,
    mut consume: impl FnMut(packet_facts::PacketFact) -> std::result::Result<(), packet_facts::Fault>,
) -> Result<ProbeFacts> {
    let mut decoder =
        ProjectionDecoder::with_stream_index(track, stream_index).map_err(Fault::Packet)?;
    let mut buffer = [0; STDOUT_CHUNK];
    loop {
        control.check()?;
        let n = match reader.read(&mut buffer) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|_| Fault::OutputIo)?,
        };
        control.check()?;
        if n == 0 {
            break;
        }
        decoder
            .feed_with(&buffer[..n], &mut consume)
            .map_err(Fault::Packet)?;
    }
    let result = decoder.finish().map_err(Fault::Packet)?;
    control.check()?;
    Ok(result)
}
fn stderr<R: Read>(mut reader: R, control: &Control<'_>) -> Result<()> {
    let mut buffer = [0; 4096];
    let mut total = 0usize;
    loop {
        control.check()?;
        let n = match reader.read(&mut buffer) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|_| Fault::OutputIo)?,
        };
        control.check()?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n)
            .filter(|n| *n <= STDERR_CAP)
            .ok_or(Fault::StderrLimit)?;
    }
    if total == 0 {
        Ok(())
    } else {
        Err(Fault::StderrOutput)
    }
}

fn metadata_bytes<R: Read>(mut reader: R, control: &Control<'_>) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(4096);
    let mut buffer = [0; 4096];
    loop {
        control.check()?;
        let n = match reader.read(&mut buffer) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|_| Fault::OutputIo)?,
        };
        control.check()?;
        if n == 0 {
            return Ok(bytes);
        }
        if bytes
            .len()
            .checked_add(n)
            .filter(|n| *n <= METADATA_CAP)
            .is_none()
        {
            return Err(Fault::OutputLimit);
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
}

#[derive(Debug)]
struct RawEvidence {
    hash: [u8; 32],
    packets: FoldResult,
    stream: StreamFacts,
}

#[cfg(windows)]
fn run<R: Read + Seek + Send>(
    command: Command,
    reader: &mut R,
    range: Range,
    track: Track,
    cancel: &AtomicBool,
    deadline: Instant,
    guard: impl FnMut() -> Result<()>,
) -> Result<RawEvidence> {
    run_with_callback(
        command,
        reader,
        range,
        track,
        0,
        cancel,
        deadline,
        guard,
        |_| Ok(()),
    )
}

/// The consumer is scoped and provisional. It cannot bypass the enclosing
/// source/tool guards or turn a failed process into committed evidence.
#[cfg(windows)]
fn run_with_callback<R: Read + Seek + Send>(
    command: Command,
    reader: &mut R,
    range: Range,
    track: Track,
    stream_index: u8,
    cancel: &AtomicBool,
    deadline: Instant,
    guard: impl FnMut() -> Result<()>,
    consume: impl FnMut(packet_facts::PacketFact) -> std::result::Result<(), packet_facts::Fault> + Send,
) -> Result<RawEvidence> {
    let (hash, facts) = run_output(
        command,
        reader,
        range,
        cancel,
        deadline,
        guard,
        move |output, state| packets_with_callback(output, track, stream_index, state, consume),
    )?;
    Ok(RawEvidence {
        hash,
        packets: facts.packets,
        stream: facts.stream,
    })
}

// Private to this YouTube probe. Only the fixed packet projection or bounded
// codec projection is supplied; this does not expose a general process runner.
#[cfg(windows)]
fn run_output<R: Read + Seek + Send, T: Send>(
    command: Command,
    reader: &mut R,
    range: Range,
    cancel: &AtomicBool,
    deadline: Instant,
    mut guard: impl FnMut() -> Result<()>,
    read_output: impl FnOnce(std::process::ChildStdout, &Control<'_>) -> Result<T> + Send,
) -> Result<([u8; 32], T)> {
    use crate::platform_media::process::{ProbeSpawnError, StreamChild};
    let control = Control::new(cancel, deadline);
    control.check()?;
    guard()?;
    let expected = hash_range(reader, range, || control.check())?;
    guard()?;
    control.check()?;
    let owned = StreamChild::spawn_packet_probe(command).map_err(|e| match e {
        ProbeSpawnError::Spawn => Fault::Spawn,
        ProbeSpawnError::Containment => Fault::Containment,
    })?;
    // `child` lives INSIDE the scoped closure: any early return, failed spawn,
    // or panic kills the job before thread::scope automatically joins workers.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        std::thread::scope(|scope| -> Result<([u8; 32], T)> {
            let mut child = owned;
            let input = child.take_stdin().ok_or(Fault::WorkerFailed)?;
            let output = child.take_stdout().ok_or(Fault::WorkerFailed)?;
            let errors = child.take_stderr().ok_or(Fault::WorkerFailed)?;
            let source = &mut *reader;
            let state = &control;
            let stdin = std::thread::Builder::new()
                .spawn_scoped(scope, move || {
                    state.worker(|| transfer(source, input, range, state))
                })
                .map_err(|_| Fault::WorkerFailed)?;
            let stdout = std::thread::Builder::new()
                .spawn_scoped(scope, move || state.worker(|| read_output(output, state)))
                .map_err(|_| Fault::WorkerFailed)?;
            let stderr = std::thread::Builder::new()
                .spawn_scoped(scope, move || state.worker(|| stderr(errors, state)))
                .map_err(|_| Fault::WorkerFailed)?;
            let status = loop {
                if let Err(error) = control.check().and_then(|_| guard()) {
                    control.fail(error);
                    break None;
                }
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() {
                            control.fail(Fault::ChildFailed);
                        }
                        break Some(status);
                    }
                    Ok(None) => std::thread::sleep(POLL),
                    Err(_) => {
                        control.fail(Fault::ChildFailed);
                        break None;
                    }
                }
            };
            // Even a successful parent can leave pipe-holding descendants.
            // Save its status first, terminate the entire job, then join ALL.
            child.terminate();
            let sent = stdin.join().unwrap_or(Err(Fault::WorkerFailed));
            let decoded = stdout.join().unwrap_or(Err(Fault::WorkerFailed));
            let drained = stderr.join().unwrap_or(Err(Fault::WorkerFailed));
            control.check()?;
            if !status.is_some_and(|s| s.success()) {
                return Err(Fault::ChildFailed);
            }
            drained?;
            Ok((sent?, decoded?))
        })
    }))
    .unwrap_or(Err(Fault::WorkerFailed))?;
    control.check()?;
    if result.0 != expected {
        return Err(Fault::InputHash);
    }
    guard()?;
    // Producer is joined: this is again the only user of the held file cursor.
    let after = hash_range(reader, range, || control.check())?;
    if after != expected {
        return Err(Fault::InputHash);
    }
    guard()?;
    control.check()?;
    Ok((expected, result.1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    const ROW: &str = "packet|stream_index=0|pts=1|dts=1|duration=2|size=3|flags=K__|data_hash=SHA256:1111111111111111111111111111111111111111111111111111111111111111\n";
    const STREAM: &str =
        "stream|index=0|codec_name=vp9|codec_type=video|width=854|height=480|time_base=1/1000\n";
    fn control(cancel: &AtomicBool) -> Control<'_> {
        Control::new(cancel, Instant::now() + BUDGET)
    }
    struct Short<R>(R);
    impl<R: Read> Read for Short<R> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let cap = bytes.len().min(3);
            self.0.read(&mut bytes[..cap])
        }
    }
    impl<R: Seek> Seek for Short<R> {
        fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
            self.0.seek(to)
        }
    }
    struct ShortWriter(Vec<u8>);
    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let n = bytes.len().min(2);
            self.0.extend_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::Other.into())
        }
    }
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn metadata_projection_observes_every_stream_without_payload_or_tags() {
        for format in [Format::Vp9Webm, Format::AacMp4] {
            let args = metadata_arguments(format);
            assert!(!args.contains(&"-select_streams"));
            assert!(!args.contains(&"-show_packets"));
            assert_eq!(args.iter().filter(|arg| **arg == "-count_packets").count(), 1);
            assert!(!args.contains(&"-read_intervals"));
            assert!(!args.contains(&"-show_data"));
            assert!(args
                .windows(2)
                .any(|v| v == ["-protocol_whitelist", "pipe"]));
            assert!(args.windows(2).any(|v| v == ["-i", "pipe:0"]));
            assert!(args
                .windows(2)
                .any(|v| v == ["-show_entries", METADATA_ENTRIES]));
            assert!(METADATA_ENTRIES.contains("stream_side_data=side_data_type"));
            assert!(METADATA_ENTRIES.ends_with("stream_tags=:stream_disposition="));
            if format == Format::AacMp4 {
                assert!(args.windows(2).any(|v| v == ["-enable_drefs", "0"]));
                assert!(args.windows(2).any(|v| v == ["-use_absolute_path", "0"]));
            }
        }
    }

    #[test]
    fn metadata_read_is_bounded_and_does_not_turn_io_or_cancel_into_eof() {
        let cancel = AtomicBool::new(false);
        let data = vec![b' '; METADATA_CAP];
        assert_eq!(
            metadata_bytes(Short(Cursor::new(&data)), &control(&cancel)).unwrap(),
            data
        );
        assert_eq!(
            metadata_bytes(Cursor::new(vec![0; METADATA_CAP + 1]), &control(&cancel)).unwrap_err(),
            Fault::OutputLimit
        );
        assert_eq!(
            metadata_bytes(Broken, &control(&cancel)).unwrap_err(),
            Fault::OutputIo
        );
        cancel.store(true, Ordering::Release);
        assert_eq!(
            metadata_bytes(Cursor::new([]), &control(&cancel)).unwrap_err(),
            Fault::Cancelled
        );
    }

    #[test]
    fn fixed_projection_has_only_pipe_and_closed_demuxer_options() {
        let common = [
            "-v",
            "error",
            "-hide_banner",
            "-f",
            "matroska",
            "-protocol_whitelist",
            "pipe",
            "-blocksize",
            "131072",
            "-i",
            "pipe:0",
            "-show_packets",
            "-show_data_hash",
            "sha256",
            "-show_entries",
            ENTRIES,
            "-of",
            PROJECTION,
        ];
        assert_eq!(arguments(Format::Vp9Webm), common);
        let mov = arguments(Format::AacMp4);
        assert_eq!(mov[4], "mov");
        assert_eq!(
            &mov[9..13],
            ["-enable_drefs", "0", "-use_absolute_path", "0"]
        );
        assert_eq!(&mov[13..], &common[9..]);
    }
    #[test]
    fn exact_range_short_reads_and_writes_never_copy_neighbors() {
        let cancel = AtomicBool::new(false);
        let state = control(&cancel);
        let mut source = Short(Cursor::new(b"beforePAYLOADafter"));
        let range = Range::new(6, 13).unwrap();
        let expected: [u8; 32] = Sha256::digest(b"PAYLOAD").into();
        assert_eq!(
            hash_range(&mut source, range, || state.check()).unwrap(),
            expected
        );
        let mut sink = ShortWriter(vec![]);
        assert_eq!(
            transfer(&mut source, &mut sink, range, &state).unwrap(),
            expected
        );
        assert_eq!(sink.0, b"PAYLOAD");
        assert_eq!(source.stream_position().unwrap(), 13);
        assert!(Range::new(0, 0).is_err());
        assert!(Range::new(0, INPUT_CAP + 1).is_err());
    }
    #[test]
    fn physical_eof_and_strict_complete_rows_are_required() {
        let cancel = AtomicBool::new(false);
        let state = control(&cancel);
        let complete = format!("{ROW}{STREAM}");
        let expected = packets(Cursor::new(&complete), Track::Video, &state).unwrap();
        assert_eq!(
            packets(
                Short(Cursor::new(complete.replace('\n', "\r\n"))),
                Track::Video,
                &state
            )
            .unwrap(),
            expected
        );
        assert_eq!(
            packets(Cursor::new(ROW), Track::Video, &state),
            Err(Fault::Packet(packet_facts::Fault::InvalidRow))
        );
        assert_eq!(
            packets(Cursor::new(ROW.trim_end()), Track::Video, &state),
            Err(Fault::Packet(packet_facts::Fault::Truncated))
        );
        assert_eq!(
            packets(Cursor::new(""), Track::Video, &state),
            Err(Fault::Packet(packet_facts::Fault::Empty))
        );
        assert_eq!(
            packets(
                Cursor::new(ROW.replace("stream_index=0", "stream_index=1")),
                Track::Video,
                &state
            ),
            Err(Fault::Packet(packet_facts::Fault::UnknownStream))
        );
        let invalid = ROW.replace("\n", "|side_data_type=Skip Samples\n");
        assert_eq!(
            packets(Cursor::new(invalid), Track::Video, &state),
            Err(Fault::Packet(packet_facts::Fault::UnsupportedSideData))
        );
    }
    #[test]
    fn native_stream_scalars_and_reported_timebase_must_match() {
        let cancel = AtomicBool::new(false);
        let state = control(&cancel);
        let video = packets(Cursor::new(format!("{ROW}{STREAM}")), Track::Video, &state)
            .unwrap()
            .stream;
        let mut configuration = Configuration {
            format: Format::Vp9Webm,
            track_id: 1,
            timebase: 1_000_000,
            width: 854,
            height: 480,
            default_duration_ns: Some(33_333_333),
            sample_rate: 0,
            channels: 0,
            semantic_sha256: [7; 32],
        };
        assert_eq!(
            validate_stream(Track::Video, &configuration, &video),
            Ok(())
        );
        configuration.width += 1;
        assert_eq!(
            validate_stream(Track::Video, &configuration, &video),
            Err(Fault::UnsupportedInput)
        );
        configuration.width -= 1;
        configuration.timebase += 1;
        assert_eq!(
            validate_stream(Track::Video, &configuration, &video),
            Err(Fault::UnsupportedInput)
        );
        configuration.timebase = u64::MAX;
        assert_eq!(
            validate_stream(Track::Video, &configuration, &video),
            Err(Fault::UnsupportedInput)
        );
        assert_eq!(
            validate_stream(Track::Audio, &configuration, &video),
            Err(Fault::UnsupportedInput)
        );
        let audio = packets(Cursor::new(format!("{ROW}stream|index=0|codec_name=aac|codec_type=audio|sample_rate=44100|channels=2|time_base=1/44100\n")), Track::Audio, &state).unwrap().stream;
        configuration.format = Format::AacMp4;
        configuration.timebase = 44_100;
        configuration.sample_rate = 44_100;
        configuration.channels = 2;
        assert_eq!(
            validate_stream(Track::Audio, &configuration, &audio),
            Ok(())
        );
        configuration.sample_rate = 48_000;
        assert_eq!(
            validate_stream(Track::Audio, &configuration, &audio),
            Err(Fault::UnsupportedInput)
        );
        configuration.sample_rate = 44_100;
        configuration.channels = 1;
        assert_eq!(
            validate_stream(Track::Audio, &configuration, &audio),
            Err(Fault::UnsupportedInput)
        );
        configuration.channels = 2;
        configuration.timebase = 48_000;
        assert_eq!(
            validate_stream(Track::Audio, &configuration, &audio),
            Err(Fault::UnsupportedInput)
        );
        // No claim that opaque native decoder hashes equal projected metadata.
        assert_eq!(configuration.semantic_sha256, [7; 32]);
    }
    #[test]
    fn input_short_broken_pipe_and_output_io_never_produce_evidence() {
        let cancel = AtomicBool::new(false);
        let state = control(&cancel);
        let range = Range::new(0, 2).unwrap();
        assert_eq!(
            transfer(&mut Cursor::new(b"x"), io::sink(), range, &state),
            Err(Fault::InputShort)
        );
        assert_eq!(
            hash_range(&mut Cursor::new(b"x"), range, || state.check()),
            Err(Fault::InputShort)
        );
        assert_eq!(
            transfer(&mut Cursor::new(b"xx"), Broken, range, &state),
            Err(Fault::InputIo)
        );
        assert_eq!(packets(Broken, Track::Video, &state), Err(Fault::OutputIo));
        assert_eq!(stderr(Broken, &state), Err(Fault::OutputIo));
    }
    #[test]
    fn stderr_is_counted_not_retained_and_empty_only_succeeds() {
        let cancel = AtomicBool::new(false);
        let state = control(&cancel);
        assert_eq!(stderr(Cursor::new([]), &state), Ok(()));
        assert_eq!(
            stderr(Cursor::new(b"fixed synthetic diagnostic"), &state),
            Err(Fault::StderrOutput)
        );
        assert_eq!(
            stderr(io::repeat(1).take(STDERR_CAP as u64), &state),
            Err(Fault::StderrOutput)
        );
        assert_eq!(
            stderr(io::repeat(1).take(STDERR_CAP as u64 + 1), &state),
            Err(Fault::StderrLimit)
        );
    }
    #[test]
    fn cancelled_deadline_and_worker_panic_latch_without_raw_error() {
        let cancel = AtomicBool::new(true);
        let state = control(&cancel);
        assert_eq!(state.check(), Err(Fault::Cancelled));
        assert_eq!(state.fail(Fault::InputIo), Fault::Cancelled);
        cancel.store(false, Ordering::Release);
        assert_eq!(state.check(), Err(Fault::Cancelled));
        assert_eq!(
            Control::new(&cancel, Instant::now()).check(),
            Err(Fault::Deadline)
        );
        let state = control(&cancel);
        assert_eq!(
            state.worker::<()>(|| panic!("synthetic worker")),
            Err(Fault::WorkerFailed)
        );
        assert_eq!(state.check(), Err(Fault::WorkerFailed));
    }

    #[test]
    fn command_environment_is_explicit_and_does_not_mutate_global_state() {
        use std::{ffi::OsStr, path::Path};
        let command = configured_command(
            Path::new("fixed-test-only-ffprobe"),
            Path::new("fixed-test-only-bin"),
            [
                ("SystemRoot", OsStr::new("fixed-windows")),
                ("WINDIR", OsStr::new("fixed-windows")),
            ],
            Format::Vp9Webm,
        );
        let env: Vec<_> = command.get_envs().collect();
        assert_eq!(env.len(), 2);
        assert!(env
            .iter()
            .all(|(k, v)| matches!(k.to_str(), Some("SystemRoot" | "WINDIR"))
                && *v == Some(OsStr::new("fixed-windows"))));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            arguments(Format::Vp9Webm)
                .iter()
                .map(OsStr::new)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            command.get_current_dir(),
            Some(Path::new("fixed-test-only-bin"))
        );
        // Inherited environment clearing is performed unconditionally by the
        // same builder; this test does not set global FFREPORT/proxy variables.
    }

    #[test]
    fn generated_input_stays_bounded_and_mid_write_cancel_is_terminal() {
        struct Generated {
            at: u64,
            end: u64,
            maximum: usize,
        }
        impl Read for Generated {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                assert!(bytes.len() <= CHUNK);
                self.maximum = self.maximum.max(bytes.len());
                let count = ((self.end - self.at) as usize).min(bytes.len());
                bytes[..count].fill(7);
                self.at += count as u64;
                Ok(count)
            }
        }
        impl Seek for Generated {
            fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
                match to {
                    SeekFrom::Start(at) if at <= self.end => {
                        self.at = at;
                        Ok(at)
                    }
                    _ => Err(io::ErrorKind::InvalidInput.into()),
                }
            }
        }
        let cancel = AtomicBool::new(false);
        let state = control(&cancel);
        let range = Range::new(0, 8 * 1024 * 1024 + 1).unwrap();
        let mut source = Generated {
            at: 0,
            end: range.end,
            maximum: 0,
        };
        let expected = hash_range(&mut source, range, || state.check()).unwrap();
        assert_eq!(
            transfer(&mut source, io::sink(), range, &state).unwrap(),
            expected
        );
        assert_eq!(source.maximum, CHUNK);
        struct Cancels<'a>(&'a AtomicBool);
        impl Write for Cancels<'_> {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                self.0.store(true, Ordering::Release);
                Ok(1)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(
            transfer(&mut source, Cancels(&cancel), range, &state),
            Err(Fault::Cancelled)
        );
    }

    #[cfg(windows)]
    fn os_fixture(script: &'static str) -> Command {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
        use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
        let mut buffer = [0u16; 32_768];
        let count = unsafe { GetWindowsDirectoryW(Some(&mut buffer)) } as usize;
        assert!((1..buffer.len()).contains(&count));
        let directory = PathBuf::from(OsString::from_wide(&buffer[..count]));
        let executable = directory.join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut command = Command::new(executable);
        command
            .env_clear()
            .env("SystemRoot", &directory)
            .env("WINDIR", &directory)
            .current_dir(&directory)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ]);
        command
    }
    #[cfg(windows)]
    fn fixture_run(
        command: Command,
        cancel: &AtomicBool,
        budget: Duration,
        guard: impl FnMut() -> Result<()>,
    ) -> Result<RawEvidence> {
        run(
            command,
            &mut Cursor::new([1]),
            Range::new(0, 1).unwrap(),
            Track::Video,
            cancel,
            Instant::now() + budget,
            guard,
        )
    }

    #[cfg(windows)]
    #[test]
    fn contained_errors_and_parser_failure_do_not_wait_for_sleeping_child() {
        let cancel = AtomicBool::new(false);
        let start = Instant::now();
        let invalid = os_fixture("$o=[Console]::OpenStandardOutput();$o.Write([byte[]](120,10),0,2);$o.Flush();[Threading.Thread]::Sleep(30000)");
        assert_eq!(
            fixture_run(invalid, &cancel, Duration::from_secs(10), || Ok(())).unwrap_err(),
            Fault::Packet(packet_facts::Fault::InvalidRow)
        );
        assert!(start.elapsed() < Duration::from_secs(10));
        // libtest's preamble is never filtered or treated as a valid packet.
        let fail = fixture_run(
            os_fixture("exit 7"),
            &cancel,
            Duration::from_secs(10),
            || Ok(()),
        );
        assert!(matches!(
            fail,
            Err(Fault::ChildFailed | Fault::Packet(packet_facts::Fault::Empty))
        ));
    }
    #[cfg(windows)]
    #[test]
    fn blocked_stdin_deadline_and_watchdog_cancel_terminate_before_join() {
        let cancel = AtomicBool::new(false);
        let start = Instant::now();
        let result = run(
            os_fixture("[Threading.Thread]::Sleep(30000)"),
            &mut Cursor::new(vec![0; 4 * CHUNK]),
            Range::new(0, (4 * CHUNK) as u64).unwrap(),
            Track::Video,
            &cancel,
            Instant::now() + Duration::from_millis(600),
            || Ok(()),
        );
        assert_eq!(result.unwrap_err(), Fault::Deadline);
        assert!(start.elapsed() < Duration::from_secs(10));
        let mut calls = 0;
        let result = fixture_run(
            os_fixture("[Threading.Thread]::Sleep(30000)"),
            &cancel,
            Duration::from_secs(10),
            || {
                calls += 1;
                if calls >= 3 {
                    cancel.store(true, Ordering::Release);
                }
                checkpoint(&cancel, Instant::now() + BUDGET)
            },
        );
        assert_eq!(result.unwrap_err(), Fault::Cancelled);
    }
    #[cfg(windows)]
    #[test]
    fn guard_panic_kills_owned_process_before_automatic_scope_join() {
        let cancel = AtomicBool::new(false);
        let mut calls = 0;
        let start = Instant::now();
        let result = fixture_run(
            os_fixture("[Threading.Thread]::Sleep(30000)"),
            &cancel,
            Duration::from_secs(10),
            || {
                calls += 1;
                assert!(calls < 3, "synthetic watchdog panic after worker start");
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err(), Fault::WorkerFailed);
        assert!(start.elapsed() < Duration::from_secs(10));
    }
    #[cfg(windows)]
    #[test]
    fn parent_exit_cannot_leave_descendant_pipe_workers_waiting() {
        let cancel = AtomicBool::new(false);
        let start = Instant::now();
        // Constant OS fixture only: no caller text, network, or filesystem output.
        // The descendant inherits the parent's redirected handles and sleeps.
        let command = os_fixture("$p=New-Object Diagnostics.ProcessStartInfo;$p.FileName=(Join-Path $PSHOME 'powershell.exe');$p.Arguments='-NoLogo -NoProfile -NonInteractive -Command [Threading.Thread]::Sleep(30000)';$p.UseShellExecute=$false;$p.CreateNoWindow=$true;$null=[Diagnostics.Process]::Start($p);exit 0");
        let result = fixture_run(command, &cancel, Duration::from_secs(10), || Ok(()));
        assert_eq!(
            result.unwrap_err(),
            Fault::Packet(packet_facts::Fault::Empty)
        );
        assert!(start.elapsed() < Duration::from_secs(10));
        // This checks bounded return after parent exit, not a PID-based proof
        // of descendant liveness. That independent assertion remains unverified.
    }

    #[cfg(windows)]
    #[test]
    fn callback_failure_terminates_sleeping_child_and_never_returns_evidence() {
        let cancel = AtomicBool::new(false);
        let start = Instant::now();
        // Fixed OS fixture. The consumer rejects a valid row while the child
        // keeps stdout open; no auth, user data or filesystem output is used.
        let command = os_fixture("[Console]::WriteLine('packet|stream_index=0|pts=1|dts=1|duration=2|size=3|flags=K__|data_hash=SHA256:1111111111111111111111111111111111111111111111111111111111111111');[Console]::Out.Flush();[Threading.Thread]::Sleep(30000)");
        let result = run_with_callback(
            command,
            &mut Cursor::new([1]),
            Range::new(0, 1).unwrap(),
            Track::Video,
            0,
            &cancel,
            Instant::now() + Duration::from_secs(10),
            || Ok(()),
            |_| Err(packet_facts::Fault::Sink),
        );
        assert_eq!(
            result.unwrap_err(),
            Fault::Packet(packet_facts::Fault::Sink)
        );
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[cfg(windows)]
    #[test]
    fn oversized_metadata_terminates_owned_child_before_worker_join() {
        let cancel = AtomicBool::new(false);
        let start = Instant::now();
        let command = os_fixture("[Console]::Write(('x' * 65537));[Console]::Out.Flush();[Threading.Thread]::Sleep(30000)");
        let result = run_output(
            command,
            &mut Cursor::new([1]),
            Range::new(0, 1).unwrap(),
            &cancel,
            Instant::now() + Duration::from_secs(10),
            || Ok(()),
            |output, state| metadata_bytes(output, state),
        );
        assert_eq!(result.unwrap_err(), Fault::OutputLimit);
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "root-only immutable fixed pair/candidate; codec observations, not production admission"]
    fn actual_fixed_pair_has_exact_supported_whole_stream_metadata() {
        use super::super::{
            files::{Directories, GuardedFile},
            index,
        };
        use crate::platform_media::PlatformMediaTools;
        use std::path::{Path, PathBuf};
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let pair =
            root.join(".runtime/youtube-webm-raw-pair-qa/0ab34128-d5cf-43b2-8fad-2d2fe9cc89c2");
        let bin = root
            .join(".runtime/media-tools")
            .join(managed_probe::PACKAGE)
            .join("bin");
        let tools = PlatformMediaTools {
            yt_dlp: PathBuf::new(),
            deno: PathBuf::new(),
            ffmpeg: bin.join("ffmpeg.exe"),
            ffprobe: bin.join("ffprobe.exe"),
        };
        let cancel = AtomicBool::new(false);
        let tool = ManagedProbe::open(&tools, &cancel, Instant::now() + BUDGET).unwrap();
        let directories = Directories::hold(&pair).unwrap();
        let inspect = |name: &str, length, format, expected: &str, raw: bool| {
            let held = GuardedFile::open(&pair.join(name), false).unwrap();
            assert_eq!(held.check().unwrap(), length);
            let range = Range::new(0, length).unwrap();
            let deadline = Instant::now() + METADATA_BUDGET;
            let mut reader = &held.file;
            let before = hash_range(&mut reader, range, || checkpoint(&cancel, deadline)).unwrap();
            assert_eq!(
                before
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                expected
            );
            let mut configuration = None;
            if raw {
                let outcome = index::scan(
                    format,
                    &mut reader,
                    length,
                    if format == Format::Vp9Webm {
                        1_000_000
                    } else {
                        44_100
                    },
                    &cancel,
                    &mut |unit| {
                        assert!(configuration.is_none());
                        configuration = Some(unit.facts.configuration);
                        Ok(())
                    },
                );
                assert_eq!(outcome.terminal, index::Terminal::Complete);
                assert_eq!(outcome.complete_units, 1);
            }
            let command = if raw {
                metadata_command(&tool, format)
            } else {
                // Only this exact hash-held fixture, not a candidate-path API.
                let mut command = Command::new(tool.executable());
                command
                    .current_dir(tool.cwd())
                    .env_clear()
                    .envs(tool.environment())
                    .args([
                        "-v",
                        "error",
                        "-hide_banner",
                        "-f",
                        "mov",
                        "-protocol_whitelist",
                        "file",
                        "-enable_drefs",
                        "0",
                        "-use_absolute_path",
                        "0",
                    ])
                    .arg("-i")
                    .arg(pair.join(name))
                    .args([
                        "-show_data_hash",
                        "sha256",
                        "-show_entries",
                        METADATA_ENTRIES,
                        "-of",
                        "json",
                    ]);
                command
            };
            let (_, bytes) = run_output(
                command,
                &mut reader,
                if raw {
                    range
                } else {
                    Range { start: 0, end: 0 }
                },
                &cancel,
                deadline,
                || {
                    directories.check().map_err(source_error)?;
                    if held.check().map_err(source_error)? != length {
                        return Err(Fault::SourceChanged);
                    }
                    tool.check(&cancel, deadline).map_err(tool_error)
                },
                |output, state| metadata_bytes(output, state),
            )
            .unwrap();
            assert_eq!(
                hash_range(&mut reader, range, || checkpoint(&cancel, deadline)).unwrap(),
                before
            );
            assert_eq!(held.check().unwrap(), length);
            directories.check().unwrap();
            tool.check(&cancel, deadline).unwrap();
            (bytes, configuration)
        };
        let (video, configuration) = inspect(
            "unit-1.webm",
            124392,
            Format::Vp9Webm,
            "0c4a795839e97419442f0a03d734e11c98f5381b3be4a121ecb0ab47a837119f",
            true,
        );
        let video = codec_facts::source(&video, &configuration.unwrap()).unwrap();
        let (audio, configuration) = inspect(
            "unit-2.mp4",
            82189,
            Format::AacMp4,
            "5baa9ceeebe7a370079c3f639951679983e5efede39ae0e9cf5f81940c6c75a5",
            true,
        );
        let audio = codec_facts::source(&audio, &configuration.unwrap()).unwrap();
        let (candidate, _) = inspect(
            "candidate-common-origin.mp4",
            209643,
            Format::AacMp4,
            "ef625424d091e5994cbc141e7969a29af98a0d1243d189d6bb1c452a2648e2ea",
            false,
        );
        let candidate = codec_facts::candidate(&candidate, &video, &audio).unwrap();
        assert_eq!(
            candidate.video_time_base(),
            TimeBase::new(1, 16_000).unwrap()
        );
        assert_eq!(
            candidate.audio_time_base(),
            TimeBase::new(1, 44_100).unwrap()
        );
        assert_eq!(video.time_base(), TimeBase::new(1, 1_000).unwrap());
        assert_eq!(audio.time_base(), TimeBase::new(1, 44_100).unwrap());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "root-only immutable fixed candidates; all-packet comparison, not production admission"]
    fn actual_fixed_candidates_preserve_every_packet_and_reject_skip_change() {
        use super::super::{
            files::{Directories, GuardedFile},
            packet_facts::{compare_iters, PairComparison, Rational},
            packet_tape,
        };
        use crate::platform_media::PlatformMediaTools;
        use std::path::{Path, PathBuf};

        // Test-only bounded typed records. No transcript, media copy or disk tape.
        struct Records(Vec<u8>);
        impl Write for Records {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                let remaining = (8 + 512 * 80_usize).saturating_sub(self.0.len());
                if bytes.len() > remaining {
                    return Err(std::io::Error::other("fixture record bound"));
                }
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let pair =
            root.join(".runtime/youtube-webm-raw-pair-qa/0ab34128-d5cf-43b2-8fad-2d2fe9cc89c2");
        let bin = root
            .join(".runtime/media-tools")
            .join(managed_probe::PACKAGE)
            .join("bin");
        let tools = PlatformMediaTools {
            yt_dlp: PathBuf::new(),
            deno: PathBuf::new(),
            ffmpeg: bin.join("ffmpeg.exe"),
            ffprobe: bin.join("ffprobe.exe"),
        };
        let cancel = AtomicBool::new(false);
        let tool = ManagedProbe::open(&tools, &cancel, Instant::now() + BUDGET).unwrap();
        let directories = Directories::hold(&pair).unwrap();
        let inspect = |name: &str, length, format, track, index, expected: &str, piped| {
            let held = GuardedFile::open(&pair.join(name), false).unwrap();
            assert_eq!(held.check().unwrap(), length);
            let range = Range::new(0, length).unwrap();
            let deadline = Instant::now() + BUDGET;
            let mut reader = &held.file;
            let before = hash_range(&mut reader, range, || checkpoint(&cancel, deadline)).unwrap();
            assert_eq!(
                before
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                expected
            );
            let command = if piped {
                command(&tool, format)
            } else {
                // Only the fixed hash-held MOV candidates above, never user paths.
                let mut command = Command::new(tool.executable());
                command
                    .current_dir(tool.cwd())
                    .env_clear()
                    .envs(tool.environment())
                    .args([
                        "-v",
                        "error",
                        "-hide_banner",
                        "-f",
                        "mov",
                        "-protocol_whitelist",
                        "file",
                        "-enable_drefs",
                        "0",
                        "-use_absolute_path",
                        "0",
                    ])
                    .arg("-i")
                    .arg(pair.join(name))
                    .args([
                        "-select_streams",
                        if track == Track::Video { "v:0" } else { "a:0" },
                        "-show_packets",
                        "-show_data_hash",
                        "sha256",
                        "-show_entries",
                        ENTRIES,
                        "-of",
                        PROJECTION,
                    ]);
                command
            };
            let mut records = Records(Vec::new());
            let mut writer = packet_tape::Writer::new(&mut records, track).unwrap();
            let evidence = run_with_callback(
                command,
                &mut reader,
                if piped {
                    range
                } else {
                    Range { start: 0, end: 0 }
                },
                track,
                index,
                &cancel,
                deadline,
                || {
                    directories.check().map_err(source_error)?;
                    if held.check().map_err(source_error)? != length {
                        return Err(Fault::SourceChanged);
                    }
                    tool.check(&cancel, deadline).map_err(tool_error)
                },
                |packet| writer.push(packet).map_err(|_| packet_facts::Fault::Sink),
            )
            .unwrap();
            assert_eq!(
                hash_range(&mut reader, range, || checkpoint(&cancel, deadline)).unwrap(),
                before
            );
            assert_eq!(held.check().unwrap(), length);
            directories.check().unwrap();
            tool.check(&cancel, deadline).unwrap();
            if piped {
                assert_eq!(evidence.hash, before);
            }
            let summary = writer
                .finish(&ProbeFacts {
                    packets: evidence.packets,
                    stream: evidence.stream,
                })
                .unwrap();
            drop(writer);
            (records.0, summary)
        };
        let video = inspect(
            "unit-1.webm",
            124392,
            Format::Vp9Webm,
            Track::Video,
            0,
            "0c4a795839e97419442f0a03d734e11c98f5381b3be4a121ecb0ab47a837119f",
            true,
        );
        let audio = inspect(
            "unit-2.mp4",
            82189,
            Format::AacMp4,
            Track::Audio,
            0,
            "5baa9ceeebe7a370079c3f639951679983e5efede39ae0e9cf5f81940c6c75a5",
            true,
        );
        let candidate_hash = "ef625424d091e5994cbc141e7969a29af98a0d1243d189d6bb1c452a2648e2ea";
        let output_video = inspect(
            "candidate-common-origin.mp4",
            209643,
            Format::Vp9Webm,
            Track::Video,
            0,
            candidate_hash,
            false,
        );
        let output_audio = inspect(
            "candidate-common-origin.mp4",
            209643,
            Format::AacMp4,
            Track::Audio,
            1,
            candidate_hash,
            false,
        );
        let altered_audio = inspect(
            "candidate.mp4",
            209631,
            Format::AacMp4,
            Track::Audio,
            1,
            "21fcdacf1b5e7f6dc2789c55f8cc2e9de8fc35150cd7983c096f317aa8e6a71f",
            false,
        );
        let compare = |track,
                       source: &(Vec<u8>, packet_tape::Summary),
                       output: &(Vec<u8>, packet_tape::Summary)| {
            let mut a = &source.0[..];
            let mut b = &output.0[..];
            compare_iters(
                PairComparison::new(
                    track,
                    source.1.facts().stream.time_base,
                    output.1.facts().stream.time_base,
                    Rational::new(2063879393, 44100).unwrap(),
                )
                .unwrap(),
                packet_tape::Reader::new(&mut a, &source.1).unwrap(),
                packet_tape::Reader::new(&mut b, &output.1).unwrap(),
            )
        };
        assert_eq!(
            video.1.facts().stream.time_base,
            TimeBase::new(1, 1000).unwrap()
        );
        assert_eq!(
            output_video.1.facts().stream.time_base,
            TimeBase::new(1, 16000).unwrap()
        );
        for item in [&video, &output_video] {
            assert_eq!(
                (item.1.facts().stream.width, item.1.facts().stream.height),
                (854, 480)
            );
        }
        for item in [&audio, &output_audio, &altered_audio] {
            assert_eq!(
                item.1.facts().stream.time_base,
                TimeBase::new(1, 44100).unwrap()
            );
            assert_eq!(
                (
                    item.1.facts().stream.sample_rate,
                    item.1.facts().stream.channels
                ),
                (44100, 2)
            );
        }
        let v = compare(Track::Video, &video, &output_video).unwrap();
        assert_eq!(v.count, 150);
        assert_eq!(v.max_pts_error, Rational::new(26, 110250).unwrap());
        assert_eq!(v.max_dts_error, Rational::new(26, 110250).unwrap());
        assert_eq!(v.max_duration_error, Rational::new(11, 16000).unwrap());
        let a = compare(Track::Audio, &audio, &output_audio).unwrap();
        assert_eq!(a.count, 216);
        assert_eq!(a.max_pts_error, Rational::new(0, 1).unwrap());
        assert_eq!(a.max_dts_error, Rational::new(0, 1).unwrap());
        assert_eq!(a.max_duration_error, Rational::new(0, 1).unwrap());
        assert_eq!(
            compare(Track::Audio, &audio, &altered_audio),
            Err(packet_facts::Fault::SideDataMismatch)
        );
        // Selected tracks only; no whole-file codec/admission/publication claim.
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "root-only immutable fixed pair; pipe/file packet evidence, not media verification"]
    fn actual_fixed_pair_pipe_matches_guarded_file_reference() {
        use super::super::files::{Directories, GuardedFile};
        use crate::platform_media::PlatformMediaTools;
        use std::path::{Path, PathBuf};
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let pair =
            root.join(".runtime/youtube-webm-raw-pair-qa/0ab34128-d5cf-43b2-8fad-2d2fe9cc89c2");
        let bin = root
            .join(".runtime/media-tools")
            .join(managed_probe::PACKAGE)
            .join("bin");
        let tools = PlatformMediaTools {
            yt_dlp: PathBuf::new(),
            deno: PathBuf::new(),
            ffmpeg: bin.join("ffmpeg.exe"),
            ffprobe: bin.join("ffprobe.exe"),
        };
        let cancel = AtomicBool::new(false);
        let tool = ManagedProbe::open(&tools, &cancel, Instant::now() + BUDGET).unwrap();
        let directories = Directories::hold(&pair).unwrap();
        let fixtures = [
            (
                "unit-1.webm",
                124392,
                Format::Vp9Webm,
                Track::Video,
                150,
                "0c4a795839e97419442f0a03d734e11c98f5381b3be4a121ecb0ab47a837119f",
            ),
            (
                "unit-2.mp4",
                82189,
                Format::AacMp4,
                Track::Audio,
                216,
                "5baa9ceeebe7a370079c3f639951679983e5efede39ae0e9cf5f81940c6c75a5",
            ),
        ];
        for (name, length, format, track, count, expected) in fixtures {
            let held = GuardedFile::open(&pair.join(name), false).unwrap();
            assert_eq!(held.check().unwrap(), length);
            let range = Range::new(0, length).unwrap();
            let deadline = Instant::now() + BUDGET;
            let mut reader = &held.file;
            let before = hash_range(&mut reader, range, || checkpoint(&cancel, deadline)).unwrap();
            let hex = |hash: [u8; 32]| hash.iter().map(|b| format!("{b:02x}")).collect::<String>();
            assert_eq!(hex(before), expected);
            let result = run(
                command(&tool, format),
                &mut reader,
                range,
                track,
                &cancel,
                deadline,
                || {
                    directories.check().map_err(source_error)?;
                    if held.check().map_err(source_error)? != length {
                        return Err(Fault::SourceChanged);
                    }
                    tool.check(&cancel, deadline).map_err(tool_error)
                },
            )
            .unwrap();
            assert_eq!(result.hash, before);
            assert_eq!(result.packets.count, count);
            assert_eq!(result.packets.track, track);
            assert_eq!(result.stream.track, track);
            match track {
                Track::Video => {
                    assert_eq!(result.stream.time_base, TimeBase::new(1, 1000).unwrap());
                    assert_eq!((result.stream.width, result.stream.height), (854, 480));
                    assert_eq!((result.stream.sample_rate, result.stream.channels), (0, 0));
                }
                Track::Audio => {
                    assert_eq!(result.stream.time_base, TimeBase::new(1, 44_100).unwrap());
                    assert_eq!(
                        (result.stream.sample_rate, result.stream.channels),
                        (44_100, 2)
                    );
                    assert_eq!((result.stream.width, result.stream.height), (0, 0));
                }
            }
            // Independent invocation, NOT a pipe-derived expected digest or a
            // production fallback. Only these two fixed hash-held paths exist.
            let mut reference = Command::new(tool.executable());
            reference
                .current_dir(tool.cwd())
                .env_clear()
                .envs(tool.environment())
                .args([
                    "-v",
                    "error",
                    "-hide_banner",
                    "-f",
                    match format {
                        Format::Vp9Webm => "matroska",
                        Format::AacMp4 => "mov",
                    },
                    "-protocol_whitelist",
                    "file",
                ]);
            if format == Format::AacMp4 {
                reference.args(["-enable_drefs", "0", "-use_absolute_path", "0"]);
            }
            reference.arg("-i").arg(pair.join(name)).args([
                "-show_packets",
                "-show_data_hash",
                "sha256",
                "-show_entries",
                ENTRIES,
                "-of",
                PROJECTION,
            ]);
            let reference_deadline = Instant::now() + BUDGET;
            // Zero is test-local only; production Range::new rejects it. No input
            // is fed, so this is file demuxing through the identical contained
            // lifecycle/strict decoder, not a second media copy.
            let comparison = run(
                reference,
                &mut reader,
                Range { start: 0, end: 0 },
                track,
                &cancel,
                reference_deadline,
                || {
                    directories.check().map_err(source_error)?;
                    if held.check().map_err(source_error)? != length {
                        return Err(Fault::SourceChanged);
                    }
                    tool.check(&cancel, reference_deadline).map_err(tool_error)
                },
            )
            .unwrap();
            assert_eq!(result.packets, comparison.packets);
            assert_eq!(result.stream, comparison.stream);
            assert_eq!(
                hash_range(&mut reader, range, || checkpoint(
                    &cancel,
                    reference_deadline
                ))
                .unwrap(),
                before
            );
            assert_eq!(held.check().unwrap(), length);
            directories.check().unwrap();
        }
        // Standalone fixed sources do not manufacture SelectedUnit proof/journal.
        // Fold equality covers ordered size/payload/key/side-data and recorded
        // time extrema, NOT all per-packet clocks or a known tool time base.
        // Production integration/remux/decode/publication remain unverified.
    }
}
