//! Root-only immutable source diagnostic; never creates production evidence.
use super::*;
use super::super::{
    files::{Directories, GuardedFile},
    production,
    recovery::selection::CompletedSelection,
    Error,
};
use crate::platform_media::PlatformMediaTools;
use std::{fs::File, io, path::{Path, PathBuf}};

/// No buffering or altered read sizes: only a fixed error/EOF discriminant.
struct ObservedReader<'a> {
    file: &'a File,
    io_failed: bool,
    premature_eof: bool,
}
impl Read for ObservedReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let result = self.file.read(buffer);
        self.io_failed |= result.is_err();
        self.premature_eof |= !buffer.is_empty() && matches!(&result, Ok(0));
        result
    }
}
impl Seek for ObservedReader<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let result = self.file.seek(position);
        self.io_failed |= result.is_err();
        result
    }
}

fn pinned(file: &GuardedFile, length: u64, expected: &str) -> bool {
    if length == 0 || length > 4 * 1024 * 1024 || file.check().ok() != Some(length) {
        return false;
    }
    let mut reader = &file.file;
    let Ok(saved) = reader.stream_position() else { return false; };
    let result = (|| -> io::Result<bool> {
        reader.seek(SeekFrom::Start(0))?;
        let mut hash = Sha256::new();
        let mut scratch = [0u8; CHUNK];
        let mut remaining = length;
        while remaining != 0 {
            let size = remaining.min(CHUNK as u64) as usize;
            reader.read_exact(&mut scratch[..size])?;
            hash.update(&scratch[..size]);
            remaining -= size as u64;
        }
        Ok(reader.read(&mut scratch[..1])? == 0 && format!("{:x}", hash.finalize()) == expected)
    })();
    let restored = reader.seek(SeekFrom::Start(saved)).is_ok();
    result.unwrap_or(false) && restored && file.check().ok() == Some(length)
}

