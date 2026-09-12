//! Opt-in, bounded transport comparison, NOT a server-limit discovery/stress test.
//! Uses the first gallery from the site's PUBLIC index, at most 40 image GETs and 64 MiB total.
//! Does not decode/save images, access the user DB, retry errors, or follow redirects.
use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use atsumi_lib::source::hitomi::{
    galleryinfo_script_url, gg_script_url, index_all_nozomi_url, parse_galleryinfo_script,
    parse_gg_routing, webp_full_candidates,
};
use reqwest::blocking::{Client, Response};
use serde_json::{json, Value};

const TOTAL_BYTES: usize = 64 * 1024 * 1024;
const IMAGE_BYTES: usize = 4 * 1024 * 1024;
const SAMPLE_COUNT: usize = 5;

fn headers(response: &Response) -> Value {
    let names = [
        "retry-after",
        "ratelimit",
        "ratelimit-policy",
        "ratelimit-limit",
        "ratelimit-remaining",
        "ratelimit-reset",
        "x-ratelimit-limit",
        "x-ratelimit-remaining",
        "x-ratelimit-reset",
    ];
    let mut result = serde_json::Map::new();
    for name in names {
        if let Some(value) = response.headers().get(name).and_then(|v| v.to_str().ok()) {
            result.insert(name.into(), json!(value));
        }
    }
    Value::Object(result)
}

