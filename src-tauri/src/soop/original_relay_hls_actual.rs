//! Opt-in actual source/manager/chat trial, with a localhost destination only.
use super::{
    facts, lock, video_files, AppHandle, AppQuitRequest, AppState, SoopOriginalHost,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::Manager;
use crate::cloud_recording::live_relay::{LiveRelayCoordinator, NativeDestinationOwner, PreparedLiveDestination};
#[path = "../../examples/support/relay_pipe_process.rs"]
mod process;
#[path = "original_relay_hls_server.rs"]
mod server;
#[path = "original_relay_event_clock_actual.rs"]
mod event_clock;

fn selected_start(
    app: &AppHandle, host: &SoopOriginalHost, token: &str,
    target: crate::streaming::relay_hls::HlsTarget,
    selections: &Mutex<Vec<Arc<LiveRelayCoordinator>>>,
) -> Result<(String, Option<Arc<LiveRelayCoordinator>>), String> {
    if std::env::var("ATSUMI_SOOP_HLS_QA_SELECTION").as_deref() != Ok("1") {
        return host.start_hls_relay(app, token, target).map(|run| (run, None));
    }
    let selected = Arc::new(LiveRelayCoordinator::select(host.select_relay_source(token)?));
    let owner = NativeDestinationOwner { channel_id: "UCabcdefghijklmnopqrstuv".into(), generation: 1 };
    let destination = PreparedLiveDestination::loopback_test(owner.clone(), target);
    let state = selected.start(app, host, &owner, destination).map_err(|_| "qa_selected_start")?;
    let run = state.run_id.ok_or("qa_selected_run")?;
    lock(selections).push(selected.clone());
    Ok((run, Some(selected)))
}

fn quiet_video(app: &AppHandle, arm: bool) -> Result<Value, String> {
    let views: Vec<_> = app.webviews().into_values()
        .filter(|v| v.label().starts_with("soop-original-")).collect();
    require(views.len()==1, "qa_quiet_receiver")?;
    let script=format!(r#"(()=>{{const v=document.getElementById('livePlayer');if(!v)return null;
      if({arm}&&window.__atsumiHlsQaQuiet!==v){{const quiet=()=>{{if(v.volume!==0)v.volume=0;if(!v.muted)v.muted=true}};
      quiet();v.addEventListener('volumechange',quiet);window.__atsumiHlsQaQuiet=v;}}
      return {{muted:v.muted,volume:v.volume,bound:window.__atsumiHlsQaQuiet===v}};}})()"#);
    let (tx,rx)=std::sync::mpsc::sync_channel(1);
    views[0].eval_with_callback(&script,move|v|{let _=tx.send(v);}).map_err(|_|"qa_quiet_dispatch")?;
    let text=rx.recv_timeout(Duration::from_secs(3)).map_err(|_|"qa_quiet_timeout")?;
    require(text.len()<1024,"qa_quiet_bound")?;
    let result:Value=serde_json::from_str(&text).map_err(|_|"qa_quiet_shape")?;
    require(result["muted"]==true&&result["volume"]==0&&result["bound"]==true,"qa_audio_not_quiet")?;
    Ok(result)
}
fn window_ms(app:&AppHandle, milliseconds:u64)->Result<Value,String>{
    let quiet_before=quiet_video(app,false)?;
    let before=facts(app)?;
    let began=Instant::now();
    thread::sleep(Duration::from_millis(milliseconds));
    let after=facts(app)?;
    let quiet_after=quiet_video(app,false)?;
    let progressed=after["time"].as_f64().zip(before["time"].as_f64()).is_some_and(|(a,b)|a>b+1.)
        &&after["frames"].as_u64().zip(before["frames"].as_u64()).is_some_and(|(a,b)|a>b)
        &&before["rate"]==1.0&&after["rate"]==1.0;
    Ok(json!({"before":before,"after":after,"progressed":progressed,"requestedMs":milliseconds,
        "measuredMs":began.elapsed().as_millis(),"quietBefore":quiet_before,"quietAfter":quiet_after}))
}
fn window(app:&AppHandle,seconds:u64)->Result<Value,String>{window_ms(app,seconds*1000)}

