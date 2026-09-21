//! Explicit one-off maintenance: no app startup, recording, playback, or deletion.
//! Usage: cargo run --example repair_recording_profiles -- DATA_DIR BrowserCapture --apply
use atsumi_lib::streaming::{provider::ChzzkProvider, replay_assets};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    id: String,
    channel_id: String,
    output_dir: String,
}

fn plain(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    for parent in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(parent) {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err("reparse point refused".into());
                }
            }
            if meta.file_type().is_symlink() {
                return Err("symlink refused".into());
            }
        }
    }
    Ok(fs::canonicalize(path)?)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 || args[2] != "--apply" {
        return Err("Expected DATA_DIR BrowserCapture --apply".into());
    }
    let data_dir = plain(Path::new(&args[0]))?;
    let root = plain(Path::new(&args[1]))?;
    if root.file_name().and_then(|s| s.to_str()) != Some("BrowserCapture") {
        return Err("Wrong archive root".into());
    }
    let catalog = plain(&data_dir.join("streaming/browser/catalog.jsonl"))?;
    if fs::metadata(&catalog)?.len() > 8 * 1024 * 1024 {
        return Err("Catalog too large".into());
    }
    let provider = ChzzkProvider::new().map_err(|e| e.code)?;
    let mut urls: HashMap<String, Option<String>> = HashMap::new();
    let mut unchanged = 0;
    let mut repaired = Vec::new();
    let mut unavailable = Vec::new();
    for line in fs::read_to_string(catalog)?.lines() {
        let entry: Entry = serde_json::from_str(line)?;
        if entry.id.len() != 32 || !entry.id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid recording ID".into());
        }
        let record = plain(Path::new(&entry.output_dir))?;
        if record != root.join(&entry.id) {
            return Err("Recording outside explicit archive".into());
        }
        if replay_assets::channel_profile::read(&record, &entry.channel_id)
            .1
            .is_some()
        {
            unchanged += 1;
            continue;
        }
        // Prefer archived bytes, even if today's channel avatar has changed.
        if replay_assets::repair_channel_image(&data_dir, &record, &entry.channel_id, None).is_ok()
        {
            repaired.push(entry.id);
            continue;
        }
        let url = urls.entry(entry.channel_id.clone()).or_insert_with(|| {
            provider
                .channel_profile(&entry.channel_id)
                .ok()
                .and_then(|(_, url)| url)
        });
        match replay_assets::repair_channel_image(
            &data_dir,
            &record,
            &entry.channel_id,
            url.as_deref(),
        ) {
            Ok(true) => repaired.push(entry.id),
            Ok(false) => unchanged += 1,
            Err(error) => unavailable.push(json!({"id":entry.id,"code":error.code})),
        }
    }
    println!(
        "{}",
        json!({"repairedCount":repaired.len(),"unchangedCount":unchanged,"repairedIds":repaired,"unavailable":unavailable})
    );
    Ok(())
}
