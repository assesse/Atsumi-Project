//! Bind public SOOP title numbers to the extractor's ordered internal file IDs.
//! Media URLs and browser credentials never enter this plan or diagnostics.
use super::chat_assets;
use reqwest::{blocking::Client, redirect::Policy, StatusCode};
use serde_json::Value;
use std::{collections::HashSet, io::Read, sync::atomic::{AtomicBool, Ordering}, time::Duration};

const ERROR: &str = "SOOP 영상의 전체 파일 구성을 확인하지 못했습니다. 다시 시도해 주세요.";
const LIMIT: usize = 2 * 1024 * 1024;
pub(super) struct Plan { pub metadata: Value, pub duration: f64, pub multipart: bool, keys: Vec<String> }
fn number(value: &Value) -> Option<u64> { value.as_u64().or_else(|| value.as_str()?.parse().ok()) }
fn duration(value: &Value) -> Option<f64> { value.as_f64().filter(|v| v.is_finite() && *v > 0.0 && *v <= 86_400.0) }
fn key(value: &Value) -> Option<&str> { value.as_str().filter(|v| !v.is_empty() && v.len() <= 100 && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))) }

pub(super) fn fetch(id: &str, cancel: &AtomicBool) -> Result<Value, String> {
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) { return Err(ERROR.into()); }
    if cancel.load(Ordering::Acquire) { return Err("cancelled".into()); }
    let host = "api.m.sooplive.com";
    let addresses = chat_assets::addresses(host)?;
    let client = Client::builder().https_only(true).no_proxy().redirect(Policy::none())
        .resolve_to_addrs(host, &addresses).connect_timeout(Duration::from_secs(3)).timeout(Duration::from_secs(20))
        .user_agent("Mozilla/5.0 (compatible; Atsumi/2; public-vod-reader)").build().map_err(|_| ERROR)?;
    let response = client.post("https://api.m.sooplive.com/station/video/a/view")
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(reqwest::header::REFERER, format!("https://vod.sooplive.com/player/{id}"))
        .body(format!("nTitleNo={id}&nApiLevel=11&nPlaylistIdx=0")).send().map_err(|_| ERROR)?;
    if response.status() != StatusCode::OK || response.content_length().is_some_and(|n| n > LIMIT as u64) { return Err(ERROR.into()); }
    let mut response = response.take((LIMIT + 1) as u64); let mut bytes = Vec::new(); let mut chunk = [0; 8192];
    loop { if cancel.load(Ordering::Acquire) { return Err("cancelled".into()); }
        let read = response.read(&mut chunk).map_err(|_| ERROR)?; if read == 0 { break; }
        bytes.extend_from_slice(&chunk[..read]); if bytes.len() > LIMIT { return Err(ERROR.into()); }
    }
    serde_json::from_slice(&bytes).map_err(|_| ERROR.into())
}

pub(super) fn bind(id: &str, metadata: Value, extracted: &Value) -> Result<Plan, String> {
    let data = &metadata["data"];
    if number(&metadata["result"]) != Some(1) || number(&data["title_no"]).map(|n| n.to_string()).as_deref() != Some(id)
        || data["adult_status"] == "notLogin" || extracted["extractor"].as_str() != Some("soop")
        || extracted["extractor_key"].as_str() != Some("AfreecaTV")
        || extracted["webpage_url"].as_str() != Some(format!("https://vod.sooplive.com/player/{id}").as_str()) {
        return Err(ERROR.into());
    }
    let files = data["files"].as_array().filter(|v| !v.is_empty() && v.len() <= 32).ok_or(ERROR)?;
    let mut keys = Vec::new(); let mut seen = HashSet::new(); let mut milliseconds = 0u64;
    let multipart = files.len() > 1;
    let entries: Vec<&Value> = if multipart {
        if extracted["_type"] != "multi_video" || extracted["id"] != id { return Err(ERROR.into()); }
        extracted["entries"].as_array().filter(|entries| entries.len() == files.len()).ok_or(ERROR)?.iter().collect()
    } else {
        if extracted.get("entries").is_some() || extracted.get("_type").is_some_and(|value| value != "video") { return Err(ERROR.into()); }
        vec![extracted]
    };
    for (file, entry) in files.iter().zip(entries) {
        let file_key = key(&file["file_info_key"]).ok_or(ERROR)?;
        let ms = number(&file["duration"]).filter(|v| *v > 0 && *v <= 86_400_000).ok_or(ERROR)?;
        if !seen.insert(file_key.to_owned()) || entry["id"].as_str() != Some(file_key)
            || entry["is_live"] == true || entry["live_status"] == "is_upcoming"
            || duration(&entry["duration"]).is_none_or(|seconds| (seconds - ms as f64 / 1000.0).abs() > 1.1) {
            return Err(ERROR.into());
        }
        milliseconds = milliseconds.checked_add(ms).filter(|v| *v <= 86_400_000).ok_or(ERROR)?;
        keys.push(file_key.to_owned());
    }
    if number(&data["total_file_duration"]).is_none_or(|ms| ms.abs_diff(milliseconds) > 1000)
        || duration(&extracted["duration"]).is_none_or(|seconds| (seconds - milliseconds as f64 / 1000.0).abs() > 1.1) { return Err(ERROR.into()); }
    Ok(Plan { metadata, duration: milliseconds as f64 / 1000.0, multipart, keys })
}
impl Plan {
    pub fn output_args(&self) -> Vec<String> {
        let mut args = vec!["--print".into(), "after_move:ATSUMI_PART_ID:%(id)s".into()];
        if self.multipart {
            args.extend(["--concat-playlist", "always", "--output", "download.part-%(playlist_index)03d.%(ext)s", "--output", "pl_video:download.%(ext)s"].map(String::from));
        } else { args.extend(["--output", "download.%(ext)s"].map(String::from)); }
        args
    }
    pub fn verify_receipt(&self, bytes: &[u8]) -> Result<(), String> {
        let text = std::str::from_utf8(bytes).map_err(|_| ERROR)?;
        let keys: Vec<_> = text.lines().filter_map(|line| line.strip_prefix("ATSUMI_PART_ID:")).collect();
        if keys.len() != self.keys.len() || keys.iter().zip(&self.keys).any(|(a,b)| *a != b) { return Err(ERROR.into()); }
        Ok(())
    }
    pub fn has_chat(&self) -> bool { self.metadata["data"]["show_chat"] == "Y" }
}