// One bounded, readonly source/mute observation. Its callback wait uses the
// remaining trial deadline, rather than extending a25s readiness wait by3s.
fn second_source_sample(app: &AppHandle, deadline: Instant) -> Result<Value, String> {
    let views: Vec<_> = app.webviews().into_values()
        .filter(|view| view.label().starts_with("soop-original-")).collect();
    require(views.len() == 1, "qa_second_receiver")?;
    let remaining = deadline.saturating_duration_since(Instant::now()).min(Duration::from_secs(3));
    require(!remaining.is_zero(), "qa_second_readiness_timeout")?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    views[0].eval_with_callback(r#"(()=>{const v=document.getElementById('livePlayer');const q=v?.getVideoPlaybackQuality?.();return {time:v?.currentTime,ready:v?.readyState,paused:v?.paused,rate:v?.playbackRate,frames:q?.totalVideoFrames,dropped:q?.droppedVideoFrames,muted:v?.muted,volume:v?.volume,bound:window.__atsumiHlsQaQuiet===v,relay:window.__atsumiSoopEncodedCapture?.getRelayStatus?.()}})()"#,
        move |value| { let _ = tx.send(value); }).map_err(|_| "qa_second_sample_dispatch")?;
    let text = rx.recv_timeout(remaining).map_err(|_| "qa_second_sample_timeout")?;
    require(text.len() <= 2048, "qa_second_sample_bound")?;
    serde_json::from_str(&text).map_err(|_| "qa_second_sample_shape".into())
}

