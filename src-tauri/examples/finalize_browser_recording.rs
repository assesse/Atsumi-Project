//! Explicit offline maintenance of ONE catalog-authorized recording.
//! Close Atsumi first. Uses the production merge/verification/cleanup pipeline;
//! never starts a browser or loads other recordings or account data.
//! cargo run --example finalize_browser_recording -- DATA_DIR ID TOOLS_DIR --apply
use atsumi_lib::streaming::{
    browser_merge::{BrowserMergeWorker, MediaTools},
    browser_store::{BrowserCaptureStore, BrowserMergeStatus, BrowserSourceCleanupStatus},
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

fn remove_cached_summary(index: &mut Value, id: &str) -> Result<bool, Box<dyn std::error::Error>> {
    if index["version"] != 1 {
        return Err("Unknown startup index; recording is still repaired".into());
    }
    let records = index["recordings"]
        .as_array_mut()
        .ok_or("Invalid startup index")?;
    let before = records.len();
    records.retain(|record| record["id"].as_str() != Some(id));
    Ok(before != records.len())
}

fn invalidate_summary(data: &Path, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    app_closed()?;
    // Remove only this disposable UI cache entry. On launch the app will read
    // the repaired recording's authoritative summary immediately instead of
    // showing the old queue state until its background recovery turn arrives.
    let path = data.join("streaming/browser/startup-index.json");
    if !path.exists() {
        return Ok(());
    }
    if fs::metadata(&path)?.len() > 64 * 1024 * 1024 {
        return Err("Startup index too large; recording is still repaired".into());
    }
    let original = fs::read(&path)?;
    let mut index: Value = serde_json::from_slice(&original)?;
    if !remove_cached_summary(&mut index, id)? {
        return Ok(());
    }
    let directory = path.parent().ok_or("Invalid index directory")?;
    let mut backup = tempfile::Builder::new()
        .prefix("startup-index-before-finalize-")
        .suffix(".json")
        .tempfile_in(directory)?;
    backup.write_all(&original)?;
    backup.as_file().sync_all()?;
    let (_, backup_path) = backup.keep()?;
    let mut replacement = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut replacement, &index)?;
    replacement.as_file().sync_all()?;
    // Never overwrite a concurrently updated cache.
    app_closed()?;
    if fs::read(&path)? != original {
        return Err("Startup index changed; preserved repaired recording and cache backup".into());
    }
    replacement.persist(path)?;
    println!(
        "{}",
        json!({"summaryInvalidated":id,"cacheBackup":backup_path})
    );
    Ok(())
}

