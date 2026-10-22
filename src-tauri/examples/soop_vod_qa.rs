//! Isolated public SOOP VOD download -> verified video + timed local replay chat.
//! Uses production service and never reads an account profile or prints chat text.
use atsumi_lib::platform_media::{protocol, MediaRef, PlatformMediaService, PlatformMediaTools};
use serde_json::{json, Value};
use std::{path::PathBuf, time::{Duration, Instant}};

fn verify(service: &PlatformMediaService, root: &std::path::Path) -> Result<Value, String> {
    let job = service.download(MediaRef { provider: "soop".into(), kind: "video".into(), external_id: "209244227".into() }, uuid::Uuid::new_v4().to_string(), root.join("media"))?;
    let started = Instant::now();
    let finished = loop {
        let current = service.jobs(Some("soop")).into_iter().find(|item| item.job_id == job.job_id).ok_or("test job missing")?;
        if matches!(current.state.as_str(), "completed" | "failed" | "interrupted" | "cancelled") { break current; }
        if started.elapsed() > Duration::from_secs(180) { return Err("isolated public VOD timed out".into()); }
        std::thread::sleep(Duration::from_millis(250));
    };
    if finished.state != "completed" { return Ok(json!({"success":false,"stage":"download","job":finished})); }
    let opened = service.open(&job.job_id)?;
    let response = protocol::respond(service, "main", tauri::http::Request::builder()
        .uri(format!("http://platform-media.localhost/{}/media.mp4", opened.token))
        .header("Range", "bytes=0-1023").body(Vec::new()).map_err(|_| "test request")?);
    let media_range_ok = response.status() == 206 && response.body().len() == 1024;
    service.close(&opened.token);
    let replay_started = Instant::now();
    let page = loop {
        let page = service.replay_chat(&job.job_id, 9500, None, 120)?;
        if page.state != "preparing" { break page; }
        if replay_started.elapsed() > Duration::from_secs(15) { return Err("isolated replay preparation timed out".into()); }
        std::thread::sleep(Duration::from_millis(100));
    };
    let chat_summary = serde_json::to_value(&opened.item.chat_archive).map_err(|_| "test chat summary")?;
    let offsets: Vec<_> = page.items.iter().filter_map(|item| item.time_ms).collect();
    let success = media_range_ok && (opened.item.duration - 9.5).abs() < 0.5
        && opened.item.sha256.len() == 64 && chat_summary["state"] == "complete"
        && page.total == 30 && page.items.len() == 30 && !page.partial_file
        && page.malformed_lines == 0 && page.timeline_quality == "native-offset"
        && offsets.first() == Some(&50) && offsets.last() == Some(&8850);
    Ok(json!({"kind":"soop_vod_qa","success":success,"isolated":true,"sample":"209244227",
        "job":finished,"libraryOpened":true,"mediaRangeVerified":media_range_ok,
        "chat":{"state":page.state,"total":page.total,"returned":page.items.len(),"timelineQuality":page.timeline_quality,
            "firstOffsetMs":offsets.first(),"lastOffsetMs":offsets.last(),"malformed":page.malformed_lines,"partial":page.partial_file}}))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().ok_or("workspace missing")?.to_owned();
    let root = repo.join(".runtime/soop-vod-qa").join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(root.join("media"))?;
    let bin = repo.join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let service = PlatformMediaService::new(root.join("data"), PlatformMediaTools {
        yt_dlp: repo.join(".runtime/platform-tools/bin/yt-dlp.exe"), deno: repo.join(".runtime/platform-tools/bin/deno.exe"),
        ffmpeg: bin.join("ffmpeg.exe"), ffprobe: bin.join("ffprobe.exe") }, None)?;
    let result = verify(&service, &root);
    service.shutdown();
    let value = result?;
    println!("{value}");
    if value["success"] != true { return Err("SOOP VOD video/chat roundtrip not verified".into()); }
    Ok(())
}