fn wait_second_ready(
    app: &AppHandle, host: &SoopOriginalHost, token: &str, run: &str, report: &mut Value,
) -> Result<(), String> {
    let began = Instant::now();
    let deadline = began + Duration::from_secs(25);
    report["secondRun"] = json!(run);
    report["secondReadiness"] = json!({"maximumWaitMs":25000,"pollMs":200,
        "sourceSampleMs":1000,"requiredSegmentAcks":2,"samples":[]});
    let before = second_source_sample(app, deadline)?;
    let mut latest = before.clone();
    let mut next_sample = Instant::now();
    let mut sampled_acks = 0;
    loop {
        let status = host.relay_delivery_status(token)?.ok_or("qa_second_missing")?;
        let acks = status.delivery.as_ref().map_or(0, |delivery| delivery.segment_acks);
        let active = status.run_id == run && !status.manager_joined && !status.interrupted
            && status.fault.is_none() && status.input_fault.is_none()
            && status.delivery.as_ref().is_none_or(|delivery| delivery.fault.is_none() && !delivery.end_list_acked);
        report["secondBeforeQuit"] = serde_json::to_value(&status).map_err(|_| "qa_second_status")?;
        report["secondReadiness"]["measuredMs"] = json!(began.elapsed().as_millis());
        require(active, "qa_second_relay_ended_before_ready")?;
        if Instant::now() >= next_sample || (acks >= 2 && sampled_acks < 2) {
            latest = second_source_sample(app, deadline)?;
            let samples = report["secondReadiness"]["samples"].as_array_mut().ok_or("qa_second_samples")?;
            require(samples.len() < 28, "qa_second_samples_bound")?;
            samples.push(json!({"elapsedMs":began.elapsed().as_millis(),"segmentAcks":acks,"source":latest}));
            sampled_acks = acks;
            next_sample = Instant::now() + Duration::from_secs(1);
        }
        let progressed = latest["time"].as_f64().zip(before["time"].as_f64()).is_some_and(|(a,b)| a > b + 1.)
            && latest["frames"].as_u64().zip(before["frames"].as_u64()).is_some_and(|(a,b)| a > b)
            && before["rate"] == 1.0 && latest["rate"] == 1.0;
        report["beforeQuit"] = json!({"before":before,"after":latest,"progressed":progressed,
            "maximumWaitMs":25000,"measuredMs":began.elapsed().as_millis(),
            "quietBefore":{"muted":before["muted"],"volume":before["volume"],"bound":before["bound"]},
            "quietAfter":{"muted":latest["muted"],"volume":latest["volume"],"bound":latest["bound"]}});
        require(before["muted"] == true && before["volume"] == 0 && before["bound"] == true
            && latest["muted"] == true && latest["volume"] == 0 && latest["bound"] == true, "qa_audio_not_quiet")?;
        require(latest["relay"]["runId"] == run, "qa_second_source_run")?;
        if acks >= 2 && status.codec_admitted && progressed && latest["paused"] == false
            && latest["ready"].as_u64().is_some_and(|ready| ready >= 2)
            && latest["relay"]["active"] == true && latest["relay"]["nativeApproved"] == true
        {
            // DOM callback may have taken time. Do not use the pre-callback
            // native snapshot to certify an owner which has since failed/ended.
            let final_status = host.relay_delivery_status(token)?.ok_or("qa_second_missing")?;
            let still_ready = final_status.run_id == run && final_status.codec_admitted
                && !final_status.manager_joined && !final_status.interrupted
                && final_status.fault.is_none() && final_status.input_fault.is_none()
                && final_status.delivery.as_ref().is_some_and(|delivery|
                    delivery.segment_acks >= 2 && delivery.fault.is_none() && !delivery.end_list_acked);
            report["secondBeforeQuit"] = serde_json::to_value(&final_status).map_err(|_| "qa_second_status")?;
            report["secondReadiness"]["measuredMs"] = json!(began.elapsed().as_millis());
            require(still_ready, "qa_second_relay_ended_before_ready")?;
            require(Instant::now() <= deadline, "qa_second_readiness_timeout")?;
            report["secondReadiness"]["ready"] = json!(true);
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        require(!remaining.is_zero(), "qa_second_readiness_timeout")?;
        thread::sleep(remaining.min(Duration::from_millis(200)));
    }
}

fn require(value: bool, reason: &str) -> Result<(), String> {
    if value {
        Ok(())
    } else {
        Err(reason.into())
    }
}
fn no_links(path: &Path) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    for part in path.ancestors() {
        let meta = fs::symlink_metadata(part).map_err(|_| "qa_path_metadata")?;
        require(
            !meta.file_type().is_symlink() && meta.file_attributes() & 0x400 == 0,
            "qa_reparse",
        )?;
    }
    Ok(())
}
fn bounded_json(path: &Path) -> Result<Value, String> {
    no_links(path)?;
    require(
        fs::metadata(path).map_err(|_| "qa_json_metadata")?.len() <= 64 * 1024,
        "qa_json_bound",
    )?;
    serde_json::from_slice(&fs::read(path).map_err(|_| "qa_json_read")?)
        .map_err(|_| "qa_json_shape".into())
}
fn event_evidence(directory: &Path) -> Result<(Vec<String>, Vec<Value>), String> {
    no_links(directory)?;
    let mut hashes = Vec::new();
    let mut controls = Vec::new();
    let mut physical_rows = 0;
    let mut parts = Vec::new();
    for item in fs::read_dir(directory).map_err(|_| "qa_chat_dir")? {
        let path = item.map_err(|_| "qa_chat_entry")?.path();
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or("qa_chat_name")?;
        if !name.starts_with("chat.events") || !name.ends_with(".jsonl") {
            continue;
        }
        require(parts.len() < 4, "qa_chat_parts_bound")?;
        let sequence = if name == "chat.events.jsonl" {
            0
        } else {
            let digits = name
                .strip_prefix("chat.events.")
                .and_then(|s| s.strip_suffix(".jsonl"))
                .ok_or("qa_part_name")?;
            require(
                digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_digit()),
                "qa_part_sequence",
            )?;
            let value = digits.parse::<u32>().map_err(|_| "qa_part_sequence")?;
            require(value > 0, "qa_part_zero_alias")?;
            value
        };
        parts.push((sequence, path));
    }
    parts.sort_by_key(|(sequence, _)| *sequence);
    for (sequence, path) in parts.into_iter().enumerate() {
        require(sequence as u32 == path.0, "qa_part_gap")?;
        let path = path.1;
        no_links(&path)?;
        require(
            fs::metadata(&path).map_err(|_| "qa_chat_metadata")?.len() <= 8 * 1024 * 1024,
            "qa_chat_bytes",
        )?;
        for line in fs::read(&path)
            .map_err(|_| "qa_chat_read")?
            .split(|b| *b == b'\n')
            .filter(|b| !b.is_empty())
        {
            require(physical_rows < 20_000, "qa_chat_rows_bound")?;
            physical_rows += 1;
            let value: Value = serde_json::from_slice(line).map_err(|_| "qa_chat_json")?;
            if value["format"] == "atsumi-soop-display-v1" {
                require(line.len() <= 2048 && value["documentEpoch"].as_u64().unwrap_or(0)>0
                    && value["sequence"].is_u64() && value["initial"].is_boolean()
                    && value["values"].as_object().is_some_and(|v|v.len()==7)
                    && value["basis"].as_object().is_some_and(|v|v.len()==7), "qa_display_control")?;
                controls.push(json!({"sha256":format!("{:x}",Sha256::digest(line)),
                    "initial":value["initial"],"documentEpoch":value["documentEpoch"],"sequence":value["sequence"],
                    "values":value["values"],"basis":value["basis"]}));
                continue;
            }
            hashes.push(format!("{:x}", Sha256::digest(line)));
        }
    }
    Ok((hashes, controls))
}
fn ordered_subset(archive: &[String], relay: &[String]) -> bool {
    let mut cursor = 0;
    for row in relay {
        if archive.get(cursor) == Some(row) {
            cursor += 1;
        }
    }
    cursor == archive.len()
}
fn timing_facts(directory:&Path,owner:&Value,summary:&Value)->Result<Value,String> {
    let path=directory.join("relay-timing.jsonl");no_links(&path)?;
    require(fs::metadata(&path).map_err(|_|"qa_timing_metadata")?.len()<=2*1024*1024,"qa_timing_bound")?;
    let bytes=fs::read(path).map_err(|_|"qa_timing_read")?;
    let mut rows=0u64;let mut known=0u64;let mut unknown=0u64;let mut segments=0u64;let mut end_count=0;
    let mut stream=Value::Null;let mut previous_end=None;let mut previous_origin=Value::Null;
    for line in bytes.split(|b|*b==b'\n').filter(|b|!b.is_empty()) {
        require(line.len()<=4096,"qa_timing_row_bound")?;
        let row:Value=serde_json::from_slice(line).map_err(|_|"qa_timing_shape")?;
        require(row["version"]==1&&row["sequence"]==rows&&row["binding"]==owner["binding"],"qa_timing_binding_order")?;
        rows+=1;let data=&row["data"];
        let before=data["nativeBeforeNs"].as_u64().ok_or("qa_timing_before")?;
        let after=data["nativeAfterNs"].as_u64().ok_or("qa_timing_after")?;
        require(after>=before,"qa_timing_bracket")?;
        match row["kind"].as_str() {
            Some("source_observation")=>{
                if data["usable"]==true {
                    let sample=&data["sample"];
                    let source=sample["sourceTimeSeconds"].as_f64().ok_or("qa_source_clock")?;
                    let current=sample["currentTimeSeconds"].as_f64().ok_or("qa_source_current")?;
                    let offset=sample["timestampOffsetSeconds"].as_f64().ok_or("qa_source_offset")?;
                    require(after-before<=3_000_000_000&&(current-offset-source).abs()<=1e-6&&sample["paused"]==false&&sample["seeking"]==false,"qa_source_observed")?;
                    known+=1;
                }else{unknown+=1;}
                require(data["archiveMapping"].is_null(),"qa_no_fabricated_archive_clock")?;
            },
            Some("observation_missing")=>unknown+=1,
            Some("segment_ack")=>{
                require(data["ordinal"]==segments&&end_count==0,"qa_timing_segment_order")?;
                if segments==0{stream=data["streamId"].clone();}
                require(data["streamId"]==stream,"qa_timing_stream")?;
                let t=&data["timing"];let start=t["segmentStart90k"].as_u64().ok_or("qa_segment_start")?;
                let end=t["segmentEnd90k"].as_u64().ok_or("qa_segment_end")?;
                require(end>start&&end-start<=450_000&&previous_end.is_none_or(|v|v==start),"qa_segment_interval")?;
                let origin=json!([t["originTicks"],t["originTimescale"]]);
                if segments>0{require(origin==previous_origin,"qa_single_origin")?;}previous_origin=origin;previous_end=Some(end);
                for track in ["video","audio"] {
                    let x=&t[track];require(x["samples"].as_u64().is_some_and(|v|v>0)&&x["timescale"].as_u64().is_some_and(|v|v>0)
                        &&x["lastDtsEnd"].as_u64()>x["firstDts"].as_u64()&&x["maxPtsEnd"].as_u64()>x["minPts"].as_u64(),"qa_source_range")?;
                }
                require(data["archiveMapping"].is_null(),"qa_ack_not_archive_origin")?;segments+=1;
            },
            Some("end_ack")=>{require(data["streamId"]==stream&&data["segments"]==segments&&segments>0,"qa_matching_sealed_end")?;end_count+=1;},
            _=>return Err("qa_timing_kind".into())
        }
    }
    require(known>0&&segments>0&&end_count==1&&summary["timingRecords"]==rows,"qa_actual_timing_evidence")?;
    Ok(json!({"records":rows,"usableSourceObservations":known,"unknownObservations":unknown,"segmentAcknowledgements":segments,
        "matchingEndAcknowledgements":end_count,"sha256":format!("{:x}",Sha256::digest(&bytes)),"timingIncomplete":summary["timingIncomplete"],
        "qualification":"Native observation brackets and sealed source/wire/ACK evidence only; no event interpolation or YouTube archive origin."}))
}
fn sidecar_facts(directory: &Path) -> Result<Value, String> {
    let summary = bounded_json(&directory.join("relay-summary.json"))?;
    let owner = bounded_json(&directory.join("relay-owner.json"))?;
    let timing=timing_facts(directory,&owner,&summary)?;
    let ledger = directory.join("relay-receive.jsonl");
    no_links(&ledger)?;
    let (events, controls) = event_evidence(directory)?;
    let assets = directory.join("chat-assets");
    let status = bounded_json(&assets.join("status.json"))?;
    let manifest = assets.join("manifest.jsonl");
    no_links(&manifest)?;
    require(
        fs::metadata(&manifest)
            .map_err(|_| "qa_asset_manifest")?
            .len()
            <= 1024 * 1024,
        "qa_asset_manifest_bound",
    )?;
    let mut stored = 0;
    for line in fs::read(&manifest)
        .map_err(|_| "qa_asset_manifest_read")?
        .split(|b| *b == b'\n')
        .filter(|b| !b.is_empty())
    {
        let row: Value = serde_json::from_slice(line).map_err(|_| "qa_asset_manifest_shape")?;
        let id = row["id"].as_str().ok_or("qa_asset_id")?;
        require(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "qa_asset_id_shape",
        )?;
        let path = assets.join(id);
        no_links(&path)?;
        require(
            fs::metadata(&path).map_err(|_| "qa_asset_metadata")?.len() <= 1024 * 1024,
            "qa_asset_bound",
        )?;
        let bytes = fs::read(&path).map_err(|_| "qa_asset_read")?;
        require(
            row["sha256"] == format!("{:x}", Sha256::digest(&bytes)) && row["bytes"] == bytes.len(),
            "qa_asset_hash",
        )?;
        let decoded = image::load_from_memory(&bytes).map_err(|_| "qa_asset_decode")?;
        require(
            decoded.width() > 0 && decoded.height() > 0,
            "qa_asset_dimensions",
        )?;
        stored += 1;
    }
    Ok(
        json!({"summary":summary,"runId":owner["binding"]["runId"],"eventHashes":events.len(),"displayControls":controls,
        "receiveLedgerBytes":fs::metadata(ledger).map_err(|_| "qa_ledger_metadata")?.len(),
        "assetStatus":status,"verifiedDecodedAssetFiles":stored,"videoFiles":video_files(directory)?,"timing":timing}),
    )
}

