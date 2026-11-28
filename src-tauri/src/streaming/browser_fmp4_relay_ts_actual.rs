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

fn decode(exe: &Path, cwd: &Path, bytes: Vec<u8>, format: &str) -> Check<process::Output> {
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
            format,
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

fn probe(exe: &Path, cwd: &Path, bytes: Vec<u8>) -> Check<process::Output> {
    process::run(
        exe,
        cwd,
        &[
            "-v",
            "error",
            "-protocol_whitelist",
            "pipe",
            "-f",
            "mpegts",
            "-i",
            "pipe:0",
            "-show_packets",
            "-show_entries",
            "packet=stream_index,pts,dts,size,flags",
            "-of",
            "json",
        ],
        Some(Arc::new(bytes)),
        2 * 1024 * 1024,
        false,
    )
}

#[test]
#[ignore = "Immutable actual codec source -> native TS in RAM; FFprobe and full decode; no network/media output"]
fn actual_native_ts_preserves_admitted_av_and_each_segment_decodes() -> Check<()> {
    require(
        std::env::var("ATSUMI_NATIVE_TS_QA").as_deref() == Ok(ID),
        "qa_opt_in",
    )?;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("qa_workspace")?;
    let source = workspace.join(format!(
        ".runtime/soop-original-qa/baa5338478ee4c8cb1cdda57d278d918/media/SOOP/{ID}/e00000008.mp4"
    ));
    no_links(&source)?;
    let source = fs::canonicalize(source).map_err(|_| "qa_source_path")?;
    let _directories = crate::streaming::browser_store::cleanup::pin_directories(
        source.parent().ok_or("qa_parent")?,
    )
    .map_err(|_| "qa_source_pin")?;
    let mut held = crate::streaming::browser_store::cleanup::verification_guard(&source)
        .map_err(|_| "qa_source_pin")?;
    require(
        held.metadata().map_err(|_| "qa_stat")?.len() == 1_789_078,
        "qa_source_size",
    )?;
    let mut bytes = Vec::new();
    (&mut held)
        .take(2 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|_| "qa_read")?;
    require(hash(&bytes) == SOURCE_SHA, "qa_source_hash")?;
    let audit = workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume");
    no_links(&audit)?;
    let outdir = audit.join(format!("native-ts-actual-{}", Uuid::new_v4().simple()));
    fs::create_dir(&outdir).map_err(|_| "qa_dir")?;
    let tools = workspace
        .join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let ffmpeg = tools.join("ffmpeg.exe");
    let ffprobe = tools.join("ffprobe.exe");
    for (tool, sha) in [
        (&ffmpeg, TOOL_SHA),
        (
            &ffprobe,
            "713b3273eff80a441666ec66bc7ea6b339962177f38a4d57cf2f8a4d71c738fa",
        ),
    ] {
        no_links(tool)?;
        require(
            fs::metadata(tool).map_err(|_| "qa_tool")?.len() <= 16 * 1024 * 1024,
            "qa_tool_size",
        )?;
        require(
            hash(&fs::read(tool).map_err(|_| "qa_tool")?) == sha,
            "qa_tool_hash",
        )?;
    }
    let mut report = serde_json::json!({"passed":false,"sourceSha256":SOURCE_SHA,"sourceBytes":bytes.len(),
        "originalWholePieceEqualityClaim":false,"sourceSamples":[720,563],"admittedSamples":[600,470],
        "prerollSamples":[120,93],"qualification":"Actual admitted source only; B-frame source, live receiver, YouTube ingest, viewer delay and archive/chat mapping remain unverified."});
    let result = (|| -> Check<()> {
        let mut split = 0;
        for expected in [*b"ftyp", *b"moov"] {
            let a = atoms(&bytes[split..]).map_err(|_| "qa_init")?[0];
            require(a.kind == expected, "qa_init")?;
            split += a.bytes.len();
        }
        let input = EncodedTrackInput {
            track_index: 0,
            mime_type: "video/mp4;codecs=avc1.640028,mp4a.40.2".into(),
            init: bytes[..split].to_vec(),
        };
        let mut baseline_reader =
            EncodedSampleReader::new(vec![input.clone()]).map_err(|_| "qa_reader")?;
        let batch = baseline_reader
            .push(0, &bytes[split..])
            .map_err(|_| "qa_samples")?;
        require(
            batch.video.len() == 720 && batch.audio.len() == 563,
            "qa_source_counts",
        )?;
        require(
            batch.video[120].key()
                && batch.video[120].dts == 180090
                && batch.audio[93].dts == 95472,
            "qa_admission",
        )?;
        let selected_video: Vec<_> = batch.video.iter().skip(120).cloned().collect();
        let selected_audio: Vec<_> = batch.audio.iter().skip(93).cloned().collect();
        let preroll_bytes: usize = batch
            .video
            .iter()
            .take(120)
            .chain(batch.audio.iter().take(93))
            .map(|s| s.data.len())
            .sum();
        let mut baseline = baseline_reader.descriptor.0.init.clone();
        baseline.extend(make_fragment(1, &selected_video, 0, 1).map_err(|_| "qa_baseline")?);
        baseline.extend(make_fragment(2, &selected_audio, 0, 2).map_err(|_| "qa_baseline")?);
        let mut reader = EncodedSampleReader::new(vec![input]).map_err(|_| "qa_reader")?;
        let mut mux =
            TsMuxer::new(reader.descriptor()).map_err(|e| format!("qa_ts_init:{:?}", e))?;
        let mut segments = Vec::new();
        for piece in bytes[split..].chunks(64 * 1024) {
            segments.extend(
                mux.push(reader.push(0, piece).map_err(|_| "qa_reader_chunk")?)
                    .map_err(|e| format!("qa_ts_chunk:{:?}", e))?,
            );
        }
        let (tail, end) = mux
            .finish(reader.finish().map_err(|_| "qa_reader_end")?)
            .map_err(|e| format!("qa_ts_end:{:?}", e))?;
        segments.extend(tail);
        require(
            end.preroll
                == PrerollCounts {
                    video_samples: 120,
                    audio_samples: 93,
                    bytes: preroll_bytes as u64,
                },
            "qa_preroll",
        )?;
        require(
            end.origin.ticks == 95472 && end.origin.timescale == 48000,
            "qa_origin",
        )?;
        require(segments.len() == 5 && end.segments() == 5, "qa_segments")?;
        // Held159c evidence, before timing metadata existed. No transport byte
        // may change when adding the source/wire ledger.
        let prior_sha = [
            "15e15144364c329659a732a0990d01511f16ebbe62c933e546e8be490e9bb53e",
            "37c6ccff6fdb9cc7847f7a1888975fab4e1f796b56b62a5b4efc0ed715c27abd",
            "4849d622d31726ed2ee2bf0f9ce95597ac8cc034cd4db6755eaddccc98b54c65",
            "f2bf3b2eb1b66d211e411e237e19269048d9ca529ecccdf263820f1fe0eb6db4",
            "e2c9370056b91781e33d40400daae2d5d6e1a26e29ad12399de4739d9aaf4d61",
        ];
        require(segments.iter().zip(prior_sha).all(|(s,h)|hash(s.bytes())==h),"qa_pre_timing_bytes_unchanged")?;
        report["preTimingBytesUnchanged"]=serde_json::json!(true);
        report["sealedTiming"]=serde_json::json!(segments.iter().map(|s|s.timing()).collect::<Vec<_>>());
        require(
            segments.iter().map(|s| s.video.samples).sum::<u64>() == 600
                && segments.iter().map(|s| s.audio.samples).sum::<u64>() == 470,
            "qa_admitted_counts",
        )?;
        let wire: Vec<u8> = segments
            .iter()
            .flat_map(|s| s.bytes().iter().copied())
            .collect();
        require(wire.len() < 16 * 1024 * 1024, "qa_wire_bound")?;
        report["segments"]=serde_json::json!(segments.iter().map(|s|serde_json::json!({"ordinal":s.ordinal(),"bytes":s.bytes().len(),"sha256":hash(s.bytes()),"durationMs":s.duration_ms(),"videoSamples":s.video.samples,"audioSamples":s.audio.samples,"videoSourceFirst":s.video.first_dts,"videoSourceEnd":s.video.last_end,"audioSourceFirst":s.audio.first_dts,"audioSourceEnd":s.audio.last_end})).collect::<Vec<_>>());
        report["origin"] = serde_json::json!({"ticks":end.origin.ticks,"timescale":end.origin.timescale,"videoBase":end.origin.video_base,"audioBase":end.origin.audio_base,"prerollBytes":preroll_bytes});
        let p = probe(&ffprobe, &outdir, wire.clone())?;
        report["ffprobe"] = p.facts.clone();
        require(
            p.facts["success"].as_bool() == Some(true)
                && p.facts["stderrRetainedBytes"].as_u64() == Some(0),
            "qa_probe",
        )?;
        let v: serde_json::Value = serde_json::from_slice(&p.bytes).map_err(|_| "qa_probe_json")?;
        let rows = v["packets"].as_array().ok_or("qa_probe_packets")?;
        let mut indexes = [0usize; 2];
        let mut max_delta = 0f64;
        for row in rows {
            let stream = row["stream_index"]
                .as_u64()
                .filter(|n| *n < 2)
                .ok_or("qa_probe_stream")? as usize;
            let sample = if stream == 0 {
                selected_video.get(indexes[stream])
            } else {
                selected_audio.get(indexes[stream])
            }
            .ok_or("qa_probe_extra")?;
            let scale = if stream == 0 { 90000 } else { 48000 };
            for (field, tick) in [
                ("dts", i128::from(sample.dts)),
                ("pts", pts(sample).map_err(|_| "qa_pts")?),
            ] {
                let actual = row[field].as_i64().ok_or("qa_probe_clock")?;
                let expected = clock(tick, scale, end.origin).map_err(|_| "qa_clock")?;
                require(
                    actual >= 0 && actual as u64 == expected,
                    "qa_probe_exact_90k",
                )?;
                let delta = (actual as f64 / 90000.0
                    - (tick as f64 / f64::from(scale)
                        - end.origin.ticks as f64 / f64::from(end.origin.timescale)))
                .abs();
                require(delta <= 1.0 / 90000.0, "qa_probe_rational")?;
                max_delta = max_delta.max(delta);
            }
            indexes[stream] += 1;
        }
        require(indexes == [600, 470], "qa_probe_all_packets")?;
        report["packetClocks"] = serde_json::json!({"counts":indexes,"allSourceRescaledExactly":true,"maxRationalDeltaSeconds":max_delta});
        let a = decode(&ffmpeg, &outdir, baseline, "mov")?;
        let b = decode(&ffmpeg, &outdir, wire, "mpegts")?;
        report["sourceDecode"] = a.facts.clone();
        report["tsDecode"] = b.facts.clone();
        for decoded in [&a, &b] {
            require(
                decoded.facts["success"].as_bool() == Some(true)
                    && decoded.facts["stderrRetainedBytes"].as_u64() == Some(0),
                "qa_decode",
            )?;
        }
        report["decodedComparison"] =
            compare(&decoded(&a.bytes)?, &decoded(&b.bytes)?, end.origin)?;
        let mut single = Vec::new();
        for s in &segments {
            let out = decode(&ffmpeg, &outdir, s.bytes().to_vec(), "mpegts")?;
            require(
                out.facts["success"].as_bool() == Some(true)
                    && out.facts["stderrRetainedBytes"].as_u64() == Some(0),
                "qa_independent_decode",
            )?;
            let rows = decoded(&out.bytes)?;
            require(
                rows.rows[0].len() == s.video.samples as usize,
                "qa_independent_frames",
            )?;
            require(
                rows.rows[1].iter().map(|r| r.bytes as u64).sum::<u64>()
                    == s.audio.samples * 1024 * 2 * 2,
                "qa_independent_pcm",
            )?;
            single.push(serde_json::json!({"ordinal":s.ordinal(),"videoFrames":rows.rows[0].len(),"audioFrames":rows.rows[1].len(),"process":out.facts}));
        }
        report["independentSegments"] = serde_json::json!(single);
        Ok(())
    })();
    held.seek(SeekFrom::Start(0))
        .map_err(|_| "qa_source_seek")?;
    let mut after = Vec::new();
    (&mut held)
        .take(2 * 1024 * 1024)
        .read_to_end(&mut after)
        .map_err(|_| "qa_source_read")?;
    require(hash(&after) == SOURCE_SHA, "qa_original_changed")?;
    report["originalUnchanged"] = serde_json::json!(true);
    require(
        fs::read_dir(&outdir)
            .map_err(|_| "qa_files")?
            .next()
            .is_none(),
        "qa_media_file",
    )?;
    report["childMediaFiles"] = serde_json::json!(0);
    report["passed"] = serde_json::json!(result.is_ok());
    if let Err(e) = &result {
        report["error"] = serde_json::json!(e);
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(outdir.join("result.json"))
        .map_err(|_| "qa_report")?;
    file.write_all(&serde_json::to_vec_pretty(&report).map_err(|_| "qa_report")?)
        .and_then(|_| file.sync_all())
        .map_err(|_| "qa_report")?;
    println!(
        "QA_NATIVE_TS_RESULT={}",
        outdir.join("result.json").display()
    );
    result
}