pub(super) fn owned_intermediate(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("download.part-") else { return false; };
    let Some(number) = rest.get(..3) else { return false; };
    if !number.bytes().all(|b| b.is_ascii_digit()) || !number.parse::<u32>().is_ok_and(|n| (1..=32).contains(&n)) { return false; }
    let Some(suffix) = rest.get(3..) else { return false; };
    if matches!(suffix, ".mp4" | ".m4a" | ".webm") { return true; }
    suffix.strip_prefix(".f").is_some_and(|value| {
        let Some((format, extension)) = value.rsplit_once('.') else { return false; };
        !format.is_empty() && format.len() <= 40 && format.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b,b'_'|b'-')) && matches!(extension,"mp4"|"m4a"|"webm")
    })
}

#[cfg(test)] mod tests {
    use super::*;
    use serde_json::json;
    fn fixtures(multi: bool) -> (Value,Value) {
        let files=if multi {json!([{"file_info_key":"one","duration":9500},{"file_info_key":"two","duration":8500}])}else{json!([{"file_info_key":"one","duration":9500}])};
        let metadata=json!({"result":1,"data":{"title_no":123,"total_file_duration":if multi{18000}else{9500},"files":files,"show_chat":"Y"}});
        let mut extracted=json!({"extractor":"soop","extractor_key":"AfreecaTV","webpage_url":"https://vod.sooplive.com/player/123","id":"one","duration":9,"_type":"video"});
        if multi {extracted["_type"]=json!("multi_video");extracted["id"]=json!("123");extracted["duration"]=json!(18);extracted["entries"]=json!([{"id":"one","duration":9},{"id":"two","duration":8}]);}
        (metadata,extracted)
    }
    #[test] fn binds_internal_file_ids_without_relaxing_public_identity() {
        for multi in [false,true] {let (metadata,extracted)=fixtures(multi);let plan=bind("123",metadata.clone(),&extracted).unwrap();assert_eq!(plan.multipart,multi);assert!(plan.has_chat());
            let mut bad=extracted.clone();bad["webpage_url"]=json!("https://vod.sooplive.com/player/456");assert!(bind("123",metadata.clone(),&bad).is_err());
            let mut bad=metadata.clone();bad["data"]["adult_status"]=json!("notLogin");assert!(bind("123",bad,&extracted).is_err());
            assert!(bind("456",metadata,&extracted).is_err());}
    }
    #[test] fn refuses_missing_swapped_duplicate_or_wrong_duration_parts() {
        let (metadata,extracted)=fixtures(true);
        for entries in [json!([{"id":"one","duration":9}]),json!([{"id":"two","duration":8},{"id":"one","duration":9}]),json!([{"id":"one","duration":9},{"id":"one","duration":8}]),json!([{"id":"one","duration":9},{"id":"two","duration":2}])] {let mut bad=extracted.clone();bad["entries"]=entries;assert!(bind("123",metadata.clone(),&bad).is_err());}
    }
    #[test] fn unique_part_output_and_exact_download_receipt() {
        let (metadata,extracted)=fixtures(true);let plan=bind("123",metadata,&extracted).unwrap();
        assert!(plan.output_args().contains(&"download.part-%(playlist_index)03d.%(ext)s".into()));assert!(plan.output_args().contains(&"pl_video:download.%(ext)s".into()));
        assert!(plan.verify_receipt(b"ATSUMI_PART_ID:one\nATSUMI_PART_ID:two\n").is_ok());
        for receipt in [b"ATSUMI_PART_ID:one\n".as_slice(),b"ATSUMI_PART_ID:two\nATSUMI_PART_ID:one\n",b"ATSUMI_PART_ID:one\nATSUMI_PART_ID:one\n"] {assert!(plan.verify_receipt(receipt).is_err());}
        for name in ["download.part-001.mp4","download.part-032.f137.mp4"] {assert!(owned_intermediate(name));}
        for name in ["download.part-../secret.mp4","download.part-001.mp4.backup","download.part-000.mp4","download.part-033.mp4","download.part-999.mp4","download.part-001.notes.txt"] {assert!(!owned_intermediate(name));}
    }
}