fn decode_segments(server: &server::Server, root: &Path) -> Result<Value, String> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let exe = workspace.join(
        ".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin/ffmpeg.exe",
    );
    no_links(&exe)?;
    require(
        format!(
            "{:x}",
            Sha256::digest(fs::read(&exe).map_err(|_| "qa_decoder_read")?)
        ) == "c2a51bdad17d321c9c11768a4bc6137ac6c666d62ba82fe4b4050fbd1db9d2a0",
        "qa_decoder_revision",
    )?;
    let exe = fs::canonicalize(exe).map_err(|_| "qa_decoder_path")?;
    let mut results = Vec::new();
    for bytes in server.segments() {
        let output = process::run(
            &exe,
            root,
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
                "mpegts",
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
            Some(bytes),
            2 * 1024 * 1024,
            false,
        )?;
        let text = std::str::from_utf8(&output.bytes).map_err(|_| "qa_decode_output")?;
        let video_frames = text
            .lines()
            .filter(|line| line.trim_start().starts_with("0,"))
            .count();
        let audio_frames = text
            .lines()
            .filter(|line| line.trim_start().starts_with("1,"))
            .count();
        require(
            video_frames > 0 && audio_frames > 0,
            "qa_both_streams_decoded",
        )?;
        let mut proof = output.facts;
        proof["videoDecodedFrames"] = json!(video_frames);
        proof["audioDecodedFrames"] = json!(audio_frames);
        require(
            proof["success"] == true && proof["stderrRetainedBytes"] == 0,
            "qa_live_ts_decode",
        )?;
        results.push(proof);
    }
    require(results.len() == 2, "qa_two_received_segments")?;
    Ok(json!(results))
}

