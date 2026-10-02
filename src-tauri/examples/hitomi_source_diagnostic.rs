//! Explicit, bounded read-only source probe. No DB, downloads, retries or files.
use atsumi_lib::source::hitomi::{
    download_full_candidates, galleryinfo_script_url, gg_script_url, parse_galleryinfo_script,
    parse_gg_routing,
};
use std::{
    io::{Cursor, Read},
    time::Duration,
};
fn get(
    client: &reqwest::blocking::Client,
    url: &str,
    limit: u64,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let response = client
        .get(url)
        .header("Referer", "https://hitomi.la/")
        .send()?
        .error_for_status()?;
    if url.ends_with(".webp") {
        println!(
            "response: status={} length={:?} range={:?} etag={:?}",
            response.status().as_u16(),
            response.headers().get("content-length"),
            response.headers().get("content-range"),
            response.headers().get("etag")
        );
    }
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("response too large".into());
    }
    Ok(bytes)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Atsumi/source-diagnostic")
        .referer(true)
        .build()?;
    for id in [4180352, 2897405] {
        let script =
            String::from_utf8(get(&client, &galleryinfo_script_url(id)?, 2 * 1024 * 1024)?)?;
        let metadata = match parse_galleryinfo_script(&script) {
            Ok(metadata) => {
                println!("metadata {id}: OK pages={}", metadata.pages.len());
                metadata
            }
            Err(error) => {
                println!("metadata {id}: {error:?}");
                continue;
            }
        };
        use atsumi_lib::{
            application::DownloadSourcePort,
            domain::GalleryId,
            infrastructure::{HitomiLiveAdapter, HitomiLiveConfig},
            thumbnail::CancellationToken,
        };
        let adapter = HitomiLiveAdapter::new(HitomiLiveConfig {
            max_retries: 0,
            request_timeout: Duration::from_secs(10),
            ..Default::default()
        })?;
        match adapter.gallery_snapshot(GalleryId::new(id as i64)?, &CancellationToken::default()) {
            Ok(snapshot) => println!("download snapshot {id}: OK pages={}", snapshot.pages.len()),
            Err(error) => println!("download snapshot {id}: {error:?}"),
        }
        if id == 2897405 && std::env::args().any(|a| a == "--production-pages") {
            for number in [11, 14] {
                match adapter.download_page(
                    GalleryId::new(id as i64)?,
                    atsumi_lib::domain::SourcePageNumber::new(number)?,
                    &CancellationToken::default(),
                ) {
                    Ok(page) => println!(
                        "production {id}/{number}: OK bytes={} dimensions={}x{} format={:?}",
                        page.bytes.len(),
                        page.width,
                        page.height,
                        page.source_format
                    ),
                    Err(error) => println!("production {id}/{number}: {error:?}"),
                }
            }
        }
        if id != 2897405 || !std::env::args().any(|a| a == "--primary-images") {
            continue;
        }
        let routing = parse_gg_routing(&String::from_utf8(get(
            &client,
            &gg_script_url(),
            2 * 1024 * 1024,
        )?)?)?;
        for page_number in [11, 14] {
            let page = metadata.page(page_number)?;
            let candidates = download_full_candidates(page, &routing)?;
            println!(
                "declared {id}/{page_number}: width={:?} height={:?}",
                page.width, page.height
            );
            let candidate_limit = if std::env::args().any(|a| a == "--all-candidates") {
                32
            } else {
                4
            };
            for (index, candidate) in candidates.iter().take(candidate_limit).enumerate() {
                if std::env::args().any(|a| a == "--tail-check") && index > 0 {
                    break;
                }
                let bytes = match get(&client, &candidate.url, 64 * 1024 * 1024) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        println!("image {id}/{page_number} candidate={index}: {error}");
                        continue;
                    }
                };
                if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
                    let expected = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize + 8;
                    println!("RIFF expected={expected} received={}", bytes.len());
                    if expected > bytes.len()
                        && expected <= 64 * 1024 * 1024
                        && std::env::args().any(|a| a == "--tail-check")
                    {
                        let response = client
                            .get(&candidate.url)
                            .header("Referer", "https://hitomi.la/")
                            .header("Range", format!("bytes={}-{}", bytes.len(), expected - 1))
                            .send()?;
                        let status = response.status().as_u16();
                        let range = response
                            .headers()
                            .get("content-range")
                            .and_then(|h| h.to_str().ok())
                            .unwrap_or("")
                            .to_owned();
                        let mut tail = Vec::new();
                        response.take(expected as u64 + 1).read_to_end(&mut tail)?;
                        println!("tail: status={status} range={range:?} bytes={}", tail.len());
                        if status == 206
                            && range == format!("bytes {}-{}/{expected}", bytes.len(), expected - 1)
                            && tail.len() + bytes.len() == expected
                        {
                            let mut combined = bytes.clone();
                            combined.extend(tail);
                            match image::load_from_memory(&combined) {
                                Ok(image) => println!(
                                    "tail-combined: OK dimensions={}x{}",
                                    image.width(),
                                    image.height()
                                ),
                                Err(error) => println!("tail-combined: {error:?}"),
                            }
                        }
                    }
                }
                let mut reader =
                    image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?;
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(16384);
                limits.max_image_height = Some(16384);
                limits.max_alloc = Some(256 * 1024 * 1024);
                reader.limits(limits);
                match reader.decode() {
                    Ok(image) => println!(
                        "image {id}/{page_number} candidate={index}: OK bytes={} dimensions={}x{}",
                        bytes.len(),
                        image.width(),
                        image.height()
                    ),
                    Err(error) => println!(
                        "image {id}/{page_number} candidate={index}: bytes={} decode={error:?}",
                        bytes.len()
                    ),
                }
            }
        }
    }
    Ok(())
}