fn app_closed() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let status = std::process::Command::new(
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
        )
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "if (Get-Process -Name Atsumi -ErrorAction SilentlyContinue) { exit 1 }; exit 0",
        ])
        .creation_flags(0x08000000)
        .status()?;
        if !status.success() {
            return Err("Close Atsumi completely before maintenance".into());
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .init();
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 4
        || !matches!(args[3].as_str(), "--apply" | "--refresh-summary")
        || args[1].len() != 32
        || !args[1]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("Expected DATA_DIR ID TOOLS_DIR --apply or --refresh-summary".into());
    }
    app_closed()?;
    let data = fs::canonicalize(&args[0])?;
    let catalog = data.join("streaming/browser/catalog.jsonl");
    if fs::metadata(&catalog)?.len() > 8 * 1024 * 1024 {
        return Err("Catalog too large".into());
    }
    let mut entry = None;
    for line in fs::read_to_string(&catalog)?.lines() {
        let row: Value = serde_json::from_str(line)?;
        if row["id"].as_str() == Some(&args[1]) {
            if entry.is_some() {
                return Err("Duplicate recording ID".into());
            }
            entry = Some(row);
        }
    }
    let entry = entry.ok_or("Recording not in catalog")?;
    if entry["deletionPending"] == true {
        return Err("Recording marked for deletion".into());
    }
    let root = PathBuf::from(
        entry["outputDir"]
            .as_str()
            .ok_or("Missing recording root")?,
    );
    if args[3] == "--refresh-summary" {
        let saved: Value = serde_json::from_slice(&fs::read(root.join("recording.json"))?)?;
        if saved["id"].as_str() != Some(&args[1])
            || saved["merge"]["status"] != "complete"
            || saved["merge"]["sourceCleanup"]["status"] != "complete"
        {
            return Err("Recording is not fully finalized; cache unchanged".into());
        }
        return invalidate_summary(&data, &args[1]);
    }
    // The staging catalog contains the same single authorization entry. Only
    // its startup cache is temporary; media/proofs remain in the original root.
    // The main catalog, other recordings, diagnostics and chat are untouched.
    // Only this record's disposable UI cache is invalidated after success.
    let staging = tempfile::Builder::new()
        .prefix("atsumi-finalize-one-")
        .tempdir()?;
    let catalog_dir = staging.path().join("streaming/browser");
    fs::create_dir_all(&catalog_dir)?;
    fs::write(
        catalog_dir.join("catalog.jsonl"),
        format!("{}\n", serde_json::to_string(&entry)?),
    )?;
    let tools_dir = fs::canonicalize(&args[2])?;
    let tools = MediaTools {
        ffmpeg: tools_dir.join("ffmpeg.exe"),
        ffprobe: tools_dir.join("ffprobe.exe"),
    };
    let store = Arc::new(Mutex::new(BrowserCaptureStore::new(staging.path())?));
    if store.lock().unwrap().snapshot()?.len() != 1 {
        return Err("Unexpected maintenance scope".into());
    }
    store.lock().unwrap().retry_merges(Some(&args[1]))?;
    let worker = BrowserMergeWorker::start(store.clone(), Some(tools))?;
    let start = Instant::now();
    let mut last = String::new();
    let outcome = loop {
        let record = store.lock().unwrap().snapshot()?.remove(0);
        let merge = record.merge.as_ref();
        let state = serde_json::to_string(
            &json!({"id":record.id,"merge":merge,"segmentCount":record.segment_count}),
        )?;
        if state != last {
            println!("{state}");
            last = state;
        }
        if merge.is_some_and(|m| {
            m.status == BrowserMergeStatus::Complete
                && m.source_cleanup
                    .as_ref()
                    .is_none_or(|c| c.status != BrowserSourceCleanupStatus::Pending)
        }) {
            break Ok(());
        }
        if merge.is_some_and(|m| {
            matches!(
                m.status,
                BrowserMergeStatus::Failed | BrowserMergeStatus::Blocked
            )
        }) {
            break Err("Merge failed/blocked; sources preserved");
        }
        if start.elapsed() > Duration::from_secs(14400) {
            break Err("Maintenance deadline; sources preserved");
        }
        thread::sleep(Duration::from_secs(2));
    };
    worker.shutdown_and_wait();
    store.lock().unwrap().shutdown()?;
    // Check the authoritative metadata, not merely the staging cache.
    let saved: Value = serde_json::from_slice(&fs::read(root.join("recording.json"))?)?;
    println!(
        "{}",
        json!({"savedMerge":saved["merge"],"elapsedSeconds":start.elapsed().as_secs()})
    );
    outcome?;
    invalidate_summary(&data, &args[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_invalidation_preserves_every_other_record_and_field() {
        let other = json!({"id":"other","merge":{"status":"queued"},"chatCount":12});
        let mut index = json!({"version":1,"extension":"preserved","recordings":[other.clone(),{"id":"target"}]});
        assert!(remove_cached_summary(&mut index, "target").unwrap());
        assert_eq!(
            index,
            json!({"version":1,"extension":"preserved","recordings":[other]})
        );
        assert!(!remove_cached_summary(&mut index, "target").unwrap());
    }

    #[test]
    fn unknown_cache_shapes_are_not_rewritten() {
        for mut index in [
            json!({"version":2,"recordings":[]}),
            json!({"version":1,"recordings":null}),
        ] {
            let original = index.clone();
            assert!(remove_cached_summary(&mut index, "target").is_err());
            assert_eq!(index, original);
        }
    }
}
