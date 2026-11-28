//! Opt-in actual codec check, using one immutable existing source piece.
//! No capture, YouTube call, user profile, network, or media-file output.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

#[path = "../../examples/support/relay_pipe_process.rs"]
mod process;
type Check<T> = std::result::Result<T, String>;
const ID: &str = "8fd00700-81d4-401f-a083-0d13fcf286ea";
const SOURCE_SHA: &str = "861f08cf68605ff4e7393e652a149d137552aaef4093b981637dc6e43911d088";
const TOOL_SHA: &str = "c2a51bdad17d321c9c11768a4bc6137ac6c666d62ba82fe4b4050fbd1db9d2a0";

fn require(condition: bool, label: &str) -> Check<()> {
    if condition {
        Ok(())
    } else {
        Err(label.into())
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn no_links(path: &Path) -> Check<()> {
    use std::os::windows::fs::MetadataExt;
    for part in path.ancestors() {
        let m = fs::symlink_metadata(part).map_err(|_| "qa_path")?;
        require(
            !m.file_type().is_symlink() && m.file_attributes() & 0x400 == 0,
            "qa_link",
        )?;
    }
    Ok(())
}

#[derive(Debug)]
struct Decoded {
    clocks: [f64; 2],
    rows: [Vec<Row>; 2],
    stereo_48000: bool,
}
#[derive(Debug)]
struct Row {
    dts: i64,
    pts: i64,
    duration: i64,
    bytes: usize,
    sha: String,
}
fn decoded(bytes: &[u8]) -> Check<Decoded> {
    require(bytes.len() <= 2 * 1024 * 1024, "qa_decode_bound")?;
    let mut clocks = [None, None];
    let mut sample_rate = false;
    let mut stereo = false;
    let mut rows: [Vec<Row>; 2] = [Vec::new(), Vec::new()];
    for line in std::str::from_utf8(bytes)
        .map_err(|_| "qa_decode_utf8")?
        .lines()
    {
        if let Some(header) = line.strip_prefix("#tb ") {
            let (index, ratio) = header.split_once(':').ok_or("qa_timebase")?;
            let index: usize = index.trim().parse().map_err(|_| "qa_timebase")?;
            require(
                index < 2 && clocks[index].is_none(),
                "qa_duplicate_timebase",
            )?;
            let (num, den) = ratio.trim().split_once('/').ok_or("qa_timebase")?;
            let num: u32 = num.parse().map_err(|_| "qa_timebase")?;
            let den: u32 = den.parse().map_err(|_| "qa_timebase")?;
            require(num > 0 && den > 0, "qa_timebase")?;
            clocks[index] = Some(f64::from(num) / f64::from(den));
        } else if let Some(rate) = line.strip_prefix("#sample_rate 1:") {
            require(
                !sample_rate && rate.trim() == "48000",
                "qa_audio_sample_rate",
            )?;
            sample_rate = true;
        } else if let Some(layout) = line.strip_prefix("#channel_layout_name 1:") {
            require(!stereo && layout.trim() == "stereo", "qa_audio_layout")?;
            stereo = true;
        } else if !line.is_empty() && !line.starts_with('#') {
            let f: Vec<_> = line.split(',').map(str::trim).collect();
            require(f.len() == 6, "qa_decode_row")?;
            let index: usize = f[0].parse().map_err(|_| "qa_decode_stream")?;
            require(index < 2 && rows[index].len() < 2000, "qa_decode_stream")?;
            let row = Row {
                dts: f[1].parse().map_err(|_| "qa_dts")?,
                pts: f[2].parse().map_err(|_| "qa_pts")?,
                duration: f[3].parse().map_err(|_| "qa_duration")?,
                bytes: f[4].parse().map_err(|_| "qa_size")?,
                sha: f[5].into(),
            };
            require(
                row.duration > 0
                    && row.bytes > 0
                    && row.sha.len() == 64
                    && row.sha.bytes().all(|c| c.is_ascii_hexdigit()),
                "qa_decode_payload",
            )?;
            rows[index].push(row);
        }
    }
    require(
        rows.iter().all(|r| !r.is_empty()),
        "qa_decode_missing_stream",
    )?;
    Ok(Decoded {
        clocks: [
            clocks[0].ok_or("qa_timebase_missing")?,
            clocks[1].ok_or("qa_timebase_missing")?,
        ],
        rows,
        stereo_48000: sample_rate && stereo,
    })
}

fn decode(exe: &Path, cwd: &Path, bytes: Vec<u8>) -> Check<process::Output> {
    process::run(
        exe,
        cwd,
        &[
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-xerror",
            "-err_detect",
            "explode",
            "-protocol_whitelist",
            "pipe",
            "-f",
            "mov",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-map",
            "0:a:0",
            "-c:v",
            "rawvideo",
            "-c:a",
            "pcm_s16le",
            "-copyts",
            "-fps_mode:v",
            "passthrough",
            "-enc_time_base:v",
            "demux",
            "-threads",
            "2",
            "-f",
            "framehash",
            "-hash",
            "sha256",
            "pipe:1",
        ],
        Some(Arc::new(bytes)),
        2 * 1024 * 1024,
        false,
    )
}

fn compare(source: &Decoded, output: &Decoded, origin: SourceOrigin) -> Check<serde_json::Value> {
    // Guard against identical early decoder termination. This pinned AAC-LC
    // fixture has 470 admitted 1024-sample stereo packets, verified below.
    for decoded in [source, output] {
        require(decoded.rows[0].len() == 600, "qa_all_video_frames")?;
        require(decoded.stereo_48000, "qa_pcm_format")?;
        let pcm_bytes = decoded.rows[1]
            .iter()
            .try_fold(0usize, |n, r| n.checked_add(r.bytes))
            .ok_or("qa_pcm_bound")?;
        require(pcm_bytes == 470 * 1024 * 2 * 2, "qa_all_pcm_samples")?;
    }
    let shifts = [
        origin.video_base as f64 / 90000.0,
        origin.audio_base as f64 / 48000.0,
    ];
    let mut result = Vec::new();
    for stream in 0..2 {
        require(
            source.rows[stream].len() == output.rows[stream].len(),
            "qa_decode_count",
        )?;
        let mut max_delta = 0f64;
        for (s, o) in source.rows[stream].iter().zip(&output.rows[stream]) {
            require(s.bytes == o.bytes && s.sha == o.sha, "qa_decode_content")?;
            for (a, b) in [(s.dts, o.dts), (s.pts, o.pts)] {
                let delta = (a as f64 * source.clocks[stream]
                    - shifts[stream]
                    - b as f64 * output.clocks[stream])
                    .abs();
                require(delta <= 0.001, "qa_decode_clock")?;
                max_delta = max_delta.max(delta);
            }
        }
        let s = source.rows[stream].last().ok_or("qa_decode_count")?;
        let o = output.rows[stream].last().ok_or("qa_decode_count")?;
        let source_end =
            s.pts.checked_add(s.duration).ok_or("qa_decode_end")? as f64 * source.clocks[stream];
        let output_end =
            o.pts.checked_add(o.duration).ok_or("qa_decode_end")? as f64 * output.clocks[stream];
        let end_delta = (source_end - shifts[stream] - output_end).abs();
        require(end_delta <= 0.001, "qa_decode_end")?;
        result.push(serde_json::json!({"stream":stream,"count":source.rows[stream].len(),"allPayloadHashesEqual":true,
            "maxClockDeltaSeconds":max_delta,"sourceDecodedEndSeconds":source_end,"outputDecodedEndSeconds":output_end,
            "clockShiftSeconds":shifts[stream],"endDeltaSeconds":end_delta}));
    }
    Ok(serde_json::json!(result))
}

#[test]
#[ignore = "Exact immutable actual codec piece; no network/media files; opt-in and owned decoder only"]
fn actual_source_piece_preserves_admitted_payload_and_continuous_clock() -> Check<()> {
    if std::env::var("ATSUMI_CONTINUOUS_SAMPLE_QA").as_deref() != Ok(ID) {
        return Err("qa_opt_in".into());
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("qa_workspace")?;
    let source = workspace.join(format!(
        ".runtime/soop-original-qa/baa5338478ee4c8cb1cdda57d278d918/media/SOOP/{ID}/e00000008.mp4"
    ));
    no_links(&source)?;
    let source = fs::canonicalize(source).map_err(|_| "qa_source_path")?;
    let _directories = crate::streaming::browser_store::cleanup::pin_directories(
        source.parent().ok_or("qa_source_parent")?,
    )
    .map_err(|_| "qa_source_pin")?;
    let mut held = crate::streaming::browser_store::cleanup::verification_guard(&source)
        .map_err(|_| "qa_source_pin")?;
    require(
        held.metadata().map_err(|_| "qa_source_stat")?.len() == 1_789_078,
        "qa_source_size",
    )?;
    let mut bytes = Vec::new();
    (&mut held)
        .take(2 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|_| "qa_source_read")?;
    require(hash(&bytes) == SOURCE_SHA, "qa_source_hash")?;
    let mut split = 0;
    for expected in [*b"ftyp", *b"moov"] {
        let atom = atoms(&bytes[split..]).map_err(|_| "qa_boxes")?[0];
        require(atom.kind == expected, "qa_initial_boxes")?;
        split += atom.bytes.len();
    }
    let mut reader = EncodedSampleReader::new(vec![EncodedTrackInput {
        track_index: 0,
        mime_type: "video/mp4;codecs=avc1.640028,mp4a.40.2".into(),
        init: bytes[..split].to_vec(),
    }])
    .map_err(|_| "qa_reader_init")?;
    let mut muxer = ContinuousMp4Muxer::new(reader.descriptor());
    let batch = reader
        .push(0, &bytes[split..])
        .map_err(|_| "qa_reader_samples")?;
    // Exact independently observed packet layout of this pinned source. A later
    // RAP is required because AAC starts at +5ms. Nothing is changed on disk.
    require(
        (batch.video.len(), batch.audio.len()) == (720, 563),
        "qa_source_counts",
    )?;
    require(
        batch.video[120].key() && batch.video[120].dts == 180090 && batch.audio[93].dts == 95472,
        "qa_source_admission",
    )?;
    let selected_video: Vec<_> = batch.video.iter().skip(120).cloned().collect();
    let selected_audio: Vec<_> = batch.audio.iter().skip(93).cloned().collect();
    require(
        selected_audio.iter().all(|s| s.duration == 1024),
        "qa_aac_lc_durations",
    )?;
    let preroll_bytes: usize = batch
        .video
        .iter()
        .take(120)
        .chain(batch.audio.iter().take(93))
        .map(|s| s.data.len())
        .sum();
    let init = reader.descriptor.0.init.clone();
    // Baseline retains admitted ORIGINAL source ticks (base=0); it is not an
    // entire-piece comparison. The new mux applies its one shared origin.
    let mut baseline = init.clone();
    baseline.extend(make_fragment(1, &selected_video, 0, 1).map_err(|_| "qa_baseline_video")?);
    baseline.extend(make_fragment(2, &selected_audio, 0, 2).map_err(|_| "qa_baseline_audio")?);
    let output = muxer
        .push(batch)
        .map_err(|_| "qa_continuous_push")?
        .ok_or("qa_start_missing")?;
    let end = muxer
        .finish(reader.finish().map_err(|_| "qa_reader_end")?)
        .map_err(|_| "qa_continuous_end")?;
    require(
        end.preroll
            == (PrerollCounts {
                video_samples: 120,
                audio_samples: 93,
                bytes: preroll_bytes as u64,
            }),
        "qa_preroll",
    )?;
    require(
        output.origin.ticks == 95472 && output.origin.timescale == 48000,
        "qa_origin",
    )?;
    // Re-admit output to compare every encoded payload/duration/CTS and fixed
    // per-track origin, independently of full-decode success below.
    let mut check = EncodedSampleReader::new(vec![EncodedTrackInput {
        track_index: 0,
        mime_type: "video/mp4;codecs=avc1.640028,mp4a.40.2".into(),
        init: init.clone(),
    }])
    .map_err(|_| "qa_output_init")?;
    let converted = check
        .push(0, &output.bytes[init.len()..])
        .map_err(|_| "qa_output_samples")?;
    for (expected, actual, base) in [
        (&selected_video, &converted.video, output.origin.video_base),
        (&selected_audio, &converted.audio, output.origin.audio_base),
    ] {
        require(expected.len() == actual.len(), "qa_encoded_count")?;
        for (s, o) in expected.iter().zip(actual) {
            require(
                s.data == o.data
                    && s.duration == o.duration
                    && s.flags == o.flags
                    && s.cts == o.cts
                    && s.dts.checked_sub(base) == Some(o.dts),
                "qa_encoded_content_clock",
            )?;
        }
    }
    check.finish().map_err(|_| "qa_output_end")?;
    let audit = workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume");
    no_links(&audit)?;
    let outdir = audit.join(format!(
        "continuous-actual-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir(&outdir).map_err(|_| "qa_result_dir")?;
    let ffmpeg = workspace.join(
        ".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin/ffmpeg.exe",
    );
    no_links(&ffmpeg)?;
    require(
        fs::metadata(&ffmpeg).map_err(|_| "qa_tool_stat")?.len() <= 16 * 1024 * 1024,
        "qa_tool_bound",
    )?;
    require(
        hash(&fs::read(&ffmpeg).map_err(|_| "qa_tool_read")?) == TOOL_SHA,
        "qa_tool_hash",
    )?;
    let baseline_hash = hash(&baseline);
    let output_hash = hash(&output.bytes);
    let output_len = output.bytes.len();
    let mut report = serde_json::json!({"passed":false,"sourceSha256":SOURCE_SHA,"sourceBytes":bytes.len(),
        "sourceSamples":[720,563],"admittedSamples":[600,470],"prerollSamples":[120,93],"prerollBytes":preroll_bytes,
        "originTicks":output.origin.ticks,"originTimescale":output.origin.timescale,"videoBase":output.origin.video_base,
        "audioBase":output.origin.audio_base,"encodedPayloadsAndClocksExact":true,"baselineSha256":baseline_hash,
        "outputSha256":output_hash,"outputBytes":output_len,"wholeOriginalDecodedEqualityClaim":false,
        "qualification":"Actual single piece, admitted source ranges only; no live ingress, network relay, B-frame, viewer latency, or archive-map proof"});
    let result = (|| -> Check<()> {
        let a = decode(&ffmpeg, &outdir, baseline)?;
        report["sourceDecode"] = a.facts.clone();
        require(
            a.facts["success"].as_bool() == Some(true)
                && a.facts["stderrRetainedBytes"].as_u64() == Some(0),
            "qa_source_decode_failed",
        )?;
        let b = decode(&ffmpeg, &outdir, output.bytes)?;
        report["outputDecode"] = b.facts.clone();
        require(
            b.facts["success"].as_bool() == Some(true)
                && b.facts["stderrRetainedBytes"].as_u64() == Some(0),
            "qa_output_decode_failed",
        )?;
        report["decodedComparison"] =
            compare(&decoded(&a.bytes)?, &decoded(&b.bytes)?, output.origin)?;
        Ok(())
    })();
    held.seek(SeekFrom::Start(0))
        .map_err(|_| "qa_source_seek")?;
    let mut after = Vec::new();
    (&mut held)
        .take(2 * 1024 * 1024)
        .read_to_end(&mut after)
        .map_err(|_| "qa_source_reread")?;
    require(hash(&after) == SOURCE_SHA, "qa_source_changed")?;
    report["originalUnchanged"] = serde_json::json!(true);
    require(
        fs::read_dir(&outdir)
            .map_err(|_| "qa_child_files")?
            .next()
            .is_none(),
        "qa_child_created_file",
    )?;
    report["childFilesCreated"] = serde_json::json!(0);
    report["passed"] = serde_json::json!(result.is_ok());
    if let Err(error) = &result {
        report["error"] = serde_json::json!(error);
    }
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(outdir.join("result.json"))
        .map_err(|_| "qa_report")?;
    file.write_all(&serde_json::to_vec_pretty(&report).map_err(|_| "qa_report")?)
        .and_then(|_| file.sync_all())
        .map_err(|_| "qa_report")?;
    println!(
        "QA_CONTINUOUS_RESULT={}",
        outdir.join("result.json").display()
    );
    result
}