fn exercise(
    app: &AppHandle,
    root: &Path,
    channel: String,
    server: &server::Server,
    report: &mut Value,
    selections: &Mutex<Vec<Arc<LiveRelayCoordinator>>>,
) -> Result<(), String> {
    let tail_ms=std::env::var("ATSUMI_SOOP_HLS_QA_TAIL_MS").ok()
        .map(|v|v.parse::<u64>().map_err(|_|"qa_tail_parameter")).transpose()?.unwrap_or(6000);
    require(matches!(tail_ms,5000|6000|6500|7250),"qa_tail_parameter")?;
    let startup = app.state::<Arc<crate::startup::Startup>>();
    crate::initialize_backend(app, &startup).map_err(|_| "backend_init")?;
    let state = app.state::<AppState>();
    let media = state.platform_media(app)?;
    let host = app.state::<SoopOriginalHost>();
    host.set_tray_muted(app, true);
    let token = host
        .open(
            app,
            media.clone(),
            channel,
            uuid::Uuid::new_v4().to_string(),
            root.join("media"),
        )?
        .token;
    host.viewport(app, &token, serde_json::from_value(json!({"sequence":1,"player":{"x":0,"y":0,"width":960,"height":720},"chat":{"x":970,"y":0,"width":320,"height":720},"visible":true,"muted":true,"occlusions":[]})).map_err(|_| "qa_viewport")?)?;
    let deadline = Instant::now() + Duration::from_secs(65);
    loop {
        let snapshot = host.snapshot(&token)?;
        if snapshot.capture_ready
            && snapshot.chat_visible
            && facts(app)?["ready"].as_u64().unwrap_or(0) >= 3
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err("source_not_ready".into());
        }
        thread::sleep(Duration::from_millis(500));
    }
    // This trial represents active viewing, while the viewport stays muted.
    // Tray mode deliberately prevents the app-owned resume action.
    host.set_tray_muted(app, false);
    host.resume(app, &token)?;
    report["quietPlaybackGuard"] = quiet_video(app, true)?;
    report["baseline"] = window(app, 4)?;
    require(report["baseline"]["progressed"] == true, "baseline_stalled")?;
    require(
        video_files(&root.join("media"))? == 0 && media.jobs(Some("soop")).is_empty(),
        "qa_nonempty_initial",
    )?;
    let before = state.active_work_snapshot().map_err(|_| "qa_work_before")?;
    let (first_run, first_selection) = selected_start(app, &host, &token, server.target(), selections)?;
    if let Some(selected) = &first_selection {
        report["firstSelectionStarted"] = serde_json::to_value(selected.status()).map_err(|_| "qa_selected_status")?;
    }
    report["relayOnly"] = window(app, 12)?;
    report["relayOnlyJobs"] = json!(media.jobs(Some("soop")).len());
    report["relayOnlyVideoFiles"] = json!(video_files(&root.join("media"))?);
    let during = state.active_work_snapshot().map_err(|_| "qa_work_during")?;
    report["receiverCount"] = json!(during.live_receivers.as_ref().map(|r| r.active_count));
    report["workRegistered"] = json!(during.work_set_fingerprint != before.work_set_fingerprint);
    let status = host
        .relay_delivery_status(&token)?
        .ok_or("qa_relay_missing")?;
    report["relayOnlyStatus"] = serde_json::to_value(status).map_err(|_| "qa_status")?;
    require(
        report["relayOnly"]["progressed"] == true
            && report["receiverCount"] == 1
            && report["relayOnlyJobs"] == 0
            && report["relayOnlyVideoFiles"] == 0
            && report["workRegistered"] == true
            && report["relayOnlyStatus"]["delivery"]["segmentAcks"]
                .as_u64()
                .unwrap_or(0)
                > 0,
        "qa_relay_only",
    )?;

    host.record(app, &token, true)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let job = loop {
        if let Some(job) = host.snapshot(&token)?.recording_job {
            break job;
        }
        if Instant::now() >= deadline {
            return Err("qa_archive_start".into());
        }
        thread::sleep(Duration::from_millis(250));
    };
    report["archiveOverlap"] = window(app, 8)?;
    let progress = host
        .relay_delivery_status(&token)?
        .ok_or("qa_relay_missing")?;
    let before_stop = serde_json::to_value(progress).map_err(|_| "qa_progress")?;
    host.record(app, &token, false)?;
    report["afterArchiveStop"] = window_ms(app, tail_ms)?;
    let progress = host
        .relay_delivery_status(&token)?
        .ok_or("qa_relay_missing")?;
    let after_stop = serde_json::to_value(progress).map_err(|_| "qa_progress")?;
    require(
        after_stop["delivery"]["segmentAcks"].as_u64().unwrap_or(0)
            > before_stop["delivery"]["segmentAcks"].as_u64().unwrap_or(0),
        "qa_relay_stopped_with_archive",
    )?;
    if let Some(selected) = &first_selection { selected.stop(false); }
    else { host.stop_hls_relay(app, &token, false)?; }
    let deadline = Instant::now() + Duration::from_secs(360);
    let first_status = loop {
        host.poll_relay_join(app);
        if let Some(status) = host.relay_delivery_status(&token)? {
            if status.manager_joined {
                break status;
            }
        }
        if Instant::now() >= deadline {
            return Err("qa_first_join_timeout".into());
        }
        thread::sleep(Duration::from_millis(50));
    };
    let sidecar = first_status
        .sidecar_directory
        .clone()
        .ok_or("qa_sidecar_path")?;
    no_links(&sidecar)?;
    require(
        fs::canonicalize(&sidecar)
            .map_err(|_| "qa_sidecar_canonical")?
            .starts_with(fs::canonicalize(root.join("media")).map_err(|_| "qa_media_canonical")?),
        "qa_sidecar_root",
    )?;
    report["firstRelay"] = serde_json::to_value(&first_status).map_err(|_| "qa_final_status")?;
    if let Some(selected) = &first_selection {
        report["firstSelectionJoined"] = serde_json::to_value(selected.status()).map_err(|_| "qa_selected_status")?;
        require(report["firstSelectionJoined"]["state"] == "joined"
            && report["firstSelectionJoined"]["managerJoined"] == true
            && report["firstSelectionJoined"]["archiveReady"] == false
            && report["firstSelectionJoined"]["runId"] == first_run, "qa_selected_first_join")?;
    }
    require(
        first_status.run_id == first_run
            && first_status.worker_joined
            && first_status.input_ended
            && !first_status.interrupted
            && report["firstRelay"]["delivery"]["endListAcked"] == true
            && report["firstRelay"]["sidecar"]["joined"] == true,
        "qa_first_relay_end",
    )?;
    report["firstSidecar"] = sidecar_facts(&sidecar)?;
    let deadline = Instant::now() + Duration::from_secs(50);
    let archive = loop {
        let row = media
            .jobs(Some("soop"))
            .into_iter()
            .find(|j| j.job_id == job)
            .ok_or("qa_archive_missing")?;
        if matches!(
            row.state.as_str(),
            "completed" | "failed" | "interrupted" | "cancelled"
        ) {
            break row;
        }
        if Instant::now() >= deadline {
            return Err("qa_archive_finish_timeout".into());
        }
        thread::sleep(Duration::from_millis(250));
    };
    let reopened = media
        .open(&job)
        .map(|p| {
            media.close(&p.token);
            true
        })
        .unwrap_or(false);
    let (recorded_rows, recorded_controls) = event_evidence(&root.join("media").join("SOOP").join(&job))?;
    let (relay_rows, relay_controls) = event_evidence(&sidecar)?;
    let shared_known_display = recorded_controls.iter().any(|recorded| {
        recorded["values"]["fontLevel"].is_u64() && relay_controls.iter().any(|relay|
            recorded["values"] == relay["values"] && recorded["basis"] == relay["basis"])
    });
    report["archive"] = json!({"state":archive.state,"reopened":reopened,"rows":recorded_rows.len(),
        "normalizedRowsInRelayInOrder":ordered_subset(&recorded_rows,&relay_rows),
        "displayControls":recorded_controls,"sharedKnownDisplay":shared_known_display,
        "appearanceTransitionVerified":false});
    require(
        archive.state == "completed"
            && reopened
            && !recorded_rows.is_empty()
            && ordered_subset(&recorded_rows, &relay_rows)
            && shared_known_display
            && report["archiveOverlap"]["progressed"] == true
            && report["afterArchiveStop"]["progressed"] == true,
        "qa_archive_isolation",
    )?;
    report["actualTsDecode"] = decode_segments(server, root)?;
    let (second_run, second_selection) = selected_start(app, &host, &token, server.target(), selections)?;
    if let Some(selected) = &second_selection {
        report["secondSelectionStarted"] = serde_json::to_value(selected.status()).map_err(|_| "qa_selected_status")?;
    }
    wait_second_ready(app, &host, &token, &second_run, report)?;
    report["quitWithActiveRelay"] = json!(true);
    Ok(())
}

