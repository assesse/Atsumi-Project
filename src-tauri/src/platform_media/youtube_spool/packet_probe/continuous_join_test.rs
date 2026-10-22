//! Explicit root-only local fixture. This is not production admission or a path API.
use super::super::{
    files::{Directories, GuardedFile},
    index,
    packet_facts::{PacketFact, PairComparison, Rational, Times},
    reconstruct::TrackBuilder,
    recovery::SequentialSource,
    Chunk, Mode, Owner, SpoolWriter, Window,
};
use super::*;
use crate::platform_media::PlatformMediaTools;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

struct Held {
    path: PathBuf,
    file: GuardedFile,
    len: u64,
    hash: [u8; 32],
}
impl Held {
    fn open(path: PathBuf, cancel: &AtomicBool, deadline: Instant) -> Self {
        let file = GuardedFile::open(&path, false).unwrap();
        let len = file.check().unwrap();
        assert!(len > 0 && len <= 2 * 1024 * 1024);
        let hash = hash_range(&mut &file.file, Range::new(0, len).unwrap(), || {
            checkpoint(cancel, deadline)
        })
        .unwrap();
        Self {
            path,
            file,
            len,
            hash,
        }
    }
    fn check(&self) -> Result<()> {
        if self.file.check().map_err(source_error)? != self.len {
            return Err(Fault::SourceChanged);
        }
        Ok(())
    }
    fn rehash(&self, cancel: &AtomicBool, deadline: Instant) {
        self.check().unwrap();
        assert_eq!(
            hash_range(
                &mut &self.file.file,
                Range::new(0, self.len).unwrap(),
                || checkpoint(cancel, deadline)
            )
            .unwrap(),
            self.hash
        );
    }
}
fn hex(hash: [u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}
fn add(
    records: &mut Vec<PacketFact>,
    packet: PacketFact,
) -> std::result::Result<(), packet_facts::Fault> {
    if records.len() >= 1024 {
        return Err(packet_facts::Fault::Bounds);
    }
    records.push(packet);
    Ok(())
}
fn local_input(command: &mut Command, path: &Path, format: Format) {
    command.args([
        "-f",
        if format == Format::Vp9Webm {
            "matroska"
        } else {
            "mov"
        },
        "-protocol_whitelist",
        "file",
    ]);
    if format == Format::AacMp4 {
        command.args(["-enable_drefs", "0", "-use_absolute_path", "0"]);
    }
    command.arg("-i").arg(path);
}
fn process_command(tool: &ManagedProbe, ffmpeg: bool) -> Command {
    let mut command = Command::new(if ffmpeg {
        tool.ffmpeg()
    } else {
        tool.executable()
    });
    command
        .current_dir(tool.cwd())
        .env_clear()
        .envs(tool.environment())
        .args(["-v", "error", "-hide_banner"]);
    command
}
fn probe_local(
    tool: &ManagedProbe,
    held: &Held,
    directories: &Directories,
    track: Track,
    stream_index: u8,
    format: Format,
    cancel: &AtomicBool,
    deadline: Instant,
) -> (Vec<PacketFact>, StreamFacts) {
    let mut command = process_command(tool, false);
    local_input(&mut command, &held.path, format);
    command.args([
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
    let mut records = Vec::new();
    let evidence = run_with_callback(
        command,
        &mut Cursor::new(Vec::<u8>::new()),
        Range { start: 0, end: 0 },
        track,
        stream_index,
        cancel,
        deadline,
        || {
            held.check()?;
            directories.check().map_err(source_error)?;
            tool.check(cancel, deadline).map_err(tool_error)
        },
        |p| add(&mut records, p),
    )
    .unwrap();
    // The runner's empty-input hash is not proof for this file-input invocation.
    held.rehash(cancel, deadline);
    (records, evidence.stream)
}
fn compare(
    track: Track,
    source: &[PacketFact],
    output: &[PacketFact],
    source_tb: TimeBase,
    output_tb: TimeBase,
    origin: Rational,
) -> packet_facts::PairReport {
    assert_eq!(source.len(), output.len());
    let mut compare = PairComparison::new(track, source_tb, output_tb, origin).unwrap();
    for (&a, &b) in source.iter().zip(output) {
        compare.push(a, b).unwrap();
    }
    let result = compare.finish().unwrap();
    let a = result.source_tick;
    let b = result.output_tick;
    let tolerance = Rational::new(
        a.numerator
            .checked_mul(b.denominator)
            .unwrap()
            .checked_add(b.numerator.checked_mul(a.denominator).unwrap())
            .unwrap(),
        a.denominator.checked_mul(b.denominator).unwrap(),
    )
    .unwrap();
    for error in [
        result.max_pts_error,
        result.max_dts_error,
        result.max_duration_error,
    ] {
        assert!(error.numerator >= 0);
        assert!(
            error.numerator.checked_mul(tolerance.denominator).unwrap()
                <= tolerance.numerator.checked_mul(error.denominator).unwrap(),
            "packet timing exceeds actual tick bound: {result:?}"
        );
    }
    result
}
fn empty_process(
    command: Command,
    inputs: &[&Held],
    dirs: &Directories,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) {
    let (_, bytes) = run_output(
        command,
        &mut Cursor::new(Vec::<u8>::new()),
        Range { start: 0, end: 0 },
        cancel,
        deadline,
        || {
            for input in inputs {
                input.check()?;
            }
            dirs.check().map_err(source_error)?;
            tool.check(cancel, deadline).map_err(tool_error)
        },
        metadata_bytes,
    )
    .unwrap();
    assert!(bytes.is_empty());
    for input in inputs {
        input.rehash(cancel, deadline);
    }
}

#[test]
#[ignore = "root-only fixed original capture, creates a new preserved QA directory; never publishes"]
fn actual_continuous_units_join_without_packet_loss() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let qa_parent = workspace.join(".runtime/youtube-webm-raw-pair-qa");
    let originals = qa_parent.join("c26c9147-735f-4ed3-821b-31e92cef9afd");
    let source_dirs = Directories::hold(&originals).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now() + Duration::from_secs(180);
    let manifest = Held::open(originals.join("manifest.json"), &cancel, deadline);
    assert_eq!(manifest.len, 3840);
    assert_eq!(
        hex(manifest.hash),
        "c4a92fbff4e9c27870b817e1d2015cb66990eaf9ec47568ead953026c15508ad"
    );
    let originals: Vec<_> = [
        (
            "video-1.webm",
            95635,
            "9be82861467e27ed66fa8220db1a7c4f1e2872996755257cbeafe149035070a4",
        ),
        (
            "video-2.webm",
            76753,
            "7bb5f6e6f3fcd435042dc5b7227ca023799014f7f3e7886e4a99bc732451a4b6",
        ),
        (
            "audio-1.mp4",
            81810,
            "c349dc8672a8ade3d961d19c7d0b30f09473b08b96c7eb58641cbaceb385779a",
        ),
        (
            "audio-2.mp4",
            82185,
            "b9fa4f91f7dc48bee2a9f7128188f27b3b041b6faf16f91e19b7a3e17daf7457",
        ),
    ]
    .into_iter()
    .map(|(name, length, expected)| {
        let held = Held::open(originals.join(name), &cancel, deadline);
        assert_eq!(held.len, length);
        assert_eq!(hex(held.hash), expected);
        held
    })
    .collect();
    let bin = workspace
        .join(".runtime/media-tools")
        .join(managed_probe::PACKAGE)
        .join("bin");
    let tool = ManagedProbe::open(
        &PlatformMediaTools {
            yt_dlp: PathBuf::new(),
            deno: PathBuf::new(),
            ffmpeg: bin.join("ffmpeg.exe"),
            ffprobe: bin.join("ffprobe.exe"),
        },
        &cancel,
        deadline,
    )
    .unwrap();
    let job = uuid::Uuid::new_v4().to_string();
    let output = qa_parent.join(format!("continuous-join-{job}"));
    let mut dirs = Directories::hold(&qa_parent).unwrap();
    dirs.create_child(&output).unwrap();
    println!("preserved_fixture={}", output.display());
    let mut spool_dirs = Directories::hold(&output).unwrap();
    spool_dirs.create_child(&output.join("YouTube")).unwrap();
    let identity = SpoolIdentity {
        job_id: job,
        request_id: uuid::Uuid::new_v4().to_string(),
        receiver_id: uuid::Uuid::new_v4().to_string(),
        source_id: uuid::Uuid::new_v4().to_string(),
        document_id: uuid::Uuid::new_v4().to_string(),
        document_epoch: 1,
        source_generation: 1,
        video_id: "T4XZmMPQ9Kw".into(),
        channel_id: format!("UC{}", "a".repeat(22)),
        owner: Owner::Manual,
    };
    let mut writer = SpoolWriter::create_new(
        &output,
        identity.clone(),
        ClockDescriptor {
            video_timestamp_scale_ns: 1_000_000,
            audio_timescale: 44100,
            mode: Mode::Segments,
            append_window: Window::Default,
            video_timestamp_offset: 0,
            audio_timestamp_offset: 0,
        },
        cancel.clone(),
    )
    .unwrap();
    for (at, held) in originals.iter().enumerate() {
        let mut reader = &held.file.file;
        reader.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = vec![0; held.len as usize]; // fixed fixture, each below 128KiB
        reader.read_exact(&mut bytes).unwrap();
        writer
            .append(
                &identity,
                Chunk {
                    track: if at < 2 { Track::Video } else { Track::Audio },
                    append_index: (at % 2) as u64,
                    chunk_index: 0,
                    final_chunk: true,
                    bytes,
                },
            )
            .unwrap();
    }
    writer.seal(&identity, SealCause::Stopped).unwrap();
    drop(writer);
    let mut source = SequentialSource::prepare(&output, &identity, &cancel).unwrap();
    let mut records: [Vec<PacketFact>; 2] = [Vec::new(), Vec::new()];
    let mut streams: [Option<StreamFacts>; 2] = [None, None];
    let mut codecs: [Option<codec_facts::SourceFacts>; 2] = [None, None];
    for track in [Track::Video, Track::Audio] {
        let at = track.index();
        let mut units = 0;
        source
            .visit(track, &cancel, |span| {
                let probe =
                    probe_span(span, &tool, &cancel, deadline, |p| add(&mut records[at], p))
                        .expect("original packet probe");
                assert_eq!(probe.input_sha256, originals[at * 2 + units].hash);
                if let Some(stream) = &streams[at] {
                    assert_eq!(stream, &probe.stream);
                } else {
                    streams[at] = Some(probe.stream);
                }
                units += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(units, 2);
        source
            .visit(track, &cancel, |span| {
                let probe = probe_codec_span(span, &tool, &cancel, deadline)
                    .expect("original whole codec probe");
                if let Some(codec) = &codecs[at] {
                    assert_eq!(codec, &probe.codec);
                } else {
                    codecs[at] = Some(probe.codec);
                }
                Ok(())
            })
            .unwrap();
    }
    assert_eq!([records[0].len(), records[1].len()], [300, 431]);
    assert_eq!(records[0][0].encode_record().unwrap()[1], 1);
    let mut derived = Vec::new();
    for track in [Track::Video, Track::Audio] {
        let at = track.index();
        let path = output.join(if at == 0 {
            "derived-video.webm"
        } else {
            "derived-audio.mp4"
        });
        let mut file = GuardedFile::open(&path, true).unwrap();
        let mut builder = TrackBuilder::new(track, &mut file.file);
        source
            .visit(track, &cancel, |span| builder.push(span, &cancel))
            .unwrap();
        let summary = builder.finish(&cancel).unwrap();
        drop(builder);
        file.file.sync_all().unwrap();
        assert_eq!(summary.units, 2);
        assert_eq!(file.check().unwrap(), summary.bytes);
        drop(file);
        let held = Held::open(path, &cancel, deadline);
        assert_eq!(held.hash, summary.sha256);
        let mut reader = &held.file.file;
        let mut indexed = Vec::new();
        let format = if at == 0 {
            Format::Vp9Webm
        } else {
            Format::AacMp4
        };
        let outcome = index::scan(
            format,
            &mut reader,
            held.len,
            if at == 0 { 1_000_000 } else { 44100 },
            &cancel,
            &mut |unit| {
                indexed.push(unit);
                Ok(())
            },
        );
        assert_eq!(outcome.terminal, index::Terminal::Complete);
        assert_eq!(outcome.complete_units, 1);
        assert_eq!(indexed[0].facts.items, records[at].len() as u64);
        assert_eq!(indexed[0].facts.clock, summary.clock);
        let (joined, stream) =
            probe_local(&tool, &held, &dirs, track, 0, format, &cancel, deadline);
        assert_eq!(
            joined, records[at],
            "derived track changed original packet facts"
        );
        assert_eq!(&stream, streams[at].as_ref().unwrap());
        println!(
            "derived_{track:?}: bytes={} hash={} packets={} clock={:?}",
            held.len,
            hex(held.hash),
            joined.len(),
            summary.clock
        );
        derived.push(held);
    }
    assert_eq!(
        streams[0].as_ref().unwrap().time_base,
        TimeBase::new(1, 1000).unwrap()
    );
    assert_eq!(
        streams[1].as_ref().unwrap().time_base,
        TimeBase::new(1, 44100).unwrap()
    );
    // Fixed verified source timebases; earliest original PTS/DTS, not arrival time.
    let earliest = |packets: &[PacketFact]| {
        packets
            .iter()
            .copied()
            .map(Times::from)
            .map(|times| times.pts.min(times.dts))
            .min()
            .unwrap()
    };
    let v = Rational::new(earliest(&records[0]) as i128, 1000).unwrap();
    let a = Rational::new(earliest(&records[1]) as i128, 44100).unwrap();
    let origin = if v.numerator * a.denominator <= a.numerator * v.denominator {
        v
    } else {
        a
    };
    assert!(origin.numerator > 0);
    // FFmpeg accepts microsecond offsets. Floor avoids creating negative priming.
    let micros = origin.numerator.checked_mul(1_000_000).unwrap() / origin.denominator;
    let offset = format!("-{}.{:06}", micros / 1_000_000, micros % 1_000_000);
    let candidate_path = output.join("candidate.mp4");
    assert!(!candidate_path.exists());
    let mut command = process_command(&tool, true);
    command.args(["-nostdin", "-n", "-copyts"]);
    local_input(&mut command, &derived[0].path, Format::Vp9Webm);
    local_input(&mut command, &derived[1].path, Format::AacMp4);
    command
        .args([
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c",
            "copy",
            "-avoid_negative_ts",
            "disabled",
            "-output_ts_offset",
            &offset,
            "-f",
            "mp4",
        ])
        .arg(&candidate_path);
    empty_process(
        command,
        &[&derived[0], &derived[1]],
        &dirs,
        &tool,
        &cancel,
        deadline,
    );
    let candidate = Held::open(candidate_path, &cancel, deadline);
    let mut metadata = process_command(&tool, false);
    local_input(&mut metadata, &candidate.path, Format::AacMp4);
    metadata.args([
        "-show_streams",
        "-show_data_hash",
        "sha256",
        "-show_entries",
        METADATA_ENTRIES,
        "-of",
        "json",
    ]);
    let (_, bytes) = run_output(
        metadata,
        &mut Cursor::new(Vec::<u8>::new()),
        Range { start: 0, end: 0 },
        &cancel,
        deadline,
        || {
            candidate.check()?;
            dirs.check().map_err(source_error)?;
            tool.check(&cancel, deadline).map_err(tool_error)
        },
        metadata_bytes,
    )
    .unwrap();
    let codec = codec_facts::candidate(
        &bytes,
        codecs[0].as_ref().unwrap(),
        codecs[1].as_ref().unwrap(),
    )
    .unwrap();
    for track in [Track::Video, Track::Audio] {
        let at = track.index();
        let (output_packets, output_stream) = probe_local(
            &tool,
            &candidate,
            &dirs,
            track,
            at as u8,
            Format::AacMp4,
            &cancel,
            deadline,
        );
        let report = compare(
            track,
            &records[at],
            &output_packets,
            streams[at].as_ref().unwrap().time_base,
            output_stream.time_base,
            origin,
        );
        println!("candidate_{track:?}: {report:?}");
    }
    let mut decode = process_command(&tool, true);
    decode.args(["-nostdin", "-xerror"]);
    local_input(&mut decode, &candidate.path, Format::AacMp4);
    decode.args(["-map", "0:v:0", "-map", "0:a:0", "-f", "null", "-"]);
    empty_process(decode, &[&candidate], &dirs, &tool, &cancel, deadline);
    for held in &originals {
        held.rehash(&cancel, deadline);
    }
    manifest.rehash(&cancel, deadline);
    source_dirs.check().unwrap();
    dirs.check().unwrap();
    tool.check(&cancel, deadline).unwrap();
    println!("candidate_bytes={} candidate_sha256={} origin={origin:?} codec={codec:?} full_decode=pass original_unchanged=true production_recording_verified=false", candidate.len, hex(candidate.hash));
}
