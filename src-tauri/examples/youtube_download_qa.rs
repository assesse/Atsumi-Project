//! Isolated public YouTube ordinary VOD download -> verified library/range.
//! Production service only; no browser, account profile or invented live chat.
use atsumi_lib::platform_media::{protocol, MediaRef, PlatformMediaService, PlatformMediaTools};
use serde_json::{json, Value};
use std::{path::{Path, PathBuf}, time::{Duration, Instant}};

const SAMPLE: &str = "jNQXAC9IVRw";

fn verify(service: &PlatformMediaService, root: &Path) -> Result<Value, String> {
    let reference = MediaRef { provider: "youtube".into(), kind: "video".into(), external_id: SAMPLE.into() };
    let job = service.download(reference.clone(), uuid::Uuid::new_v4().to_string(), root.join("media"))?;
    let started = Instant::now();
    let finished = loop {
        let current = service.jobs(Some("youtube")).into_iter().find(|item| item.job_id == job.job_id).ok_or("test job missing")?;
        if matches!(current.state.as_str(), "completed" | "failed" | "interrupted" | "cancelled") { break current; }
        if started.elapsed() > Duration::from_secs(180) { return Err("isolated public YouTube VOD timed out".into()); }
        std::thread::sleep(Duration::from_millis(250));
    };
    if finished.state != "completed" { return Ok(json!({"kind":"youtube_download_qa","success":false,"stage":"download","job":finished})); }
    let listed = service.library("youtube", None, 50)?.items.iter().any(|item| item.library_id == job.job_id && item.reference == reference);
    let opened = service.open(&job.job_id)?;
    let response = protocol::respond(service, "main", tauri::http::Request::builder()
        .uri(format!("http://platform-media.localhost/{}/media.mp4", opened.token))
        .header("Range", "bytes=0-1023").body(Vec::new()).map_err(|_| "test request")?);
    let media_range_ok = response.status() == 206 && response.body().len() == 1024;
    let tail = protocol::respond(service, "main", tauri::http::Request::builder()
        .uri(format!("http://platform-media.localhost/{}/media.mp4", opened.token))
        .header("Range", "bytes=-1024").body(Vec::new()).map_err(|_| "test request")?);
    let media_tail_ok = tail.status() == 206 && tail.body().len() == 1024;
    service.close(&opened.token);
    let reopened = service.open(&job.job_id)?;
    let identity_stable = reopened.item.sha256 == opened.item.sha256 && reopened.item.reference == reference;
    service.close(&reopened.token);
    let directory = root.join("media/YouTube").join(&job.job_id);
    let no_chat_invented = opened.item.chat_archive.is_none()
        && !directory.join("chat-status.json").exists()
        && !directory.join("chat-status.final.json").exists()
        && !directory.join("chat.events.jsonl").exists();
    let success = listed && media_range_ok && media_tail_ok && identity_stable && no_chat_invented
        && (opened.item.duration - 19.0).abs() < 1.0
        && opened.item.bytes > 1024 && opened.item.sha256.len() == 64
        && opened.item.sha256.bytes().all(|byte| byte.is_ascii_hexdigit());
    Ok(json!({"kind":"youtube_download_qa","success":success,"isolated":true,"sample":SAMPLE,
        "job":finished,"libraryListed":listed,"libraryOpened":true,"libraryReopened":identity_stable,
        "mediaRangeVerified":media_range_ok,"mediaTailRangeVerified":media_tail_ok,"noInventedChatArchive":no_chat_invented}))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().ok_or("workspace missing")?.to_owned();
    let root = repo.join(".runtime/youtube-download-qa").join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(root.join("media"))?;
    let bin = repo.join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let service = PlatformMediaService::new(root.join("data"), PlatformMediaTools {
        yt_dlp: repo.join(".runtime/platform-tools/bin/yt-dlp.exe"), deno: repo.join(".runtime/platform-tools/bin/deno.exe"),
        ffmpeg: bin.join("ffmpeg.exe"), ffprobe: bin.join("ffprobe.exe") }, None)?;
    let result = verify(&service, &root);
    service.shutdown();
    let value = result?;
    println!("{value}");
    if value["success"] != true { return Err("YouTube ordinary VOD download/library roundtrip not verified".into()); }
    Ok(())
}