#[test]
#[ignore = "Root-only fixed actual raw source; sequential contained metadata reads, no media/file writes"]
fn actual_raw_metadata_pipe_current_vs_count_packets_read_only() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let root = workspace.join(".runtime/youtube-auto-record-qa/local.atsumi.youtubeautoqa.aeb71d4637694e9bb3aa16f2ce0605fb/media");
    let job = "3e5fee79-6417-4c82-9e85-295a72a97675";
    let directory = root.join("YouTube").join(job);
    let specs = [
        ("proof.json", 942u64, "4c56a1470d38dc13a2b7c235915069f94114117357346132369e89a949251c07"),
        ("journal.jsonl", 89029, "2c0fbff2566fe6281d861dd91e9c87f744f0d49ae7afc90d8845d2ddc17a561d"),
        ("video.raw", 1380681, "3bb0b865a31c196591e8f6f1055006059fa1a26a00817e4b3eb9a96dbe391919"),
        ("audio.raw", 573806, "74430cceecb478bbf2a5d6e39a85d96cfb809a16a2a8199438b132de671f208a"),
    ];
    let directories = Directories::hold(&directory).unwrap();
    let held: Vec<_> = specs.iter().map(|(name, length, digest)| {
        let file = GuardedFile::open(&directory.join(name), false).unwrap();
        assert!(pinned(&file, *length, digest), "fixed input pin mismatch");
        file
    }).collect();
    let cancel = AtomicBool::new(false);
    let overall_deadline = Instant::now() + Duration::from_secs(240);
    let identity = production::recovery_identity(&root, job, &cancel).unwrap();
    let mut selected = CompletedSelection::prepare(&root, &identity, &cancel).unwrap();
    let bin = workspace.join(".runtime/media-tools").join(managed_probe::PACKAGE).join("bin");
    let tools = PlatformMediaTools {
        yt_dlp: PathBuf::new(), deno: PathBuf::new(),
        ffmpeg: bin.join("ffmpeg.exe"), ffprobe: bin.join("ffprobe.exe"),
    };
    let tool = ManagedProbe::open(&tools, &cancel, overall_deadline).unwrap();
    let mut rows = Vec::new();
    let mut counts = [0usize; 2];
    let mut current_input_io = 0usize;
    let mut counted_successes = 0usize;
    let mut unexpected = None;
    let mut visit_failed = false;
    for track in [Track::Video, Track::Audio] {
        let visit = selected.visit(track, &cancel, |mut span| {
            if counts[track.index()] >= 16 {
                unexpected = Some(Fault::SourceInvalid);
                return Err(Error::Bounds);
            }
            counts[track.index()] += 1;
            let source = match SourceFacts::copy(span.facts()) {
                Ok(value) => value,
                Err(error) => { unexpected = Some(error); return Err(Error::Corrupt); }
            };
            let range = Range::new(source.unit.facts.start, source.unit.facts.end)
                .map_err(|_| Error::Bounds)?;
            let file = span.take_reader()?;
            let mut current_success = None;
            for counted in [false, true] {
                let deadline = overall_deadline.min(Instant::now() + METADATA_BUDGET);
                let mut reader = ObservedReader { file, io_failed: false, premature_eof: false };
                let format = source.unit.facts.configuration.format;
                let command = if counted {
                    metadata_command(&tool, format)
                } else {
                    // Preserve the pre-fix control for this immutable diagnostic.
                    // Production always drains input; this branch is test-only.
                    let mut legacy = Command::new(tool.executable());
                    legacy.current_dir(tool.cwd()).env_clear().envs(tool.environment())
                        .args(metadata_arguments(format).into_iter().filter(|arg| *arg != "-count_packets"));
                    legacy
                };
                let result = run_output(command, &mut reader, range, &cancel, deadline, || {
                    checkpoint(&cancel, deadline)?;
                    directories.check().map_err(source_error)?;
                    span.check().map_err(source_error)?;
                    tool.check(&cancel, deadline).map_err(tool_error)
                }, metadata_bytes).and_then(|(hash, bytes)| {
                    let codec = codec_facts::source(&bytes, &source.unit.facts.configuration)
                        .map_err(Fault::Codec)?;
                    span.check().map_err(source_error)?;
                    tool.check(&cancel, deadline).map_err(tool_error)?;
                    checkpoint(&cancel, deadline)?;
                    Ok((hash, codec))
                });
                let source_ok = !reader.io_failed && !reader.premature_eof;
                let result_label = match &result {
                    Ok(_) => "success",
                    Err(Fault::InputIo) if source_ok => "input_io_with_source_io_ok",
                    Err(Fault::InputIo) => "input_io_with_source_io_error",
                    Err(_) => "unexpected_failure",
                };
                rows.push(serde_json::json!({
                    "mode":if counted { "production_count_packets" } else { "legacy_metadata_only" },
                    "track":if track == Track::Video { "video" } else { "audio" },
                    "ordinal":source.unit.ordinal,"bytes":range.len(),
                    "result":result_label,"sourceIoFailed":reader.io_failed,
                    "prematureEof":reader.premature_eof
                }));
                match result {
                    Ok(value) if counted => {
                        counted_successes += 1;
                        if !source_ok || current_success.as_ref().is_some_and(|old| old != &value) {
                            unexpected = Some(Fault::SourceChanged);
                            return Err(Error::Corrupt);
                        }
                    }
                    Ok(value) => { current_success = Some(value); }
                    Err(Fault::InputIo) if !counted && source_ok => { current_input_io += 1; }
                    Err(error) => {
                        unexpected = Some(error);
                        return Err(Error::Corrupt);
                    }
                }
            }
            // This diagnostic creates no CodecProbeEvidence or media authority.
            Ok(())
        });
        if visit.is_err() { visit_failed = true; break; }
    }
    // Attempt every fixed posthash even after a diagnostic failure. No source
    // bytes or raw tool output are retained or printed in the test report.
    let mut originals_unchanged = true;
    for (file, (_, length, digest)) in held.iter().zip(specs) {
        originals_unchanged &= pinned(file, length, digest);
    }
    originals_unchanged &= directories.check().is_ok();
    let selected_valid = selected.check(&cancel).is_ok();
    let tool_valid = tool.check(&cancel, overall_deadline).is_ok();
    println!("{}", serde_json::json!({
        "kind":"youtube_metadata_pipe_comparison","originalsUnchanged":originals_unchanged,
        "selectedValid":selected_valid,"toolValid":tool_valid,"units":counts,
        "currentInputIo":current_input_io,"countedSuccesses":counted_successes,
        "reproduction":if current_input_io != 0 { "input_io_observed" } else { "not_reproduced" },
        "unexpected":unexpected.map(|fault|format!("{fault:?}")),"rows":rows
    }));
    assert!(originals_unchanged && selected_valid && tool_valid, "fixed source/tool guard failed");
    assert!(!visit_failed && unexpected.is_none(), "metadata comparison failed");
    assert!(counts.iter().all(|count| *count > 0 && *count <= 16));
    assert_eq!(counted_successes, counts[0] + counts[1]);
    assert_eq!(rows.len(), counted_successes * 2);
}