#[test]
#[ignore = "Explicit isolated public SOOP source to localhost, independent chat/assets and normal shutdown"]
fn actual_soop_hls_same_receiver_sidecar_recording_overlap_and_quit() {
    assert_eq!(
        std::env::var("ATSUMI_SOOP_HLS_QA").as_deref(),
        Ok("public-source-loopback-only"),
        "QA_OPT_IN"
    );
    let channel = std::env::var("ATSUMI_SOOP_RELAY_QA_CHANNEL").expect("QA_CHANNEL_REQUIRED");
    assert!(
        !channel.is_empty()
            && channel.len() <= 40
            && channel
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')),
        "QA_CHANNEL"
    );
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string();
    let root = workspace.join(".runtime/soop-hls-native-qa").join(&run);
    fs::create_dir_all(root.parent().unwrap()).unwrap();
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("media")).unwrap();
    no_links(&root).unwrap();
    let identity = format!("local.atsumi.soophlsqa.{run}");
    let profile = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(&identity);
    assert!(!profile.exists(), "QA_PROFILE_EXISTS");
    let server = Arc::new(server::Server::start().unwrap());
    let worker_server = server.clone();
    let selections = Arc::new(Mutex::new(Vec::<Arc<LiveRelayCoordinator>>::new()));
    let worker_selections = selections.clone();
    let result = Arc::new(Mutex::new(
        json!({"passed":false,"isolated":true,"externalRelay":false}),
    ));
    let output = result.clone();
    let worker_root = root.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = identity;
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app = tauri::Builder::default()
        .any_thread()
        .manage(SoopOriginalHost::default())
        .manage(crate::youtube::original::YouTubeOriginalHost::default())
        .manage(Arc::new(crate::startup::Startup::default()))
        .manage(Arc::new(
            crate::ui_download_events::DownloadEvents::default(),
        ))
        .setup(move |app| {
            if app.path().app_data_dir()? != profile {
                return Err("QA_PROFILE_PATH".into());
            }
            tauri::window::WindowBuilder::new(app, "main")
                .title("Atsumi isolated relay validation")
                .inner_size(1290., 720.)
                .position(-16000., -16000.)
                .focused(false)
                .skip_taskbar(true)
                .build()?;
            let handle = app.handle().clone();
            thread::spawn(move || {
                let mut report = json!({"passed":false,"isolated":true,"externalRelay":false});
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    exercise(&handle, &worker_root, channel, &worker_server, &mut report, &worker_selections)
                }))
                .unwrap_or_else(|_| Err("qa_worker_panic".into()));
                report["passed"] = json!(outcome.is_ok());
                report["error"] = json!(outcome.err());
                *lock(&output) = report;
                if let Some(state) = handle.try_state::<AppState>() {
                    if let Ok(snapshot) = state.active_work_snapshot() {
                        if state
                            .request_graceful_quit(
                                handle.clone(),
                                AppQuitRequest {
                                    expected_work_set_fingerprint: snapshot.work_set_fingerprint,
                                    confirm_active_work: true,
                                    force_when_status_unknown: false,
                                },
                            )
                            .is_ok_and(|r| r.accepted)
                        {
                            return;
                        }
                    }
                    lock(&output)["passed"] = json!(false);
                    lock(&output)["quitError"] = json!("normal_quit_rejected");
                    let _ = fs::write(
                        worker_root.join("pending-result.json"),
                        serde_json::to_vec_pretty(&*lock(&output)).unwrap(),
                    );
                } else {
                    handle.exit(2);
                }
            });
            Ok(())
        })
        .build(context)
        .expect("QA_BUILD");
    app.run_return(|_, _| {});
    let mut report = lock(&result).clone();
    report["normalAppExitReturned"] = json!(true);
    report["http"] = server.snapshot();
    let final_check = (|| -> Result<(), String> {
        if std::env::var("ATSUMI_SOOP_HLS_QA_SELECTION").as_deref() == Ok("1") {
            let snapshots: Vec<_> = lock(&selections).iter().map(|selected| selected.status()).collect();
            report["selectedCoordinator"] = json!({"controlledDestination":true,"googleProvisioning":false,"snapshots":snapshots});
            let states = report["selectedCoordinator"]["snapshots"].as_array().ok_or("qa_selected_shape")?;
            require(states.len() == 2 && states[0]["selectionId"] != states[1]["selectionId"]
                && states[0]["runId"] != states[1]["runId"]
                && states.iter().all(|s| s["state"] == "joined" && s["managerJoined"] == true
                    && s["archiveReady"] == false && s["fault"].is_null()
                    && s["destination"]["visibility"] == "unlisted"
                    && s["relay"]["workerJoined"] == true
                    && s["relay"]["sidecar"]["joined"] == true
                    && s["relay"]["delivery"]["endListAcked"] == true), "qa_selected_both_joined")?;
        }
        require(report["http"]["error"].is_null(), "qa_http_error")?;
        let ended = report["http"]["objects"]
            .as_array()
            .ok_or("qa_http_objects")?
            .iter()
            .filter(|v| v["endList"] == true)
            .count();
        require(ended == 2, "qa_both_end_lists")?;
        let mut sidecars = Vec::new();
        for entry in
            fs::read_dir(root.join("media/SOOP/Relays")).map_err(|_| "qa_sidecars_missing")?
        {
            let directory = entry.map_err(|_| "qa_sidecar_entry")?.path();
            no_links(&directory)?;
            require(sidecars.len() < 2, "qa_sidecar_count_bound")?;
            let facts = sidecar_facts(&directory)?;
            // The persisted writer summary cannot certify its own JoinHandle.
            // Actual join is required by the normal AppState drain before exit.
            require(
                facts["summary"]["interrupted"] == false
                    && facts["videoFiles"] == 0
                    && facts["summary"]["textState"] == "complete",
                "qa_sidecar_exit",
            )?;
            sidecars.push(facts);
        }
        require(sidecars.len() == 2, "qa_two_sidecars")?;
        report["sidecarsAfterQuit"] = json!(sidecars);
        report["sidecarJoinEvidence"] = json!("First runtime status plus actual normal-work-gate exit; stored writer summaries are not join receipts.");
        Ok(())
    })();
    if let Err(error) = final_check {
        report["passed"] = json!(false);
        report["finalError"] = json!(error);
    }
    report["qualifications"] = json!(["Actual public source and localhost only; no YouTube ingestion or archive-clock mapping.","A short playback-progress check is not a universal latency/buffering measurement.","Only the deliberately created overlap recording writes video; relay-only phases write text/assets, no media jobs.","No existing user profile or media was opened, reset, moved or deleted."]);
    fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("QA_SOOP_HLS_RESULT={}", root.join("result.json").display());
    assert_eq!(
        report["passed"], true,
        "Actual SOOP HLS manager flow remains unverified; inspect bounded report"
    );
}