fn script(client: &Client, url: &str) -> Result<String, Box<dyn std::error::Error>> {
    let response = client.get(url).send()?;
    println!(
        "{}",
        json!({"kind":"metadata", "status":response.status().as_u16(), "limit_headers":headers(&response)})
    );
    if response.status().as_u16() != 200 {
        return Err("Metadata request failed; no retries".into());
    }
    let mut data = Vec::new();
    response.take(2 * 1024 * 1024 + 1).read_to_end(&mut data)?;
    if data.len() > 2 * 1024 * 1024 {
        return Err("Metadata body too large".into());
    }
    Ok(String::from_utf8(data)?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--public-sample"] {
        return Err("Opt in with --public-sample; private gallery IDs are not accepted".into());
    }
    let mut default_headers = reqwest::header::HeaderMap::new();
    default_headers.insert(reqwest::header::REFERER, "https://hitomi.la/".parse()?);
    let client = Client::builder()
        .user_agent(concat!(
            "Atsumi/",
            env!("CARGO_PKG_VERSION"),
            " (+desktop source adapter)"
        ))
        .default_headers(default_headers)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()?;
    // Independently select a public sample. Never derive outbound requests from user history.
    let response = client
        .get(index_all_nozomi_url())
        .header(reqwest::header::RANGE, "bytes=0-3")
        .send()?;
    println!(
        "{}",
        json!({"kind":"public_index", "status":response.status().as_u16(), "limit_headers":headers(&response)})
    );
    if response.status().as_u16() != 206
        || !response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|range| range.starts_with("bytes 0-3/"))
    {
        return Err("Public sample range request failed; no retries".into());
    }
    let mut index = Vec::new();
    response.take(5).read_to_end(&mut index)?;
    let id = u64::from(u32::from_be_bytes(
        index.try_into().map_err(|_| "Invalid public index range")?,
    ));
    let routing = parse_gg_routing(&script(&client, &gg_script_url())?)?;
    let gallery = parse_galleryinfo_script(&script(&client, &galleryinfo_script_url(id)?)?)?;
    if gallery.id != id {
        return Err("Gallery identity mismatch".into());
    }
    let urls = gallery
        .pages
        .iter()
        .take(SAMPLE_COUNT)
        .map(|page| {
            webp_full_candidates(page, &routing)
                .and_then(|candidates| {
                    candidates.into_iter().next().ok_or_else(|| {
                        atsumi_lib::source::SourceContractError::protocol(
                            "No primary WebP candidate",
                        )
                    })
                })
                .map(|candidate| candidate.url)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if urls.len() != SAMPLE_COUNT {
        return Err("Need five pages for an equal-size sample".into());
    }
    let urls = Arc::new(urls);
    let stop = Arc::new(AtomicBool::new(false));
    let transferred = Arc::new(AtomicUsize::new(0));
    let start_lock = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1)));
    let order = [1_usize, 3, 2, 5, 5, 2, 3, 1];
    println!(
        "{}",
        json!({"kind":"plan", "gallery_id":id, "concurrency_order":order, "images_per_round":SAMPLE_COUNT, "max_image_requests":40, "max_bytes":TOTAL_BYTES, "start_interval_ms":25})
    );
    for (round, concurrency) in order.into_iter().enumerate() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if round > 0 {
            thread::sleep(Duration::from_secs(2));
        }
        let next = Arc::new(AtomicUsize::new(0));
        let results = Arc::new(Mutex::new(Vec::new()));
        let begun = Instant::now();
        thread::scope(|scope| {
            for _ in 0..concurrency {
                let (client, urls, stop, transferred, next, results, start_lock) = (
                    client.clone(),
                    Arc::clone(&urls),
                    Arc::clone(&stop),
                    Arc::clone(&transferred),
                    Arc::clone(&next),
                    Arc::clone(&results),
                    Arc::clone(&start_lock),
                );
                scope.spawn(move || loop {
                    if stop.load(Ordering::SeqCst) || transferred.load(Ordering::SeqCst) >= TOTAL_BYTES { break; }
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(url) = urls.get(index) else { break; };
                    {
                        let mut last = start_lock.lock().unwrap();
                        if let Some(delay) = Duration::from_millis(25).checked_sub(last.elapsed()) { thread::sleep(delay); }
                        if stop.load(Ordering::SeqCst) { break; }
                        *last = Instant::now();
                    }
                    let started = Instant::now();
                    let mut bytes = 0_usize;
                    let result = (|| -> Result<Value, String> {
                        let mut response = client.get(url).send().map_err(|e| e.to_string())?;
                        let status = response.status().as_u16();
                        let limits = headers(&response);
                        let content_type = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
                        if status != 200 {
                            return Err(format!("HTTP {status}; limit_headers={limits}"));
                        }
                        if !content_type.starts_with("image/") { return Err("Non-image response".into()); }
                        let mut buffer = [0_u8; 16 * 1024];
                        loop {
                            if stop.load(Ordering::SeqCst) { return Err("Comparison stopped".into()); }
                            let desired = buffer.len().min(IMAGE_BYTES.saturating_sub(bytes));
                            if desired == 0 { return Err("Bounded image byte budget reached".into()); }
                            // Reserve before reading so simultaneous workers cannot overshoot
                            // the aggregate body budget. Return any unused reservation.
                            let previous = transferred.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |used| {
                                (used < TOTAL_BYTES).then(|| used + desired.min(TOTAL_BYTES - used))
                            }).map_err(|_| "Bounded total byte budget reached".to_owned())?;
                            let reserved = desired.min(TOTAL_BYTES - previous);
                            let read = response.read(&mut buffer[..reserved]);
                            let length = match read {
                                Ok(length) => length,
                                Err(error) => {
                                    transferred.fetch_sub(reserved, Ordering::SeqCst);
                                    return Err(error.to_string());
                                }
                            };
                            transferred.fetch_sub(reserved - length, Ordering::SeqCst);
                            if length == 0 { break; }
                            bytes += length;
                        }
                        Ok(json!({"status":status,"content_type":content_type,"limit_headers":limits}))
                    })();
                    if result.is_err() { stop.store(true, Ordering::SeqCst); }
                    results.lock().unwrap().push(json!({"sample":index,"bytes":bytes,"elapsed_ms":started.elapsed().as_millis(),"response":result.as_ref().ok(),"error":result.err()}));
                });
            }
        });
        let elapsed = begun.elapsed();
        let values = results.lock().unwrap();
        let round_bytes: u64 = values
            .iter()
            .map(|r| r["bytes"].as_u64().unwrap_or(0))
            .sum();
        println!(
            "{}",
            json!({"kind":"round", "round":round+1,"concurrency":concurrency,"elapsed_ms":elapsed.as_millis(),"bytes":round_bytes,"mib_per_second":round_bytes as f64 / 1048576.0 / elapsed.as_secs_f64(),"samples":*values})
        );
    }
    println!(
        "{}",
        json!({"kind":"end", "stopped_early":stop.load(Ordering::SeqCst),"image_bytes":transferred.load(Ordering::SeqCst),"disclaimer":"This is a bounded sample, not an official server limit or permission estimate."})
    );
    Ok(())
}
