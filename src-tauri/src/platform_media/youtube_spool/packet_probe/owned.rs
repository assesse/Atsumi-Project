//! Closed operations over finalizer-owned, held files. The existing contained
//! runner owns every child and worker; this module is not a general path API.
use super::super::{
    finalize::{self, HeldInput, OutputSlot, OwnedKind, OwnedSelection, SelectedRemux},
    packet_facts::Rational,
};
use super::*;
use std::{io::Cursor, path::Path};

fn owned_error(error: finalize::Fault) -> Fault {
    match error {
        finalize::Fault::Cancelled => Fault::Cancelled,
        finalize::Fault::Deadline => Fault::Deadline,
        finalize::Fault::Space | finalize::Fault::Bounds => Fault::OutputLimit,
        finalize::Fault::Io => Fault::OutputIo,
        _ => Fault::SourceChanged,
    }
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

fn local_input(command: &mut Command, path: &Path, kind: OwnedKind) {
    let webm = matches!(kind, OwnedKind::DerivedVideo);
    command.args([
        "-f",
        if webm { "matroska" } else { "mov" },
        "-protocol_whitelist",
        "file",
    ]);
    if !webm {
        command.args(["-enable_drefs", "0", "-use_absolute_path", "0"]);
    }
    command.arg("-i").arg(path);
}

fn selection(kind: OwnedKind, selected: OwnedSelection) -> Result<(Track, u8)> {
    match (kind, selected) {
        (OwnedKind::DerivedVideo, OwnedSelection::DerivedVideo0)
        | (OwnedKind::Candidate, OwnedSelection::CandidateVideo0) => Ok((Track::Video, 0)),
        (OwnedKind::DerivedAudio, OwnedSelection::DerivedAudio0) => Ok((Track::Audio, 0)),
        (OwnedKind::Candidate, OwnedSelection::CandidateAudio1) => Ok((Track::Audio, 1)),
        _ => Err(Fault::UnsupportedInput),
    }
}

fn selected_probe_arguments(
    command: &mut Command,
    path: &Path,
    kind: OwnedKind,
    selected: OwnedSelection,
) -> Result<(Track, u8)> {
    // Only the finalizer-held Selected candidate can use the post-analysis
    // rewind. Raw stdin and derived/Whole probes retain their original path.
    if kind != OwnedKind::Candidate {
        return Err(Fault::UnsupportedInput);
    }
    let (track, stream_index) = selection(kind, selected)?;
    // An unspecified interval end can expose uninitialized FFprobe fields and
    // silently stop early. Explicitly initialize a packet-count end one ABOVE
    // the decoder's acceptance cap: valid inputs still read through EOF, while
    // excess packets cannot be hidden by ending at the expected tape count.
    let packet_limit = packet_facts::ROWS.checked_add(1)
        .filter(|limit| *limit < i32::MAX as u64)
        .ok_or(Fault::OutputLimit)?;
    local_input(command, path, kind);
    command.arg("-read_intervals").arg(format!("0%+#{packet_limit}"));
    command.args([
        "-select_streams", if track == Track::Video { "v:0" } else { "a:0" },
        "-show_packets", "-show_data_hash", "sha256",
        "-show_entries", ENTRIES, "-of", PROJECTION,
    ]);
    Ok((track, stream_index))
}

fn inputs_check(
    inputs: &[&HeldInput],
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<()> {
    checkpoint(cancel, deadline)?;
    tool.check(cancel, deadline).map_err(tool_error)?;
    for input in inputs {
        input.check(cancel, deadline).map_err(owned_error)?;
    }
    Ok(())
}

fn inputs_rehash(
    inputs: &[&HeldInput],
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<()> {
    inputs_check(inputs, tool, cancel, deadline)?;
    for input in inputs {
        input.rehash(cancel, deadline).map_err(owned_error)?;
    }
    inputs_check(inputs, tool, cancel, deadline)
}

#[cfg(windows)]
pub(in crate::platform_media::youtube_spool) fn probe_owned(
    input: &HeldInput,
    tool: &ManagedProbe,
    selected: OwnedSelection,
    cancel: &AtomicBool,
    deadline: Instant,
    consume: impl FnMut(packet_facts::PacketFact) -> std::result::Result<(), packet_facts::Fault> + Send,
) -> Result<ProbeFacts> {
    let (track, stream_index) = selection(input.kind(), selected)?;
    inputs_check(&[input], tool, cancel, deadline)?;
    let mut command = process_command(tool, false);
    local_input(&mut command, input.path(), input.kind());
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
    let evidence = run_with_callback(
        command,
        &mut Cursor::new([]),
        Range { start: 0, end: 0 },
        track,
        stream_index,
        cancel,
        deadline,
        || inputs_check(&[input], tool, cancel, deadline),
        consume,
    )?;
    // Empty stdin's digest is not evidence for file input. The held capability
    // binds its own complete file hash and length before/after the operation.
    inputs_rehash(&[input], tool, cancel, deadline)?;
    Ok(ProbeFacts {
        packets: evidence.packets,
        stream: evidence.stream,
    })
}

#[cfg(windows)]
pub(in crate::platform_media::youtube_spool) fn probe_selected_owned(
    input: &HeldInput,
    tool: &ManagedProbe,
    selected: OwnedSelection,
    cancel: &AtomicBool,
    deadline: Instant,
    consume: impl FnMut(packet_facts::PacketFact) -> std::result::Result<(), packet_facts::Fault> + Send,
) -> Result<ProbeFacts> {
    let mut command = process_command(tool, false);
    let (track, stream_index) =
        selected_probe_arguments(&mut command, input.path(), input.kind(), selected)?;
    inputs_check(&[input], tool, cancel, deadline)?;
    let evidence = run_with_callback(
        command,
        &mut Cursor::new([]),
        Range { start: 0, end: 0 },
        track,
        stream_index,
        cancel,
        deadline,
        || inputs_check(&[input], tool, cancel, deadline),
        consume,
    )?;
    // The explicit interval cap exceeds the decoder's accepted packet count.
    // Missing/changed/excess packets still fail the complete tape comparison;
    // the exact held file is rehashed to physical EOF.
    inputs_rehash(&[input], tool, cancel, deadline)?;
    Ok(ProbeFacts {
        packets: evidence.packets,
        stream: evidence.stream,
    })
}

#[cfg(windows)]
pub(in crate::platform_media::youtube_spool) fn metadata_owned(
    input: &HeldInput,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<Vec<u8>> {
    inputs_check(&[input], tool, cancel, deadline)?;
    let mut command = process_command(tool, false);
    local_input(&mut command, input.path(), input.kind());
    // Deliberately no stream selection: extra tracks must remain observable.
    command.args([
        "-show_streams",
        "-show_data_hash",
        "sha256",
        "-show_entries",
        METADATA_ENTRIES,
        "-of",
        "json",
    ]);
    let (_, bytes) = run_output(
        command,
        &mut Cursor::new([]),
        Range { start: 0, end: 0 },
        cancel,
        deadline,
        || inputs_check(&[input], tool, cancel, deadline),
        metadata_bytes,
    )?;
    inputs_rehash(&[input], tool, cancel, deadline)?;
    Ok(bytes)
}

fn common_offset(origin: Rational) -> Result<String> {
    if origin.numerator < 0 || origin.denominator <= 0 {
        return Err(Fault::UnsupportedInput);
    }
    // FFmpeg's output offset is microsecond based. Floor a single common
    // origin; downstream packet comparisons enforce actual tick error bounds.
    let micros = origin
        .numerator
        .checked_mul(1_000_000)
        .ok_or(Fault::UnsupportedInput)?
        / origin.denominator;
    if micros > i64::MAX as i128 {
        return Err(Fault::UnsupportedInput);
    }
    Ok(format!("-{}.{:06}", micros / 1_000_000, micros % 1_000_000))
}

fn selected_audio_filter((first, end): (u64, u64)) -> Result<String> {
    if first >= end || end > 20_000_000 {
        return Err(Fault::UnsupportedInput);
    }
    // Native evidence supplies decimal packet ordinals, not an expression.
    // Escape commas for FFmpeg's BSF parser; no shell is involved.
    Ok(format!(
        "noise=amount=0:drop=lt(n\\,{first})+gte(n\\,{end})"
    ))
}

fn remux_arguments(
    command: &mut Command,
    video: &Path,
    audio: &Path,
    output: &Path,
    offset: &str,
    audio_filter: Option<&str>,
) {
    command.args(["-nostdin", "-n", "-copyts"]);
    local_input(command, video, OwnedKind::DerivedVideo);
    local_input(command, audio, OwnedKind::DerivedAudio);
    command.args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"]);
    if let Some(filter) = audio_filter {
        command.args(["-bsf:a:0", filter]);
        // Preserve a positive AAC start offset without flat-MP4 tail discard.
        // Indexed fragments are still subject to complete packet/decode proof.
        command.args(["-movflags", "frag_keyframe+delay_moov+dash"]);
    }
    command
        .args([
            "-avoid_negative_ts",
            "disabled",
            "-output_ts_offset",
            offset,
            "-f",
            "mp4",
        ])
        .arg(output);
}

#[cfg(windows)]
pub(in crate::platform_media::youtube_spool) fn remux_owned(
    video: &HeldInput,
    audio: &HeldInput,
    output: &OutputSlot,
    origin: Rational,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<()> {
    if !matches!(video.kind(), OwnedKind::DerivedVideo)
        || !matches!(audio.kind(), OwnedKind::DerivedAudio)
    {
        return Err(Fault::UnsupportedInput);
    }
    let offset = common_offset(origin)?;
    inputs_check(&[video, audio], tool, cancel, deadline)?;
    output.begin(cancel, deadline).map_err(owned_error)?;
    let mut command = process_command(tool, true);
    remux_arguments(
        &mut command,
        video.path(),
        audio.path(),
        output.path(),
        &offset,
        None,
    );
    let (_, bytes) = run_output(
        command,
        &mut Cursor::new([]),
        Range { start: 0, end: 0 },
        cancel,
        deadline,
        || {
            inputs_check(&[video, audio], tool, cancel, deadline)?;
            output.check(cancel, deadline).map_err(owned_error)
        },
        metadata_bytes,
    )?;
    if !bytes.is_empty() {
        return Err(Fault::OutputIo);
    }
    inputs_rehash(&[video, audio], tool, cancel, deadline)?;
    output.check(cancel, deadline).map_err(owned_error)
}

#[cfg(windows)]
pub(in crate::platform_media::youtube_spool) fn remux_selected_owned(
    video: &HeldInput,
    audio: &HeldInput,
    output: &OutputSlot,
    selected: &SelectedRemux<'_>,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<()> {
    if !matches!(video.kind(), OwnedKind::DerivedVideo)
        || !matches!(audio.kind(), OwnedKind::DerivedAudio)
    {
        return Err(Fault::UnsupportedInput);
    }
    let offset = common_offset(selected.origin())?;
    let filter = selected_audio_filter(selected.ordinals())?;
    inputs_check(&[video, audio], tool, cancel, deadline)?;
    // The finalizer alone constructs this proof after both complete derived
    // tracks match their source tapes. Check it before claiming the output.
    selected.check_audio(audio, cancel, deadline).map_err(owned_error)?;
    output.begin(cancel, deadline).map_err(owned_error)?;
    let mut command = process_command(tool, true);
    remux_arguments(
        &mut command,
        video.path(),
        audio.path(),
        output.path(),
        &offset,
        Some(&filter),
    );
    let (_, bytes) = run_output(
        command,
        &mut Cursor::new([]),
        Range { start: 0, end: 0 },
        cancel,
        deadline,
        || {
            inputs_check(&[video, audio], tool, cancel, deadline)?;
            selected.check_audio(audio, cancel, deadline).map_err(owned_error)?;
            output.check(cancel, deadline).map_err(owned_error)
        },
        metadata_bytes,
    )?;
    if !bytes.is_empty() {
        return Err(Fault::OutputIo);
    }
    inputs_rehash(&[video, audio], tool, cancel, deadline)?;
    selected.check_audio(audio, cancel, deadline).map_err(owned_error)?;
    output.check(cancel, deadline).map_err(owned_error)
}

#[cfg(windows)]
pub(in crate::platform_media::youtube_spool) fn decode_owned(
    candidate: &HeldInput,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<()> {
    if !matches!(candidate.kind(), OwnedKind::Candidate) {
        return Err(Fault::UnsupportedInput);
    }
    inputs_check(&[candidate], tool, cancel, deadline)?;
    let mut command = process_command(tool, true);
    command.args(["-nostdin", "-xerror"]);
    local_input(&mut command, candidate.path(), candidate.kind());
    // Required, not optional maps: a missing audio or video track must fail.
    command.args(["-map", "0:v:0", "-map", "0:a:0", "-f", "null", "-"]);
    let (_, bytes) = run_output(
        command,
        &mut Cursor::new([]),
        Range { start: 0, end: 0 },
        cancel,
        deadline,
        || inputs_check(&[candidate], tool, cancel, deadline),
        metadata_bytes,
    )?;
    if !bytes.is_empty() {
        return Err(Fault::OutputIo);
    }
    inputs_rehash(&[candidate], tool, cancel, deadline)
}

#[cfg(not(windows))]
pub(in crate::platform_media::youtube_spool) fn probe_owned(
    _: &HeldInput,
    _: &ManagedProbe,
    _: OwnedSelection,
    _: &AtomicBool,
    _: Instant,
    _: impl FnMut(packet_facts::PacketFact) -> std::result::Result<(), packet_facts::Fault> + Send,
) -> Result<ProbeFacts> {
    Err(Fault::UnsupportedInput)
}
#[cfg(not(windows))]
pub(in crate::platform_media::youtube_spool) fn probe_selected_owned(
    _: &HeldInput,
    _: &ManagedProbe,
    _: OwnedSelection,
    _: &AtomicBool,
    _: Instant,
    _: impl FnMut(packet_facts::PacketFact) -> std::result::Result<(), packet_facts::Fault> + Send,
) -> Result<ProbeFacts> {
    Err(Fault::UnsupportedInput)
}
#[cfg(not(windows))]
pub(in crate::platform_media::youtube_spool) fn metadata_owned(
    _: &HeldInput,
    _: &ManagedProbe,
    _: &AtomicBool,
    _: Instant,
) -> Result<Vec<u8>> {
    Err(Fault::UnsupportedInput)
}
#[cfg(not(windows))]
pub(in crate::platform_media::youtube_spool) fn remux_owned(
    _: &HeldInput,
    _: &HeldInput,
    _: &OutputSlot,
    _: Rational,
    _: &ManagedProbe,
    _: &AtomicBool,
    _: Instant,
) -> Result<()> {
    Err(Fault::UnsupportedInput)
}
#[cfg(not(windows))]
pub(in crate::platform_media::youtube_spool) fn remux_selected_owned(
    _: &HeldInput,
    _: &HeldInput,
    _: &OutputSlot,
    _: &SelectedRemux<'_>,
    _: &ManagedProbe,
    _: &AtomicBool,
    _: Instant,
) -> Result<()> {
    Err(Fault::UnsupportedInput)
}
#[cfg(not(windows))]
pub(in crate::platform_media::youtube_spool) fn decode_owned(
    _: &HeldInput,
    _: &ManagedProbe,
    _: &AtomicBool,
    _: Instant,
) -> Result<()> {
    Err(Fault::UnsupportedInput)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remux_args(filter: Option<&str>) -> Vec<String> {
        let mut command = Command::new("unused");
        command.args(["-v", "error", "-hide_banner"]);
        remux_arguments(
            &mut command,
            Path::new("owned video.webm"),
            Path::new("owned audio.mp4"),
            Path::new("merge-owned.mp4"),
            "-46789.997256",
            filter,
        );
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn whole_remux_argv_is_unchanged_and_has_no_selection_filter() {
        assert_eq!(
            remux_args(None),
            [
                "-v", "error", "-hide_banner", "-nostdin", "-n", "-copyts",
                "-f", "matroska", "-protocol_whitelist", "file", "-i", "owned video.webm",
                "-f", "mov", "-protocol_whitelist", "file", "-enable_drefs", "0",
                "-use_absolute_path", "0", "-i", "owned audio.mp4",
                "-map", "0:v:0", "-map", "1:a:0", "-c", "copy",
                "-avoid_negative_ts", "disabled", "-output_ts_offset", "-46789.997256",
                "-f", "mp4", "merge-owned.mp4",
            ]
        );
    }

    #[test]
    fn selected_remux_has_only_fixed_audio_filter_and_indexed_fragment_deltas() {
        for (first, end) in [(0, 1), (1, 430), (0, 20_000_000), (19_999_999, 20_000_000)] {
            let filter = selected_audio_filter((first, end)).unwrap();
            assert_eq!(
                filter,
                format!("noise=amount=0:drop=lt(n\\,{first})+gte(n\\,{end})")
            );
            let mut args = remux_args(Some(&filter));
            let filters: Vec<_> = args.iter().enumerate()
                .filter(|(_, arg)| arg.starts_with("-bsf"))
                .map(|(index, _)| index).collect();
            assert_eq!(filters.len(), 1);
            let at = filters[0];
            assert_eq!(&args[at - 2..at], &["-c", "copy"]);
            assert_eq!(args[at], "-bsf:a:0");
            assert_eq!(args[at + 1], filter);
            assert_eq!(&args[at + 2..at + 4], &["-movflags", "frag_keyframe+delay_moov+dash"]);
            assert_eq!(args.iter().filter(|arg| *arg == "-movflags").count(), 1);
            assert_eq!(args[at + 4], "-avoid_negative_ts");
            args.drain(at..at + 4);
            assert_eq!(args, remux_args(None));
        }
    }

    #[test]
    fn selected_probe_rewinds_only_candidates_with_a_limit_above_acceptance() {
        for (selected, track, index, selector) in [
            (OwnedSelection::CandidateVideo0, Track::Video, 0, "v:0"),
            (OwnedSelection::CandidateAudio1, Track::Audio, 1, "a:0"),
        ] {
            let mut command = Command::new("unused");
            assert_eq!(
                selected_probe_arguments(&mut command, Path::new("held candidate.mp4"), OwnedKind::Candidate, selected),
                Ok((track, index)),
            );
            let args: Vec<_> = command.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();
            assert_eq!(args, [
                "-f", "mov", "-protocol_whitelist", "file", "-enable_drefs", "0",
                "-use_absolute_path", "0", "-i", "held candidate.mp4",
                "-read_intervals", "0%+#20000001", "-select_streams", selector,
                "-show_packets", "-show_data_hash", "sha256", "-show_entries", ENTRIES,
                "-of", PROJECTION,
            ]);
            let interval_index = args.iter().position(|arg| arg == "-read_intervals").unwrap();
            let limit: u64 = args[interval_index + 1].strip_prefix("0%+#").unwrap().parse().unwrap();
            assert_eq!(limit, packet_facts::ROWS + 1);
            assert!(limit < i32::MAX as u64);
        }
    }

    #[test]
    fn selected_probe_rejects_every_derived_or_wrong_role_before_adding_arguments() {
        for kind in [OwnedKind::DerivedVideo, OwnedKind::DerivedAudio, OwnedKind::Candidate] {
            for selected in [OwnedSelection::DerivedVideo0, OwnedSelection::DerivedAudio0,
                OwnedSelection::CandidateVideo0, OwnedSelection::CandidateAudio1] {
                if kind == OwnedKind::Candidate && matches!(selected,
                    OwnedSelection::CandidateVideo0 | OwnedSelection::CandidateAudio1) { continue; }
                let mut command = Command::new("unused");
                assert_eq!(selected_probe_arguments(&mut command, Path::new("not opened"), kind, selected),
                    Err(Fault::UnsupportedInput));
                assert_eq!(command.get_args().count(), 0);
            }
        }
    }

    #[test]
    fn selected_filter_rejects_empty_reverse_and_unbounded_ordinals() {
        for ordinals in [
            (0, 0), (1, 1), (2, 1), (0, 20_000_001),
            (20_000_000, 20_000_001), (0, u64::MAX), (u64::MAX, u64::MAX),
        ] {
            assert_eq!(selected_audio_filter(ordinals), Err(Fault::UnsupportedInput));
        }
    }

    #[test]
    fn owned_selection_cannot_cross_input_roles() {
        assert!(selection(OwnedKind::DerivedVideo, OwnedSelection::CandidateVideo0).is_err());
        assert!(selection(OwnedKind::DerivedAudio, OwnedSelection::DerivedVideo0).is_err());
        assert!(selection(OwnedKind::Candidate, OwnedSelection::DerivedAudio0).is_err());
        assert_eq!(
            selection(OwnedKind::Candidate, OwnedSelection::CandidateAudio1).unwrap(),
            (Track::Audio, 1)
        );
    }
    #[test]
    fn common_origin_is_not_independently_zeroed_or_rounded_up() {
        assert_eq!(
            common_offset(Rational::new(2063438879, 44100).unwrap()).unwrap(),
            "-46789.997256"
        );
        assert_eq!(
            common_offset(Rational::new(0, 1).unwrap()).unwrap(),
            "-0.000000"
        );
        assert!(common_offset(Rational::new(-1, 1000).unwrap()).is_err());
        assert!(common_offset(Rational {
            numerator: i128::MAX,
            denominator: 1
        })
        .is_err());
    }
    #[test]
    fn owned_inputs_force_local_formats_and_disable_mov_references() {
        for (kind, format) in [
            (OwnedKind::DerivedVideo, "matroska"),
            (OwnedKind::DerivedAudio, "mov"),
            (OwnedKind::Candidate, "mov"),
        ] {
            let mut command = Command::new("unused");
            local_input(&mut command, Path::new("candidate.mp4"), kind);
            let args: Vec<_> = command
                .get_args()
                .map(|x| x.to_string_lossy().into_owned())
                .collect();
            assert_eq!(&args[..4], &["-f", format, "-protocol_whitelist", "file"]);
            if format == "mov" {
                assert_eq!(
                    &args[4..8],
                    &["-enable_drefs", "0", "-use_absolute_path", "0"]
                );
            }
            assert_eq!(&args[args.len() - 2..], &["-i", "candidate.mp4"]);
        }
    }
}
