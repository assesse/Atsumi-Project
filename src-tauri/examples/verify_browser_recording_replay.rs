//! Verify one recovered recording through the actual in-app replay service.
//! Requires an isolated, single-entry catalog (not the user's whole app data).
//! Does not start WebView2, recording workers, network requests or playback.
//! Only disposable replay indexes are written; chat bodies/tokens are not logged.
//! cargo run --example verify_browser_recording_replay -- ISOLATED_DATA_DIR ID
use atsumi_lib::streaming::{
    browser_store::{BrowserCaptureStore, BrowserMergeStatus},
    replay::ReplayService,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::http::{header, Request, StatusCode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2
        || args[1].len() != 32
        || !args[1]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("Expected ISOLATED_DATA_DIR RECORDING_ID".into());
    }
    let data = fs::canonicalize(PathBuf::from(&args[0]))?;
    let catalog = data.join("streaming/browser/catalog.jsonl");
    if fs::metadata(&catalog)?.len() > 8192 {
        return Err("Expected isolated single-entry catalog".into());
    }
    let rows: Vec<Value> = fs::read_to_string(catalog)?
        .lines()
        .filter(|line| !line.is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    if rows.len() != 1 || rows[0]["id"].as_str() != Some(&args[1]) {
        return Err("Unexpected recording scope".into());
    }
    let store = Arc::new(Mutex::new(BrowserCaptureStore::new(&data)?));
    let record = store.lock().unwrap().snapshot()?.remove(0);
    if record
        .merge
        .as_ref()
        .is_none_or(|m| m.status != BrowserMergeStatus::Complete)
    {
        return Err("Recording has no verified final output".into());
    }
    let media = store.lock().unwrap().merged_file(&args[1])?;
    let bytes = fs::metadata(media)?.len();
    let service = ReplayService::new(&data, store);
    let session = service.open(&args[1])?;
    let outcome = (|| -> Result<(), Box<dyn std::error::Error>> {
        if bytes < 4096 || session.recording_active || session.duration_seconds <= 0.0 {
            return Err("Unexpected replay descriptor".into());
        }
        for start in [0, bytes / 2, bytes - 1024] {
            let request = Request::builder()
                .uri(format!("http://atsumi-replay.localhost/{}", session.token))
                .header(header::RANGE, format!("bytes={start}-{}", start + 1023))
                .body(Vec::new())?;
            let response = service.media_response(&request, "main");
            if response.status() != StatusCode::PARTIAL_CONTENT || response.body().len() != 1024 {
                return Err("In-app media range failed".into());
            }
        }
        let started = Instant::now();
        let timeline = loop {
            let timeline = service.timeline(&session.token, Some(60.0))?;
            if timeline.index_state != "building" {
                break timeline;
            }
            if started.elapsed() > Duration::from_secs(120) {
                return Err("Replay index deadline".into());
            }
            thread::sleep(Duration::from_millis(100));
        };
        if timeline.index_state == "failed" {
            return Err("Replay index failed".into());
        }
        let mut samples = Vec::new();
        for (generation, at) in [
            0.0,
            session.duration_seconds / 2.0,
            session.duration_seconds - 1.0,
        ]
        .into_iter()
        .enumerate()
        {
            let page = service.chat_at(&session.token, at, generation as u64 + 1, Some(20))?;
            samples.push(json!({"atSeconds":at,"rows":page.items.len(),"indexState":page.index_state,"syncQuality":page.sync_quality}));
        }
        println!(
            "{}",
            json!({"recordingId":record.id,"durationSeconds":session.duration_seconds,
            "bytes":bytes,"rangeChecks":3,"channelProfileAvailable":session.channel_profile_image.is_some(),
            "chatStatus":session.chat_status,"indexState":timeline.index_state,"viewerMetricStatus":timeline.viewer_metric_status,
            "chatCount":timeline.buckets.iter().map(|b| b.chat_count).sum::<u64>(),
            "viewerSamples":timeline.buckets.iter().map(|b| b.viewer_sample_count).sum::<u64>(),"chatChecks":samples})
        );
        Ok(())
    })();
    let _ = service.close(&session.token);
    service.shutdown_and_wait();
    outcome
}
