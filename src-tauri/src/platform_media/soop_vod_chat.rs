//! Public SOOP replay chat, bound to the same VOD and its ordered media files.
//! Official ChatCore requests hourly ChatLoadSplit parts with startTime seconds.
use super::{chat_assets, chat_parts::{Stream, Writer}, chat_status::{self, Reason}, MediaJob};
use quick_xml::{events::Event, Reader};
use reqwest::{blocking::{Client, Response}, redirect::Policy, Url};
use serde_json::{json, Map, Value};
use std::{collections::HashSet, io::Read, path::Path, sync::atomic::{AtomicBool, Ordering}, time::{Duration, Instant}};

const ERROR: &str = "SOOP 다시보기 채팅을 모두 받지 못했습니다. 영상과 받은 채팅은 보존합니다.";
const BODY: usize = 8 * 1024 * 1024;
const MAX_REQUESTS: usize = 96;
const MAX_EVENTS: usize = 100_000;
struct Part { url: Url, duration_ms: u64, offset_ms: u64 }
fn positive(value: &Value) -> Option<u64> { value.as_u64().or_else(|| value.as_str()?.parse().ok()).filter(|v| *v > 0) }
fn url(raw: &str, key: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| ERROR)?;
    let pairs: Vec<_> = url.query_pairs().collect();
    if raw.len() > 512 || url.scheme() != "https" || url.host_str() != Some("videoimg.sooplive.com")
        || url.port().is_some() || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some()
        || url.path() != "/php/ChatLoadSplit.php" || pairs.len() != 1 || pairs[0].0 != "rowKey"
        || pairs[0].1 != format!("{key}_c") || key.is_empty() || key.len() > 100
        || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') { return Err(ERROR.into()); }
    Ok(url)
}
fn parts(value: &Value, id: &str) -> Result<Vec<Part>, String> {
    let data = &value["data"];
    if value["result"].as_i64() != Some(1) || positive(&data["title_no"]).map(|v| v.to_string()).as_deref() != Some(id)
        || data["show_chat"].as_str() != Some("Y") { return Err(ERROR.into()); }
    let files = data["files"].as_array().filter(|v| !v.is_empty() && v.len() <= 32).ok_or(ERROR)?;
    let mut offset = 0u64;
    let mut keys = HashSet::new();
    let mut result = Vec::new();
    for file in files {
        let duration = positive(&file["duration"]).filter(|v| *v <= 86_400_000).ok_or(ERROR)?;
        let key = file["file_info_key"].as_str().ok_or(ERROR)?;
        if !keys.insert(key) { return Err(ERROR.into()); }
        let url = url(file["chat"].as_str().ok_or(ERROR)?, key)?;
        result.push(Part { url, duration_ms: duration, offset_ms: offset });
        offset = offset.checked_add(duration).filter(|n| *n <= 86_400_000).ok_or(ERROR)?;
    }
    if positive(&data["total_file_duration"]).is_none_or(|total| total.abs_diff(offset) > 1000) { return Err(ERROR.into()); }
    Ok(result)
}
fn client(host: &str) -> Result<Client, String> {
    let addresses = chat_assets::addresses(host)?;
    Client::builder().https_only(true).no_proxy().redirect(Policy::none())
        .resolve_to_addrs(host, &addresses).connect_timeout(Duration::from_secs(3)).timeout(Duration::from_secs(20))
        .user_agent("Mozilla/5.0 (compatible; Atsumi/2; public-replay-chat)").build().map_err(|_| ERROR.into())
}
fn body(response: Response, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    if response.status() != reqwest::StatusCode::OK || response.content_length().is_some_and(|n| n > BODY as u64) { return Err(ERROR.into()); }
    let mut response = response.take((BODY + 1) as u64);
    let mut result = Vec::new(); let mut chunk = [0; 16384];
    loop { if cancel.load(Ordering::Acquire) { return Err("cancelled".into()); }
        let n = response.read(&mut chunk).map_err(|_| ERROR)?; if n == 0 { break; }
        result.extend_from_slice(&chunk[..n]); if result.len() > BODY { return Err(ERROR.into()); }
    }
    Ok(result)
}
fn text_append(fields: &mut Map<String, Value>, stack: &[String], text: &str) -> Result<(), String> {
    if stack.len() < 2 { return if text.trim().is_empty() { Ok(()) } else { Err(ERROR.into()) }; }
    let key = stack.get(2).map(String::as_str).unwrap_or("value");
    if fields.len() >= 64 && !fields.contains_key(key) { return Err(ERROR.into()); }
    let entry = fields.entry(key.to_string()).or_insert_with(|| json!(""));
    let previous = entry.as_str().ok_or(ERROR)?;
    if previous.len() + text.len() > 16_384 { return Err(ERROR.into()); }
    let mut value = previous.to_owned(); value.push_str(text); fields.insert(key.into(), json!(value)); Ok(())
}
/// Streaming XML tokenizer; no DTD, entity expansion, arbitrary URLs or HTML rendering.
fn parse(bytes: &[u8], part: &Part, start: u64, mut sink: impl FnMut(Value, Option<Value>) -> Result<(), String>) -> Result<(usize, bool), String> {
    if bytes.len() > BODY { return Err(ERROR.into()); }
    let mut reader = Reader::from_reader(bytes); reader.config_mut().check_end_names = true;
    let mut stack = Vec::<String>::new(); let mut fields = Map::new(); let mut root = false; let mut closed = false; let mut count = 0usize;
    let mut highlight = 0f64;
    let mut version = false; let mut unsupported = false; let mut raw_start = 0usize;
    loop { let event_start = reader.buffer_position() as usize; match reader.read_event().map_err(|_| ERROR)? {
        Event::Start(tag) => {
            let name = std::str::from_utf8(tag.name().as_ref()).map_err(|_| ERROR)?.to_owned();
            if name.len() > 64 || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') || stack.len() >= 8 || tag.attributes().next().is_some() { return Err(ERROR.into()); }
            if stack.is_empty() { if root || closed || name != "root" { return Err(ERROR.into()); } root = true; }
            if stack.len() == 1 { fields.clear(); raw_start = event_start; }
            if stack.len() >= 3 { unsupported = true; }
            stack.push(name);
        }
        Event::Text(text) => text_append(&mut fields, &stack, &text.decode().map_err(|_| ERROR)?)?,
        Event::CData(text) => text_append(&mut fields, &stack, &text.decode().map_err(|_| ERROR)?)?,
        Event::GeneralRef(reference) => {
            let name = reference.decode().map_err(|_| ERROR)?;
            let escaped = format!("&{name};");
            let text = quick_xml::escape::unescape(&escaped).map_err(|_| ERROR)?;
            text_append(&mut fields, &stack, &text)?;
        }
        Event::End(_) => {
            if stack.len() == 2 {
                count += 1; if count > MAX_EVENTS { return Err(ERROR.into()); }
                let tag = &stack[1];
                if tag == "version" {
                    if version || !fields.get("value").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty() && s.len() <= 64) { return Err(ERROR.into()); }
                    version = true;
                } else if tag != "chat" && tag != "highlight_start" { unsupported = true; }
                if tag == "highlight_start" {
                    highlight = fields.get("value").and_then(Value::as_str).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0 && *v < 86_400.0).ok_or(ERROR)?;
                }
                let normalized = if tag == "chat" {
                    let seconds = fields.get("t").and_then(Value::as_str).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite()).ok_or(ERROR)? - highlight;
                    if seconds < 0.0 || seconds * 1000.0 > part.duration_ms as f64 + 1000.0 { return Err(ERROR.into()); }
                    // A repeated/ignored startTime page is not proof of the
                    // requested interval. Do not silently drop it as "empty".
                    if seconds < start as f64 || seconds >= (start + 3600) as f64 { return Err(ERROR.into()); }
                    let offset = (seconds * 1000.0).round() as u64;
                        Some(json!({"format":"atsumi-chat-v1","receivedAtMs":0,"sourceTimestampMs":null,"videoOffsetMs":part.offset_ms+offset,
                            "kind":"chat","author":fields.get("n").and_then(Value::as_str).unwrap_or(""),
                            "text":fields.get("m").and_then(Value::as_str).unwrap_or(""),"parts":[],"amount":null,"badges":[],"color":null}))
                } else { None };
                // Preserve exact XML, including nested and duplicate fields;
                // the normalized display map is not the archival source.
                let raw = std::str::from_utf8(bytes.get(raw_start..reader.buffer_position() as usize).ok_or(ERROR)?).map_err(|_| ERROR)?;
                sink(json!({"format":"soop-vod-xml-v1","tag":tag,"partOffsetMs":part.offset_ms,"xml":raw}), normalized)?;
            }
            if stack.pop().is_none() { return Err(ERROR.into()); } if stack.is_empty() { closed = true; }
        }
        Event::Empty(tag) => {
            if stack.len() < 2 || tag.attributes().next().is_some() { return Err(ERROR.into()); }
        }
        Event::Decl(_) | Event::Comment(_) => (),
        Event::Eof => break,
        _ => return Err(ERROR.into()),
    }}
    if !root || !closed || !stack.is_empty() || !version { return Err(ERROR.into()); } Ok((count, unsupported))
}
fn acquire(job: &MediaJob, metadata: &Value, cancel: &AtomicBool, writer: &mut Writer) -> Result<(), String> {
    if job.reference.external_id.is_empty() || job.reference.external_id.len() > 20 || !job.reference.external_id.bytes().all(|b| b.is_ascii_digit()) { return Err(ERROR.into()); }
    let parts = parts(metadata, &job.reference.external_id)?;
    let client = client("videoimg.sooplive.com")?; let began = Instant::now(); let mut requests = 0usize; let mut unsupported = false;
    for part in parts { for start in (0..part.duration_ms.div_ceil(1000)).step_by(3600) {
        if cancel.load(Ordering::Acquire) { return Err("cancelled".into()); }
        requests += 1; if requests > MAX_REQUESTS || began.elapsed() > Duration::from_secs(1800) { return Err(ERROR.into()); }
        let mut url = part.url.clone(); url.query_pairs_mut().append_pair("startTime", &start.to_string());
        let bytes = body(client.get(url).send().map_err(|_| ERROR)?, cancel)?;
        let (_, partial) = parse(&bytes, &part, start, |raw, normalized| {
            if cancel.load(Ordering::Acquire) { return Err("cancelled".into()); }
            let raw = serde_json::to_vec(&raw).map_err(|_| ERROR)?;
            writer.append(Stream::Source, &raw).map_err(|_| ERROR)?;
            if let Some(event) = normalized { writer.append(Stream::Events, &serde_json::to_vec(&event).map_err(|_| ERROR)?).map_err(|_| ERROR)?; }
            Ok(())
        })?;
        unsupported |= partial;
    }}
    if unsupported { Err(ERROR.into()) } else { Ok(()) }
}
pub(super) fn download_with_metadata(job: &MediaJob, directory: &Path, cancel: &AtomicBool, metadata: &Value) -> Result<(), String> {
    let marker = chat_status::begin_multipart(directory, "soop", &job.job_id)?;
    let mut writer = Writer::create(directory, &marker)?;
    let result = acquire(job, metadata, cancel, &mut writer);
    let closed = writer.close();
    let reason = closed.reason().or_else(|| result.as_ref().err().map(|_| if cancel.load(Ordering::Acquire) { Reason::Interrupted } else { Reason::StatusUnknown }));
    chat_status::finish_multipart(directory, &marker, closed, reason, false, cancel)?;
    if cancel.load(Ordering::Acquire) { return Err("cancelled".into()); }
    Ok(()) // The independently verified video remains usable with explicit partial chat.
}

