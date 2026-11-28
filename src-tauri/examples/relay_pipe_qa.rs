//! Opt-in OFFLINE experiment, not production relay or YouTube validation.
//! relay_pipe_qa.exe --ffmpeg ABSOLUTE_VERIFIED_FFMPEG_PATH
//! Only a result JSON is written, inside a fresh workspace audit directory.
#[cfg(windows)]
#[path = "support/relay_pipe_process.rs"]
mod process;

#[cfg(windows)]
mod qa {
    use super::process;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        io::{Read, Write},
        path::{Path, PathBuf},
        sync::Arc,
    };
    type Check<T> = Result<T, String>;
    const REMUX: &[&str] = &[
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
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
        "-c",
        "copy",
        "-copyts",
        "-f",
        "flv",
        "-flvflags",
        "no_duration_filesize",
        "pipe:1",
    ];
    fn require(ok: bool, message: &str) -> Check<()> {
        if ok {
            Ok(())
        } else {
            Err(message.into())
        }
    }
    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
    fn digest_file(path: &Path) -> Check<String> {
        let mut file = fs::File::open(path).map_err(|_| "tool_read")?;
        let size = file.metadata().map_err(|_| "tool_metadata")?.len();
        require(size > 0 && size <= 512 * 1024 * 1024, "tool_bound")?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|_| "tool_read")?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        Ok(format!("{:x}", hash.finalize()))
    }
    fn u24(b: &[u8]) -> u32 {
        ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32
    }
    #[derive(Clone)]
    struct FlvPacket {
        dts_ms: u32,
        pts_ms: i64,
        bytes: usize,
        sha256: String,
    }
    struct Flv {
        facts: Value,
        packets: [Vec<FlvPacket>; 2],
    }
    fn inspect_flv(bytes: &[u8]) -> Check<Flv> {
        require(
            bytes.len() >= 13 && &bytes[..3] == b"FLV" && bytes[3] == 1,
            "flv_header",
        )?;
        let header = u32::from_be_bytes(bytes[5..9].try_into().unwrap()) as usize;
        require(header == 9 && bytes[9..13] == [0; 4], "flv_header_size")?;
        let mut at = 13;
        let mut times: [Vec<u32>; 2] = [vec![], vec![]];
        let mut composition = Vec::new();
        let mut packets: [Vec<FlvPacket>; 2] = [vec![], vec![]];
        let mut avc = false;
        let mut aac = false;
        while at < bytes.len() {
            require(at + 15 <= bytes.len(), "flv_truncated_tag")?;
            let kind = bytes[at];
            let len = u24(&bytes[at + 1..at + 4]) as usize;
            let timestamp = u24(&bytes[at + 4..at + 7]) | ((bytes[at + 7] as u32) << 24);
            require(
                bytes[at + 8..at + 11] == [0; 3]
                    && len <= 2 * 1024 * 1024
                    && at + 15 + len <= bytes.len(),
                "flv_tag_bounds",
            )?;
            let body = &bytes[at + 11..at + 11 + len];
            require(
                u32::from_be_bytes(bytes[at + 11 + len..at + 15 + len].try_into().unwrap())
                    as usize
                    == len + 11,
                "flv_previous_size",
            )?;
            match kind {
                9 => {
                    require(body.len() >= 5 && body[0] & 15 == 7, "flv_not_avc")?;
                    if body[1] == 0 {
                        avc = true;
                    } else if body[1] == 1 {
                        require(avc, "avc_before_header")?;
                        times[0].push(timestamp);
                        let raw = u24(&body[2..5]);
                        let cts = ((raw << 8) as i32) >> 8;
                        require((-1000..=1000).contains(&cts), "composition_bound")?;
                        composition.push(cts);
                        packets[0].push(FlvPacket {
                            dts_ms: timestamp,
                            pts_ms: i64::from(timestamp) + i64::from(cts),
                            bytes: body.len() - 5,
                            sha256: sha(&body[5..]),
                        });
                    } else {
                        require(body[1] == 2, "avc_packet_type")?;
                    }
                }
                8 => {
                    require(body.len() >= 2 && body[0] >> 4 == 10, "flv_not_aac")?;
                    if body[1] == 0 {
                        aac = true;
                    } else {
                        require(body[1] == 1 && aac, "aac_header_or_type")?;
                        times[1].push(timestamp);
                        packets[1].push(FlvPacket {
                            dts_ms: timestamp,
                            pts_ms: i64::from(timestamp),
                            bytes: body.len() - 2,
                            sha256: sha(&body[2..]),
                        });
                    }
                }
                18 => {}
                _ => return Err("flv_unexpected_tag".into()),
            }
            at += len + 15;
        }
        let mut streams = vec![];
        for (index, values) in times.iter().enumerate() {
            require(values.len() >= 800, "flv_insufficient_samples")?;
            require(
                values
                    .windows(2)
                    .all(|w| w[1] >= w[0] && w[1] - w[0] <= 100),
                "timestamp_reset_or_gap",
            )?;
            let first = values[0];
            let last = *values.last().unwrap();
            require(
                last - first >= 29_900 && last - first <= 30_200,
                "flv_duration",
            )?;
            for boundary in [12_000, 24_000] {
                require(
                    values
                        .windows(2)
                        .any(|w| w[0] < boundary && w[1] >= boundary && w[1] - w[0] <= 100),
                    "boundary_not_crossed",
                )?;
            }
            streams.push(json!({"stream":index,"packets":values.len(),"firstDtsMs":first,"lastDtsMs":last,"firstSixDtsMs":values.iter().take(6).collect::<Vec<_>>(),"lastSixPackets":packets[index].iter().skip(packets[index].len().saturating_sub(6)).map(|p|json!({"dtsMs":p.dts_ms,"ptsMs":p.pts_ms,"durationMs":null})).collect::<Vec<_>>(),"crossed12s":true,"crossed24s":true}));
        }
        require(times[0].len() == 900 && avc && aac, "av_stream_count")?;
        Ok(Flv {
            facts: json!({"streams":streams,"videoPackets":times[0].len(),"audioPackets":times[1].len(),"ctsMinMs":composition.iter().min(),"ctsMaxMs":composition.iter().max(),"firstSixVideoCtsMs":composition.iter().take(6).collect::<Vec<_>>(),"lastSixVideoCtsMs":&composition[composition.len().saturating_sub(6)..],"bytes":bytes.len(),"sha256":sha(bytes)}),
            packets,
        })
    }
    fn decoded_head(bytes: &[u8]) -> Check<Value> {
        let text = std::str::from_utf8(bytes).map_err(|_| "decode_not_utf8")?;
        let mut headers = vec![];
        let mut rows: [Vec<Value>; 2] = [vec![], vec![]];
        let mut tails: [Vec<Value>; 2] = [vec![], vec![]];
        for line in text.lines() {
            if line.starts_with("#tb ") && headers.len() < 2 {
                headers.push(line.to_owned());
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<_> = line.split(',').map(str::trim).collect();
            // Packet-copy framehash can append side-data columns. This diagnostic
            // reads only the fixed numeric prefix; acceptance is checked separately.
            require(fields.len() >= 6, "decode_row")?;
            let id = fields[0].parse::<usize>().map_err(|_| "decode_stream")?;
            require(id < 2, "decode_stream")?;
            let values = fields[1..5]
                .iter()
                .map(|v| v.parse::<i64>().map_err(|_| "decode_head_number"))
                .collect::<Result<Vec<_>, _>>()?;
            let row =
                json!({"dts":values[0],"pts":values[1],"duration":values[2],"bytes":values[3]});
            if rows[id].len() < 6 {
                rows[id].push(row.clone());
            }
            if tails[id].len() == 6 {
                tails[id].remove(0);
            }
            tails[id].push(row);
        }
        Ok(json!({"timebases":headers,"streams":rows,"lastSixStreams":tails}))
    }
    fn quantized_step(delta_ticks: i64, timebase: f64) -> bool {
        timebase.is_finite()
            && timebase > 0.0
            && ((delta_ticks as f64) * timebase).abs() <= 0.001 + 1e-12
    }
    fn timing_rule_checks() -> Check<()> {
        for ticks in [-48, -32, -16, 0, 16, 32, 48] {
            require(
                quantized_step(ticks, 1.0 / 48000.0),
                "checker_rejects_ms_quantization",
            )?;
        }
        for ticks in [-49, 49] {
            require(
                !quantized_step(ticks, 1.0 / 48000.0),
                "checker_accepts_over_1ms",
            )?;
        }
        for ticks in [-1, 1] {
            require(
                !quantized_step(ticks, 1.0 / 30.0),
                "checker_accepts_video_frame_gap",
            )?;
        }
        for base in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            require(!quantized_step(0, base), "checker_accepts_invalid_timebase")?;
        }
        Ok(())
    }
    #[derive(Clone)]
    struct HashPacket {
        dts: i64,
        pts: i64,
        duration: i64,
        bytes: usize,
        sha256: String,
    }
    #[derive(Default, Clone)]
    struct HashStream {
        base: f64,
        rows: Vec<HashPacket>,
    }
    fn parse_hashes(bytes: &[u8], packet_side_data: bool) -> Check<[HashStream; 2]> {
        require(bytes.len() <= 2 * 1024 * 1024, "framehash_bound")?;
        let text = std::str::from_utf8(bytes).map_err(|_| "framehash_utf8")?;
        let mut streams = [HashStream::default(), HashStream::default()];
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("#tb ") {
                let (id, ratio) = rest.split_once(':').ok_or("framehash_timebase")?;
                let id = id.trim().parse::<usize>().map_err(|_| "framehash_stream")?;
                require(
                    id < 2 && streams[id].base == 0.0,
                    "framehash_duplicate_timebase",
                )?;
                let (n, d) = ratio.trim().split_once('/').ok_or("framehash_timebase")?;
                let n = n.parse::<u32>().map_err(|_| "framehash_timebase")?;
                let d = d.parse::<u32>().map_err(|_| "framehash_timebase")?;
                require(n > 0 && d > 0, "framehash_timebase")?;
                streams[id].base = f64::from(n) / f64::from(d);
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<_> = line.split(',').map(str::trim).collect();
            require(
                fields.len() == 6 || (packet_side_data && fields.len() > 6),
                "framehash_row",
            )?;
            let id = fields[0].parse::<usize>().map_err(|_| "framehash_stream")?;
            require(
                id < 2 && streams[id].base > 0.0 && streams[id].rows.len() < 10_000,
                "framehash_stream_bound",
            )?;
            let dts = fields[1].parse::<i64>().map_err(|_| "framehash_dts")?;
            let pts = fields[2].parse::<i64>().map_err(|_| "framehash_pts")?;
            let duration = fields[3].parse::<i64>().map_err(|_| "framehash_duration")?;
            let size = fields[4].parse::<usize>().map_err(|_| "framehash_size")?;
            require(
                dts >= 0
                    && pts >= 0
                    && duration > 0
                    && size > 0
                    && size <= 2 * 1024 * 1024
                    && dts.checked_add(duration).is_some()
                    && pts.checked_add(duration).is_some()
                    && fields[5].len() == 64
                    && fields[5].bytes().all(|c| c.is_ascii_hexdigit()),
                "framehash_payload",
            )?;
            streams[id].rows.push(HashPacket {
                dts,
                pts,
                duration,
                bytes: size,
                sha256: fields[5].to_ascii_lowercase(),
            });
        }
        require(
            streams.iter().all(|s| s.base > 0.0 && !s.rows.is_empty()),
            "framehash_missing_stream",
        )?;
        Ok(streams)
    }
    fn source_contiguous(stream: &HashStream) -> bool {
        !stream.rows.is_empty()
            && stream
                .rows
                .windows(2)
                .all(|pair| pair[0].dts.checked_add(pair[0].duration) == Some(pair[1].dts))
    }
    fn within_ms(a: f64, b: f64) -> bool {
        a.is_finite() && b.is_finite() && (a - b).abs() <= 0.001 + 1e-12
    }
    fn compare_packets(source: &HashStream, output: &[FlvPacket]) -> Check<Value> {
        require(source_contiguous(source), "source_packet_gap_or_overlap")?;
        require(
            source.rows.len() == output.len(),
            "remux_packet_count_changed",
        )?;
        let mut max_dts_ms: f64 = 0.0;
        let mut max_pts_ms: f64 = 0.0;
        for (index, (a, b)) in source.rows.iter().zip(output).enumerate() {
            require(
                a.bytes == b.bytes && a.sha256 == b.sha256,
                "remux_payload_changed",
            )?;
            let dts = a.dts as f64 * source.base;
            let pts = a.pts as f64 * source.base;
            let out_dts = f64::from(b.dts_ms) / 1000.0;
            let out_pts = b.pts_ms as f64 / 1000.0;
            if !within_ms(dts, out_dts) || !within_ms(pts, out_pts) {
                return Err(format!("remux_timestamp_changed: packet={index}, dts_delta_ms={:.9}, pts_delta_ms={:.9}", (out_dts-dts)*1000.0, (out_pts-pts)*1000.0));
            }
            max_dts_ms = max_dts_ms.max((out_dts - dts).abs() * 1000.0);
            max_pts_ms = max_pts_ms.max((out_pts - pts).abs() * 1000.0);
        }
        Ok(
            json!({"packets":output.len(),"payloadsIdentical":true,"maxDtsDeltaMs":max_dts_ms,"maxPtsDeltaMs":max_pts_ms}),
        )
    }
    fn compare_decoded(
        source: &HashStream,
        flv: &[FlvPacket],
        decoded: &HashStream,
    ) -> Check<Value> {
        require(
            source.rows.len() == flv.len() && source.rows.len() == decoded.rows.len(),
            "decoded_frame_count_changed",
        )?;
        require(source_contiguous(source), "source_packet_gap_or_overlap")?;
        let mut max_source_ms: f64 = 0.0;
        let mut max_flv_ms: f64 = 0.0;
        for (index, ((a, b), c)) in source.rows.iter().zip(flv).zip(&decoded.rows).enumerate() {
            let source_pts = a.pts as f64 * source.base;
            let flv_pts = b.pts_ms as f64 / 1000.0;
            let actual_pts = c.pts as f64 * decoded.base;
            if !within_ms(source_pts, actual_pts) || !within_ms(flv_pts, actual_pts) {
                return Err(format!("decoded_timestamp_changed: frame={index}, source_delta_ms={:.9}, flv_delta_ms={:.9}",(actual_pts-source_pts)*1000.0,(actual_pts-flv_pts)*1000.0));
            }
            max_source_ms = max_source_ms.max((actual_pts - source_pts).abs() * 1000.0);
            max_flv_ms = max_flv_ms.max((actual_pts - flv_pts).abs() * 1000.0);
        }
        let a = source.rows.last().ok_or("source_empty")?;
        let c = decoded.rows.last().ok_or("decode_empty")?;
        let source_end =
            a.pts.checked_add(a.duration).ok_or("source_end_overflow")? as f64 * source.base;
        let decoded_end =
            c.pts.checked_add(c.duration).ok_or("decode_end_overflow")? as f64 * decoded.base;
        let duration = decoded_end - decoded.rows[0].pts as f64 * decoded.base;
        Ok(
            json!({"decodedFrames":decoded.rows.len(),"timebaseSeconds":decoded.base,"durationSeconds":duration,"sourceContainerEndSeconds":source_end,"decodedEndSeconds":decoded_end,"maxSourcePtsDeltaMs":max_source_ms,"maxFlvPtsDeltaMs":max_flv_ms,"sourceContainerToDecodedEndDeltaMs":(decoded_end-source_end)*1000.0}),
        )
    }
    fn compare_full_decoders(input: &HashStream, output: &HashStream) -> Check<Value> {
        require(
            !input.rows.is_empty() && input.rows.len() == output.rows.len(),
            "decoder_baseline_count_changed",
        )?;
        let mut max_pts_ms: f64 = 0.0;
        for (index, (a, b)) in input.rows.iter().zip(&output.rows).enumerate() {
            require(
                a.bytes == b.bytes && a.sha256 == b.sha256,
                "decoder_baseline_payload_changed",
            )?;
            let expected = a.pts as f64 * input.base;
            let actual = b.pts as f64 * output.base;
            if !within_ms(expected, actual) {
                return Err(format!(
                    "decoder_baseline_pts_changed: frame={index}, delta_ms={:.9}",
                    (actual - expected) * 1000.0
                ));
            }
            max_pts_ms = max_pts_ms.max((actual - expected).abs() * 1000.0);
        }
        let a = input.rows.last().ok_or("baseline_empty")?;
        let b = output.rows.last().ok_or("decode_empty")?;
        let expected_end = a
            .pts
            .checked_add(a.duration)
            .ok_or("baseline_end_overflow")? as f64
            * input.base;
        let actual_end =
            b.pts.checked_add(b.duration).ok_or("decode_end_overflow")? as f64 * output.base;
        if !within_ms(expected_end, actual_end) {
            return Err(format!("decoder_baseline_end_changed: expected_seconds={expected_end:.12}, actual_seconds={actual_end:.12}, delta_ms={:.9}",(actual_end-expected_end)*1000.0));
        }
        Ok(
            json!({"frames":input.rows.len(),"decodedPayloadsIdentical":true,"maxPtsDeltaMs":max_pts_ms,"sourceDecodedEndSeconds":expected_end,"outputDecodedEndSeconds":actual_end,"finalEndDeltaMs":(actual_end-expected_end)*1000.0}),
        )
    }
    fn inspect_decode(
        bytes: &[u8],
        source: &[HashStream; 2],
        flv: &Flv,
        baseline: &[HashStream; 2],
    ) -> Check<Value> {
        let decoded = parse_hashes(bytes, false)?;
        require(
            decoded[0].rows.len() == 900 && decoded[1].rows.len() >= 1000,
            "decoded_frame_count",
        )?;
        let mut streams = vec![];
        for id in 0..2 {
            let mut facts = compare_decoded(&source[id], &flv.packets[id], &decoded[id])?;
            facts["inputDecodeComparison"] = compare_full_decoders(&baseline[id], &decoded[id])?;
            require(
                (29.95..=30.20).contains(
                    &facts["durationSeconds"]
                        .as_f64()
                        .ok_or("decoded_duration")?,
                ),
                "decoded_duration",
            )?;
            facts["stream"] = json!(id);
            streams.push(facts);
        }
        Ok(
            json!({"fullDecode":true,"streams":streams,"framehashBytes":bytes.len(),"framehashSha256":sha(bytes)}),
        )
    }
    fn preservation_rule_checks() -> Check<()> {
        let payload_hash = sha(&[1, 2, 3, 4]);
        let source = HashStream {
            base: 1.0 / 15360.0,
            rows: [(0, 840), (840, 512), (1352, 512)]
                .into_iter()
                .map(|(pts, duration)| HashPacket {
                    dts: pts,
                    pts,
                    duration,
                    bytes: 4,
                    sha256: payload_hash.clone(),
                })
                .collect(),
        };
        let flv: Vec<_> = [0, 55, 88]
            .into_iter()
            .map(|pts| FlvPacket {
                dts_ms: pts,
                pts_ms: i64::from(pts),
                bytes: 4,
                sha256: payload_hash.clone(),
            })
            .collect();
        let decoded = HashStream {
            base: 0.001,
            rows: [0, 55, 88]
                .into_iter()
                .map(|pts| HashPacket {
                    dts: pts,
                    pts,
                    duration: 33,
                    bytes: 16,
                    sha256: payload_hash.clone(),
                })
                .collect(),
        };
        compare_packets(&source, &flv)?;
        compare_decoded(&source, &flv, &decoded)?;
        let mut changed = flv.clone();
        changed[1].dts_ms += 2;
        require(
            compare_packets(&source, &changed).is_err(),
            "checker_accepts_2ms_dts_change",
        )?;
        changed = flv.clone();
        changed[1].pts_ms += 2;
        require(
            compare_packets(&source, &changed).is_err(),
            "checker_accepts_2ms_pts_change",
        )?;
        changed = flv.clone();
        changed[1].sha256 = sha(&[4, 3, 2, 1]);
        require(
            compare_packets(&source, &changed).is_err(),
            "checker_accepts_changed_payload",
        )?;
        require(
            compare_packets(&source, &flv[1..]).is_err(),
            "checker_accepts_first_packet_drop",
        )?;
        changed = flv.clone();
        changed[1] = changed[0].clone();
        require(
            compare_packets(&source, &changed).is_err(),
            "checker_accepts_packet_duplicate",
        )?;
        let mut gap = source.clone();
        gap.rows[1].dts += 1;
        require(
            compare_packets(&gap, &flv).is_err(),
            "checker_accepts_source_gap",
        )?;
        let mut bad_decoded = decoded.clone();
        bad_decoded.rows[1].pts += 2;
        require(
            compare_decoded(&source, &flv, &bad_decoded).is_err(),
            "checker_accepts_decoded_shift",
        )?;
        bad_decoded = decoded.clone();
        bad_decoded.rows.remove(0);
        require(
            compare_decoded(&source, &flv, &bad_decoded).is_err(),
            "checker_accepts_decoded_drop",
        )?;
        bad_decoded = decoded.clone();
        bad_decoded.rows[1] = bad_decoded.rows[0].clone();
        require(
            compare_decoded(&source, &flv, &bad_decoded).is_err(),
            "checker_accepts_decoded_duplicate",
        )?;
        bad_decoded = decoded.clone();
        bad_decoded.rows[2].duration += 2;
        require(
            compare_full_decoders(&decoded, &bad_decoded).is_err(),
            "checker_accepts_end_shift",
        )?;
        compare_full_decoders(&decoded, &decoded)?;
        bad_decoded = decoded.clone();
        bad_decoded.rows[1].sha256 = sha(&[9, 8, 7, 6]);
        require(
            compare_full_decoders(&decoded, &bad_decoded).is_err(),
            "checker_accepts_decoded_payload_change",
        )?;
        bad_decoded = decoded.clone();
        bad_decoded.rows[1].pts += 2;
        require(
            compare_full_decoders(&decoded, &bad_decoded).is_err(),
            "checker_accepts_baseline_pts_shift",
        )?;
        bad_decoded = decoded.clone();
        bad_decoded.rows.remove(0);
        require(
            compare_full_decoders(&decoded, &bad_decoded).is_err(),
            "checker_accepts_baseline_drop",
        )?;
        for boundary in [12_000u32, 24_000] {
            let boundary_source = HashStream {
                base: 0.001,
                rows: [boundary - 50, boundary]
                    .into_iter()
                    .map(|pts| HashPacket {
                        dts: i64::from(pts),
                        pts: i64::from(pts),
                        duration: 50,
                        bytes: 4,
                        sha256: payload_hash.clone(),
                    })
                    .collect(),
            };
            let mut boundary_flv: Vec<_> = [boundary - 50, boundary]
                .into_iter()
                .map(|pts| FlvPacket {
                    dts_ms: pts,
                    pts_ms: i64::from(pts),
                    bytes: 4,
                    sha256: payload_hash.clone(),
                })
                .collect();
            compare_packets(&boundary_source, &boundary_flv)?;
            boundary_flv[1].dts_ms = 0;
            boundary_flv[1].pts_ms = 0;
            require(
                compare_packets(&boundary_source, &boundary_flv).is_err(),
                "checker_accepts_boundary_reset",
            )?;
        }
        let row = format!("0,0,0,1,4,{payload_hash}\n1,0,0,1,4,{payload_hash}\n");
        for header in [
            "#tb 0: 1/1000\n#tb 0: 1/1000\n#tb 1: 1/48000\n",
            "#tb 0: NaN/1\n#tb 1: 1/48000\n",
            "#tb 0: 0/1\n#tb 1: 1/48000\n",
            "#tb 0: 1/0\n#tb 1: 1/48000\n",
            "#tb 0: 1/1000\n",
        ] {
            require(
                parse_hashes(format!("{header}{row}").as_bytes(), true).is_err(),
                "checker_accepts_invalid_timebase_header",
            )?;
        }
        require(
            parse_hashes(&vec![0; 2 * 1024 * 1024 + 1], true).is_err(),
            "checker_accepts_oversize_framehash",
        )?;
        Ok(())
    }
    fn decode_media(
        exe: &Path,
        cwd: &Path,
        format: &'static str,
        input: Arc<Vec<u8>>,
    ) -> Check<process::Output> {
        require(matches!(format, "mov" | "flv"), "fixed_decode_format")?;
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
            Some(input),
            2 * 1024 * 1024,
            false,
        )
    }
    fn experiment(exe: &Path, cwd: &Path, progress: &mut Value) -> Check<Value> {
        timing_rule_checks()?;
        preservation_rule_checks()?;
        progress["timingCheckerSelfChecksPassed"] = json!(true);
        progress["stage"] = json!("tool_version");
        let version = process::run(exe, cwd, &["-hide_banner", "-version"], None, 65536, false)?;
        progress["ffmpegVersion"] = json!(String::from_utf8_lossy(&version.bytes).lines().next());
        progress["stage"] = json!("generate_real_fixture");
        let generator = process::run(
            exe,
            cwd,
            &[
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
                "-protocol_whitelist",
                "pipe",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=30",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=997:sample_rate=48000",
                "-t",
                "30",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:v",
                "libopenh264",
                "-threads",
                "2",
                "-pix_fmt",
                "yuv420p",
                "-g",
                "60",
                "-bf",
                "0",
                "-rc_mode",
                "bitrate",
                "-allow_skip_frames",
                "0",
                "-b:v",
                "800k",
                "-maxrate",
                "800k",
                "-bufsize",
                "1600k",
                "-c:a",
                "aac",
                "-b:a",
                "96k",
                "-ar",
                "48000",
                "-ac",
                "2",
                "-movflags",
                "frag_keyframe+empty_moov+default_base_moof",
                "-f",
                "mp4",
                "pipe:1",
            ],
            None,
            process::MEDIA_LIMIT,
            false,
        )?;
        require(
            generator.bytes.len() > 1024 * 1024,
            "fixture_too_small_for_backpressure",
        )?;
        let input_bytes = generator.bytes.len();
        let input_sha = sha(&generator.bytes);
        progress["generator"] = generator.facts.clone();
        progress["inputBytes"] = json!(input_bytes);
        progress["inputSha256"] = json!(input_sha);
        let input = Arc::new(generator.bytes);
        progress["stage"] = json!("inspect_source_packet_timing");
        let source_packets = process::run(
            exe,
            cwd,
            &[
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
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
                "-copyts",
                "-c",
                "copy",
                "-f",
                "framehash",
                "-hash",
                "sha256",
                "pipe:1",
            ],
            Some(input.clone()),
            2 * 1024 * 1024,
            false,
        )?;
        progress["sourcePacketProcess"] = source_packets.facts.clone();
        progress["sourcePacketHead"] = decoded_head(&source_packets.bytes)?;
        progress["sourcePacketFramehashBytes"] = json!(source_packets.bytes.len());
        progress["sourcePacketFramehashSha256"] = json!(sha(&source_packets.bytes));
        let source = parse_hashes(&source_packets.bytes, true)?;
        require(
            source[0].rows.len() == 900 && source[1].rows.len() >= 1000,
            "source_packet_count",
        )?;
        drop(source_packets);
        progress["stage"] = json!("decode_original_source_baseline");
        let source_decode = decode_media(exe, cwd, "mov", input.clone())?;
        let source_decode_facts = source_decode.facts.clone();
        progress["sourceDecodeProcess"] = source_decode_facts.clone();
        progress["sourceDecodedHead"] = decoded_head(&source_decode.bytes)?;
        progress["sourceDecodedFramehashBytes"] = json!(source_decode.bytes.len());
        progress["sourceDecodedFramehashSha256"] = json!(sha(&source_decode.bytes));
        let source_decoded = parse_hashes(&source_decode.bytes, false)?;
        drop(source_decode);
        progress["stage"] = json!("remux_to_flv");
        let remux = process::run(
            exe,
            cwd,
            REMUX,
            Some(input.clone()),
            process::MEDIA_LIMIT,
            false,
        )?;
        progress["remux"] = remux.facts.clone();
        progress["stage"] = json!("inspect_flv_tags");
        let tags = inspect_flv(&remux.bytes)?;
        progress["flv"] = tags.facts.clone();
        progress["stage"] = json!("compare_source_to_flv");
        progress["sourceToFlv"] = json!([
            compare_packets(&source[0], &tags.packets[0])?,
            compare_packets(&source[1], &tags.packets[1])?,
        ]);
        progress["stage"] = json!("decode_flv");
        let decode = decode_media(exe, cwd, "flv", Arc::new(remux.bytes))?;
        progress["decodeProcess"] = decode.facts.clone();
        progress["framehashBytes"] = json!(decode.bytes.len());
        progress["framehashSha256"] = json!(sha(&decode.bytes));
        progress["decodedHead"] = decoded_head(&decode.bytes)?;
        progress["stage"] = json!("inspect_decoded_timing");
        let decoded = inspect_decode(&decode.bytes, &source, &tags, &source_decoded)?;
        progress["decoded"] = decoded.clone();
        progress["stage"] = json!("slow_consumer_cancel");
        let slow = process::run(exe, cwd, REMUX, Some(input), process::MEDIA_LIMIT, true)?;
        progress["stage"] = json!("finished");
        Ok(
            json!({"passed":true,"ffmpegVersion":String::from_utf8_lossy(&version.bytes).lines().next(),
            "fixture":{"seconds":30,"video":"real CPU libopenh264, bf0; B frames unverified","audio":"real AAC from sine","inputBytes":input_bytes,"inputSha256":input_sha},
            "flv":tags.facts,"decode":decoded,"processes":{"generator":generator.facts,"sourceDecode":source_decode_facts,"remux":remux.facts,"decode":decode.facts,"slowConsumer":slow.facts},
            "networkRequests":0,"accountActions":0,"userMediaRead":0,"productionParserUsed":false,
            "notVerified":["Container trimming/presentation-end metadata preservation (separate container-to-decoder endpoint deltas are reported)","B-frame composition/conversion","production MSE/parser integration","original live viewing latency","RTMPS","YouTube archive sync","live chat/donations","rollover/reconnect"]}),
        )
    }
    pub fn main() -> Check<()> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        require(
            args.len() == 2 && args[0] == "--ffmpeg",
            "usage: relay_pipe_qa --ffmpeg ABSOLUTE_PATH",
        )?;
        let executable = PathBuf::from(&args[1]);
        require(executable.is_absolute(), "absolute_tool_required")?;
        let executable = executable.canonicalize().map_err(|_| "tool_path")?;
        require(
            executable
                .file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("ffmpeg.exe")),
            "ffmpeg_executable_required",
        )?;
        let tool_sha = digest_file(&executable)?;
        let project = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("project_root")?;
        let root = project.join(".runtime/requirements-audit/20261117-cloud-goal-resume");
        let directory = root.join(format!(
            "relay-pipe-offline-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir(&directory).map_err(|_| "audit_create_new")?;
        let work = directory.join("empty-child-work");
        fs::create_dir(&work).map_err(|_| "work_create_new")?;
        let mut progress = json!({});
        let outcome = experiment(&executable, &work, &mut progress);
        let files: Vec<_> = fs::read_dir(&work)
            .map_err(|_| "work_inspection")?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "work_inspection")?;
        let no_files = files.is_empty();
        let result = match outcome {
            Ok(value) => value,
            Err(error) => json!({"passed":false,"error":error}),
        };
        let passed = result["passed"] == true && no_files;
        let report = json!({"result":result,"progress":progress,"noChildFilesCreated":no_files,"childEntryCount":files.len(),"ffmpegSha256":tool_sha,
            "classification":"offline pipe experiment; not shipping relay acceptance","productionChanges":0});
        let output = directory.join("result.json");
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output)
            .map_err(|_| "result_create_new")?;
        serde_json::to_writer_pretty(&mut file, &report).map_err(|_| "result_write")?;
        file.write_all(b"\n").map_err(|_| "result_write")?;
        println!("{}", output.display());
        require(passed, "offline_pipe_experiment_failed")
    }
}
#[cfg(windows)]
fn main() {
    if let Err(error) = qa::main() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!("This bounded process experiment requires Windows Job Objects.");
    std::process::exit(2);
}