#[cfg(test)]
mod tests {
    use super::*;
    fn part() -> Part { Part { url: url("https://videoimg.sooplive.com/php/ChatLoadSplit.php?rowKey=fixture_c", "fixture").unwrap(), duration_ms:9500, offset_ms:10000 } }
    #[test]
    fn exact_public_part_urls_only() {
        for bad in ["http://videoimg.sooplive.com/php/ChatLoadSplit.php?rowKey=fixture_c", "https://videoimg.sooplive.com.evil.invalid/php/ChatLoadSplit.php?rowKey=fixture_c", "https://videoimg.sooplive.com/php/ChatLoadSplit.php?rowKey=other_c", "https://videoimg.sooplive.com/php/ChatLoadSplit.php?rowKey=fixture_c&startTime=0"] { assert!(url(bad,"fixture").is_err()); }
    }
    #[test]
    fn replay_offsets_entities_and_cdata_are_preserved_without_html() {
        let mut output = Vec::new();
        parse(b"<root><version>2</version><chat><t>0.05</t><n>A&amp;B</n><m><![CDATA[<b>literal</b>]]></m></chat><chat><t>8.85</t><n>C</n><m>last</m></chat></root>", &part(), 0, |_, row| {if let Some(row)=row{output.push(row);} Ok(())}).unwrap();
        assert_eq!(output.len(),2); assert_eq!(output[0]["videoOffsetMs"],10050); assert_eq!(output[0]["author"],"A&B"); assert_eq!(output[0]["text"],"<b>literal</b>"); assert_eq!(output[1]["videoOffsetMs"],18850);
    }
    #[test]
    fn malformed_and_external_entities_do_not_claim_empty_success() {
        for bytes in [b"<root><chat>".as_slice(), b"<!DOCTYPE root [<!ENTITY e SYSTEM 'file:///secret'>]><root>&e;</root>", b"<html></html>", b"<root><chat><t>NaN</t></chat></root>"] {assert!(parse(bytes,&part(),0,|_,_|Ok(())).is_err());}
        assert!(parse(b"<root><version>2</version></root>",&part(),0,|_,_|Ok(())).is_ok());
    }
    #[test]
    fn metadata_parts_are_exact_ordered_and_not_arbitrary_targets() {
        let file=|key:&str,duration:u64| json!({"file_info_key":key,"duration":duration,"chat":format!("https://videoimg.sooplive.com/php/ChatLoadSplit.php?rowKey={key}_c")});
        let good=json!({"result":1,"data":{"title_no":123,"show_chat":"Y","total_file_duration":19000,"files":[file("first",9500),file("second",9500)]}});
        let parsed=parts(&good,"123").unwrap(); assert_eq!(parsed.len(),2); assert_eq!(parsed[1].offset_ms,9500);
        assert!(parts(&good,"456").is_err());
        let mut duplicate=good.clone(); duplicate["data"]["files"][1]=duplicate["data"]["files"][0].clone(); assert!(parts(&duplicate,"123").is_err());
        let mut wrong=good.clone(); wrong["data"]["total_file_duration"]=json!(90000); assert!(parts(&wrong,"123").is_err());
        let mut private=good; private["data"]["files"][0]["chat"]=json!("https://127.0.0.1/php/ChatLoadSplit.php?rowKey=first_c"); assert!(parts(&private,"123").is_err());
    }
    #[test]
    fn official_highlight_start_and_multipart_offsets_are_applied() {
        let mut rows=Vec::new();
        parse(b"<root><version>2</version><highlight_start>120</highlight_start><chat><t>122.5</t><n>A</n><m>fixture</m></chat></root>",&part(),0,|_,event|{if let Some(row)=event{rows.push(row)} Ok(())}).unwrap();
        assert_eq!(rows[0]["videoOffsetMs"],12500);
    }
    #[test]
    fn rich_or_nested_events_retain_exact_xml_but_do_not_claim_replay_complete() {
        let xml=b"<root><version>2</version><balloon><t>1</t><detail><n>A</n><n>B</n></detail></balloon></root>";
        let mut raw=Vec::new(); let (_,partial)=parse(xml,&part(),0,|value,_|{raw.push(value); Ok(())}).unwrap();
        assert!(partial); assert_eq!(raw[1]["xml"],"<balloon><t>1</t><detail><n>A</n><n>B</n></detail></balloon>");
        assert!(parse(b"<root><error>unavailable</error></root>",&part(),0,|_,_|Ok(())).is_err());
    }
    #[test]
    fn repeated_hour_page_is_not_empty_success_and_rounding_keeps_last_message() {
        let mut part=part(); part.duration_ms=7_200_000;
        let bytes=b"<root><version>2</version><chat><t>3599.9999</t><n>A</n><m>fixture</m></chat></root>";
        let mut rows=Vec::new(); parse(bytes,&part,0,|_,row|{if let Some(row)=row{rows.push(row)} Ok(())}).unwrap();
        assert_eq!(rows.len(),1); assert!(parse(bytes,&part,3600,|_,_|Ok(())).is_err());
    }
}
