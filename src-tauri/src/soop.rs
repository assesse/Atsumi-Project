//! Anonymous SOOP metadata and discovery. No account/session access.
//! Public responses checked 2026-10-13: chapi.sooplive.com/api/aflol/station.
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{Cursor, Read},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::Manager;
pub mod auto_record;
pub mod original;
pub mod original_commands;
pub(crate) mod chat;
pub mod discovery;
mod discovery_session;

const JSON_LIMIT: usize = 512 * 1024;
const IMAGE_LIMIT: usize = 128 * 1024;
// Public source portraits can exceed the small normalized IPC/storage payload.
// Keep decoded dimensions/allocation and the final PNG limit independently bounded.
const PROFILE_SOURCE_LIMIT: usize = 4 * 1024 * 1024;
const CACHE_LIMIT: usize = 128;
// SOOP's public station router returns a 404 HTML page for an unrecognized
// browser token (confirmed with identical requests on 2026-10-13). Retain
// Atsumi's identity while supplying its required browser-compatibility token.
const USER_AGENT: &str = "Mozilla/5.0 (compatible; Atsumi/2; public-channel-reader)";
static REQUEST_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
static CACHE: OnceLock<Mutex<HashMap<String, (Instant, SoopChannel)>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoopChannel {
    channel_id: String,
    name: String,
    title: Option<String>,
    image: Option<String>,
    pub(crate) status: &'static str,
    viewers: Option<u64>,
    pub(crate) broadcast_key: Option<String>,
    pub(crate) restricted: bool,
    #[serde(skip)]
    pub(crate) recording_profile_url: Option<String>,
}

fn valid_id(value: &str) -> bool {
    (1..=40).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn normalize_channel(input: &str) -> Result<String, String> {
    let input = input.trim();
    if valid_id(input) {
        return Ok(input.to_ascii_lowercase());
    }
    if input.len() > 2048 {
        return Err("SOOP 주소가 너무 깁니다.".into());
    }
    let address = if input.to_ascii_lowercase().starts_with("https://")
        || input.to_ascii_lowercase().starts_with("http://")
    {
        input.to_string()
    } else {
        format!("https://{input}")
    };
    let url = Url::parse(&address).map_err(|_| "SOOP 채널 링크 또는 ID를 확인해 주세요.")?;
    let host = url.host_str().unwrap_or_default();
    if !matches!(url.scheme(), "http" | "https")
        || !matches!(
            host,
            "play.sooplive.com"
                | "play.sooplive.co.kr"
                | "play.afreecatv.com"
                | "ch.sooplive.com"
                | "ch.sooplive.co.kr"
                | "bj.sooplive.co.kr"
                | "bj.afreecatv.com"
                | "www.sooplive.com"
                | "www.sooplive.co.kr"
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("SOOP 채널 링크만 사용할 수 있습니다.".into());
    }
    let mut parts: Vec<_> = url
        .path()
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if host.starts_with("www.") {
        if parts.first() != Some(&"station")
            || (parts.len() != 2 && !(parts.len() == 3 && parts[2] == "board"))
        {
            return Err("SOOP 채널 링크를 확인해 주세요.".into());
        }
        parts = parts[1..2].to_vec();
    }
    let Some(id) = parts.first().copied().filter(|id| valid_id(id)) else {
        return Err("SOOP 채널 ID를 확인해 주세요.".into());
    };
    if host.starts_with("play.") {
        if parts.len() > 3
            || parts.get(1).is_some_and(|part| {
                *part != "embed"
                    && (part.len() > 20 || !part.bytes().all(|byte| byte.is_ascii_digit()))
            })
            || parts
                .get(2)
                .is_some_and(|part| *part != "embed" || parts[1] == "embed")
        {
            return Err("SOOP 방송 링크를 확인해 주세요.".into());
        }
    } else if parts.len() != 1 {
        return Err("SOOP 채널 링크를 확인해 주세요.".into());
    }
    Ok(id.to_ascii_lowercase())
}

fn clean_text(value: Option<&str>, limit: usize) -> Option<String> {
    value
        .map(|text| {
            text.chars()
                .filter(|ch| !ch.is_control())
                .take(limit)
                .collect::<String>()
        })
        .filter(|text| !text.trim().is_empty())
}

fn parse_station(id: &str, value: &Value) -> Result<SoopChannel, String> {
    let station = value
        .get("station")
        .and_then(Value::as_object)
        .ok_or("채널을 찾을 수 없습니다.")?;
    let remote_id = station
        .get("user_id")
        .and_then(Value::as_str)
        .ok_or("채널 응답을 확인할 수 없습니다.")?;
    if !remote_id.eq_ignore_ascii_case(id) {
        return Err("요청한 채널과 응답이 일치하지 않습니다.".into());
    }
    let name = clean_text(station.get("user_nick").and_then(Value::as_str), 120)
        .unwrap_or_else(|| id.into());
    let broad = value.get("broad");
    let status = match broad {
        Some(Value::Null) => "offline",
        Some(Value::Object(fields))
            if fields.get("broad_no").is_some_and(|number| {
                number.as_u64().is_some_and(|n| n > 0)
                    || number
                        .as_str()
                        .is_some_and(|s| s.parse::<u64>().is_ok_and(|n| n > 0))
            }) =>
        {
            "live"
        }
        _ => "unknown",
    };
    let title = clean_text(
        broad
            .and_then(|broad| broad.get("broad_title"))
            .and_then(Value::as_str),
        240,
    );
    let viewers = broad
        .and_then(|broad| {
            broad
                .get("current_sum_viewer")
                .or_else(|| broad.get("current_view_cnt"))
        })
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()));
    let broadcast_key = broad
        .and_then(|b| b.get("broad_no"))
        .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
        .filter(|number| *number > 0)
        .map(|number| number.to_string());
    let restricted = broad.is_some_and(|b| {
        [
            "is_password",
            "password_yn",
            "ppv_yn",
            "fan_flag",
            "subs_flag",
            "broad_grade",
        ]
        .iter()
        .any(|field| {
            b.get(*field).is_some_and(|v| {
                v == "Y"
                    || v == "19"
                    || v.as_bool() == Some(true)
                    || v.as_u64().is_some_and(|n| n > 0)
                    || v.as_str()
                        .is_some_and(|s| s.parse::<u64>().is_ok_and(|n| n > 0))
            })
        })
    });
    Ok(SoopChannel {
        channel_id: id.into(),
        name,
        title,
        image: None,
        status,
        viewers,
        broadcast_key,
        restricted,
        recording_profile_url: value
            .get("profile_image")
            .and_then(Value::as_str)
            .and_then(profile_url)
            .map(|url| url.to_string()),
    })
}

// Background schedules need fresh station metadata, not thumbnails or a cached
// online/offline answer. Keep this anonymous and within the normal HTTP bound.
pub(crate) fn observe_channel(id: &str) -> Result<SoopChannel, String> {
    let id = normalize_channel(id)?;
    let bytes = bounded_get(
        &client()?,
        &format!("https://chapi.sooplive.com/api/{id}/station"),
        JSON_LIMIT,
    )?;
    let value =
        serde_json::from_slice(&bytes).map_err(|_| "SOOP 채널 응답을 확인할 수 없습니다.")?;
    parse_station(&id, &value)
}

fn client() -> Result<Client, String> {
    Client::builder()
        .redirect(Policy::none())
        .referer(false)
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(12))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|_| "SOOP 연결을 준비하지 못했습니다.".into())
}

fn bounded_get(client: &Client, url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|_| "SOOP에 연결하지 못했습니다. 잠시 후 다시 시도해 주세요.")?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            404 | 515 => "채널을 찾을 수 없습니다.",
            429 => "요청이 많습니다. 잠시 후 다시 시도해 주세요.",
            _ => "SOOP에서 채널을 불러오지 못했습니다.",
        }
        .into());
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err("SOOP 응답이 허용 크기를 초과했습니다.".into());
    }
    let mut bytes = Vec::new();
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "SOOP 응답을 읽지 못했습니다.")?;
    if bytes.len() > limit {
        return Err("SOOP 응답이 허용 크기를 초과했습니다.".into());
    }
    Ok(bytes)
}

fn profile_url(value: &str) -> Option<Url> {
    if value.len() > 2048 {
        return None;
    }
    let url = Url::parse(&if value.starts_with("//") {
        format!("https:{value}")
    } else {
        value.into()
    })
    .ok()?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str()?,
            "profile.img.sooplive.com" | "profile.img.sooplive.co.kr" | "profile.img.afreecatv.com"
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !url.path().starts_with("/LOGO/")
    {
        return None;
    }
    Some(url)
}

fn portrait(client: &Client, url: &str) -> Option<String> {
    let url = profile_url(url)?;
    let bytes = bounded_get(client, url.as_str(), PROFILE_SOURCE_LIMIT).ok()?;
    normalize_portrait(&bytes)
}

pub(crate) fn normalize_portrait(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > PROFILE_SOURCE_LIMIT {
        return None;
    }
    // Some SOOP .jpg URLs and image/jpeg responses contain a real GIF. Validate
    // its bounded container, then decode only its first frame into a small PNG.
    // URL suffix and response MIME are not trusted as decoder selection.
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        crate::streaming::chat_assets::validate_gif_source(bytes).ok()?;
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if !matches!(
        reader.format()?,
        image::ImageFormat::Jpeg
            | image::ImageFormat::Png
            | image::ImageFormat::WebP
            | image::ImageFormat::Gif
    ) {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(96, 96);
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, image::ImageFormat::Png).ok()?;
    if output.get_ref().len() > IMAGE_LIMIT {
        return None;
    }
    Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(output.into_inner())
    ))
}

pub(crate) fn recording_portrait(url: &str) -> Option<String> {
    let client = Client::builder()
        .redirect(Policy::none())
        .referer(false)
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(4))
        .user_agent(USER_AGENT)
        .build()
        .ok()?;
    portrait(&client, url)
}

fn inspect_channel(id: &str) -> Result<SoopChannel, String> {
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock() {
        if let Some((at, value)) = cache
            .get(id)
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(30))
        {
            let _ = at;
            return Ok(value.clone());
        }
    }
    let client = client()?;
    let bytes = bounded_get(
        &client,
        &format!("https://chapi.sooplive.com/api/{id}/station"),
        JSON_LIMIT,
    )?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "SOOP 채널 응답을 확인할 수 없습니다.")?;
    let mut channel = parse_station(id, &value)?;
    channel.image = value
        .get("profile_image")
        .and_then(Value::as_str)
        .and_then(|url| portrait(&client, url));
    if let Ok(mut cache) = cache.lock() {
        if cache.len() >= CACHE_LIMIT {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(id, _)| id.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(id.into(), (Instant::now(), channel.clone()));
    }
    Ok(channel)
}

#[tauri::command]
pub async fn soop_channel_inspect(
    window: tauri::Webview,
    input: String,
) -> Result<SoopChannel, String> {
    let url = window
        .url()
        .map_err(|_| "앱 창의 주소를 확인하지 못했습니다.")?;
    if !crate::frontend_origin::trusted_main(
        window.label(),
        &url,
        cfg!(debug_assertions),
        window.app_handle().config().build.dev_url.as_ref(),
    ) {
        return Err("SOOP 채널 정보는 Atsumi 기본 창에서만 확인할 수 있습니다.".into());
    }
    let channel = normalize_channel(&input)?;
    let _slot = REQUEST_SLOTS
        .try_acquire()
        .map_err(|_| "다른 채널을 확인 중입니다. 잠시 후 다시 시도해 주세요.")?;
    tauri::async_runtime::spawn_blocking(move || inspect_channel(&channel))
        .await
        .map_err(|_| "SOOP 채널 확인을 완료하지 못했습니다.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_current_and_legacy_player_links() {
        for input in [
            " AfLol ",
            "https://play.sooplive.com/aflol/12345",
            "play.sooplive.co.kr/aflol/embed",
            "http://play.afreecatv.com/aflol/123/embed",
            "https://bj.afreecatv.com/aflol",
            "https://ch.sooplive.com/aflol",
        ] {
            assert_eq!(normalize_channel(input).unwrap(), "aflol", "{input}");
        }
    }
    #[test]
    fn rejects_untrusted_or_unsupported_urls() {
        for input in [
            "",
            "https://play.sooplive.com.evil.test/aflol",
            "https://play.sooplive.com@evil.test/aflol",
            "https://evil@play.sooplive.com/aflol",
            "https://play.sooplive.com:444/aflol",
            "https://play.sooplive.com/aflol/abc",
            "https://ch.sooplive.com/aflol/posts",
            "file:///aflol",
            "https://play.sooplive.com/aflol/embed/embed",
        ] {
            assert!(normalize_channel(input).is_err(), "{input}");
        }
    }
    #[test]
    fn distinguishes_live_offline_unknown_without_guessing() {
        let station =
            serde_json::json!({"station":{"user_id":"aflol","user_nick":"LCK_KR"},"broad":null});
        assert_eq!(parse_station("aflol", &station).unwrap().status, "offline");
        let live = serde_json::json!({"station":{"user_id":"aflol","user_nick":"LCK_KR"},"broad":{"broad_no":"123","broad_title":"방송", "current_sum_viewer":12}});
        let parsed = parse_station("aflol", &live).unwrap();
        assert_eq!(parsed.status, "live");
        assert_eq!(parsed.viewers, Some(12));
        let unknown = serde_json::json!({"station":{"user_id":"aflol","user_nick":"LCK_KR"}});
        assert_eq!(parse_station("aflol", &unknown).unwrap().status, "unknown");
        assert!(parse_station("other", &station).is_err());
        assert!(parse_station("aflol", &serde_json::json!({"error":"missing"})).is_err());
    }
    #[test]
    fn portraits_are_https_first_party_rasters_only() {
        assert!(profile_url("//profile.img.sooplive.com/LOGO/af/aflol/aflol.jpg").is_some());
        for url in [
            "https://evil.test/image.png",
            "http://profile.img.sooplive.com/LOGO/a.jpg",
            "https://profile.img.sooplive.com.evil.test/LOGO/a.jpg",
            "https://profile.img.sooplive.com/api/test",
            "https://user@profile.img.sooplive.com/LOGO/a.jpg",
            "https://profile.img.sooplive.com:444/LOGO/a.jpg",
        ] {
            assert!(profile_url(url).is_none(), "{url}");
        }
    }
    fn gif_portrait_fixture() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for color in [[190, 30, 20, 255], [10, 180, 30, 255]] {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                        600,
                        600,
                        image::Rgba(color),
                    )))
                    .unwrap();
            }
        }
        bytes
    }
    #[test]
    fn mislabelled_large_gif_portrait_is_normalized_from_only_the_first_frame() {
        let bytes = gif_portrait_fixture();
        assert!(bytes.len() < IMAGE_LIMIT);
        let result = normalize_portrait(&bytes).unwrap();
        let bytes = STANDARD
            .decode(result.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (96, 96));
        assert_eq!(image.get_pixel(48, 48).0, [190, 30, 20, 255]);
        assert!(bytes.len() <= IMAGE_LIMIT);
    }
    #[test]
    fn portrait_rejects_truncated_oversized_or_non_raster_inputs() {
        let bytes = gif_portrait_fixture();
        for length in [0, 5, 13, bytes.len() / 2, bytes.len() - 1] {
            assert!(normalize_portrait(&bytes[..length]).is_none());
        }
        let mut oversized = bytes.clone();
        oversized.resize(PROFILE_SOURCE_LIMIT + 1, 0);
        assert!(normalize_portrait(&oversized).is_none());
        let mut huge_dimensions = bytes;
        huge_dimensions[6..8].copy_from_slice(&8192u16.to_le_bytes());
        huge_dimensions[8..10].copy_from_slice(&8192u16.to_le_bytes());
        assert!(normalize_portrait(&huge_dimensions).is_none());
        assert!(normalize_portrait(b"<svg xmlns='http://www.w3.org/2000/svg'/>").is_none());
    }
    #[test]
    fn public_source_larger_than_normalized_limit_remains_a_small_portrait() {
        let mut state = 0x736f6f70u32;
        let image = image::RgbImage::from_fn(1024, 1024, |_, _| {
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            };
            image::Rgb([next(), next(), next()])
        });
        let mut input = Cursor::new(Vec::new());
        image.write_to(&mut input, image::ImageFormat::Png).unwrap();
        assert!(input.get_ref().len() > 512 * 1024);
        assert!(input.get_ref().len() <= PROFILE_SOURCE_LIMIT);
        let data = normalize_portrait(input.get_ref()).unwrap();
        let png = STANDARD.decode(data.strip_prefix("data:image/png;base64,").unwrap()).unwrap();
        assert!(png.len() <= IMAGE_LIMIT);
        let output = image::load_from_memory(&png).unwrap();
        assert_eq!((output.width(), output.height()), (96, 96));
    }
    #[test]
    fn large_gif_source_keeps_shared_limit_and_normalizes_only_first_frame() {
        let mut input = gif_portrait_fixture();
        assert_eq!(input.pop(), Some(0x3b));
        input.extend_from_slice(&[0x21, 0xfe]);
        while input.len() <= 1024 * 1024 {
            input.push(255);
            input.extend_from_slice(&[b'x'; 255]);
        }
        input.extend_from_slice(&[0, 0x3b]);
        assert!(input.len() > 1024 * 1024 && input.len() <= PROFILE_SOURCE_LIMIT);
        assert!(crate::streaming::chat_assets::validate_raster(&input, None).is_err());
        assert!(crate::streaming::chat_assets::validate_gif_source(&input).is_ok());
        let data = normalize_portrait(&input).unwrap();
        let png = STANDARD
            .decode(data.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        let image = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (96, 96));
        assert_eq!(image.get_pixel(48, 48).0, [190, 30, 20, 255]);
        assert!(png.len() <= IMAGE_LIMIT);
    }
    #[test]
    fn portrait_keeps_decoder_dimension_and_allocation_limits() {
        for (width, height) in [(4097, 1), (1, 4097), (3072, 3072)] {
            let mut input = Cursor::new(Vec::new());
            image::DynamicImage::new_rgba8(width, height)
                .write_to(&mut input, image::ImageFormat::Png)
                .unwrap();
            assert!(input.get_ref().len() <= PROFILE_SOURCE_LIMIT);
            assert!(
                normalize_portrait(input.get_ref()).is_none(),
                "source {width}x{height} must retain dimension/allocation rejection"
            );
        }
    }
    #[test]
    #[ignore = "Explicit three public profiles only; no accounts, playback, media or image-file writes"]
    fn public_selected_profile_sources_normalize_with_bounded_gif_source() {
        let client = client().unwrap();
        for id in ["ozan201", "sikhye1004", "bmwy00"] {
            let station = bounded_get(
                &client,
                &format!("https://chapi.sooplive.com/api/{id}/station"),
                JSON_LIMIT,
            )
            .expect("selected public station should respond");
            let value: Value = serde_json::from_slice(&station).unwrap();
            let channel = parse_station(id, &value).unwrap();
            assert_eq!(channel.channel_id, id);
            let url = value
                .get("profile_image")
                .and_then(Value::as_str)
                .and_then(profile_url)
                .expect("selected station must provide an admitted public portrait");
            let input = bounded_get(&client, url.as_str(), PROFILE_SOURCE_LIMIT)
                .expect("selected public portrait must remain within the source bound");
            let format = image::guess_format(&input).unwrap();
            let data = normalize_portrait(&input)
                .expect("selected public portrait must pass structural and first-frame decode guards");
            let png = STANDARD
                .decode(data.strip_prefix("data:image/png;base64,").unwrap())
                .unwrap();
            let image = image::load_from_memory(&png).unwrap();
            assert!(image.width() > 0 && image.width() <= 96);
            assert!(image.height() > 0 && image.height() <= 96);
            assert!(png.len() <= IMAGE_LIMIT);
            println!(
                "{}",
                serde_json::json!({
                    "channelId": id,
                    "sourceFormat": format!("{format:?}"),
                    "sourceBytes": input.len(),
                    "normalizedWidth": image.width(),
                    "normalizedHeight": image.height(),
                    "normalizedBytes": png.len()
                })
            );
        }
    }
    #[test]
    #[ignore = "explicit anonymous townboy portrait only; no account, playback or recording"]
    fn public_townboy_portrait_normalizes_with_bounded_source() {
        let data = recording_portrait("https://profile.img.sooplive.com/LOGO/to/townboy/townboy.jpg")
            .expect("public townboy portrait should decode");
        let png = STANDARD.decode(data.strip_prefix("data:image/png;base64,").unwrap()).unwrap();
        let image = image::load_from_memory(&png).unwrap();
        assert!(image.width() <= 96 && image.height() <= 96);
        assert!(png.len() <= IMAGE_LIMIT);
        println!("SOOP townboy normalized portrait: {}x{}, {}bytes", image.width(), image.height(), png.len());
    }
    #[test]
    #[ignore = "Explicit bounded public portrait only; no account, playback or recording"]
    fn public_large_gif_portrait_normalizes_to_png() {
        let image =
            recording_portrait("https://profile.img.sooplive.com/LOGO/ce/cevin8000/cevin8000.jpg")
                .expect("public portrait must be available and decodable");
        let bytes = STANDARD
            .decode(image.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        let raster = image::load_from_memory(&bytes).unwrap();
        assert!(raster.width() > 0 && raster.width() <= 96);
        assert!(raster.height() > 0 && raster.height() <= 96);
        assert!(bytes.len() <= IMAGE_LIMIT);
        println!(
            "normalized portrait: {}x{}, {}bytes",
            raster.width(),
            raster.height(),
            bytes.len()
        );
    }
    #[test]
    fn recording_metadata_has_stable_key_and_does_not_serialize_profile_source() {
        let mut response = serde_json::json!({"station":{"user_id":"fixture","user_nick":"Fixture"},"broad":{"broad_no":"000123","broad_grade":0},"profile_image":"https://profile.img.sooplive.com/LOGO/fi/fixture/fixture.jpg"});
        let channel = parse_station("fixture", &response).unwrap();
        assert_eq!(channel.broadcast_key.as_deref(), Some("123"));
        assert!(!channel.restricted);
        assert!(channel.recording_profile_url.is_some());
        let serialized = serde_json::to_string(&channel).unwrap();
        assert!(!serialized.contains("profile.img"));
        for (key, value) in [
            ("is_password", serde_json::json!(true)),
            ("ppv_yn", serde_json::json!("Y")),
            ("broad_grade", serde_json::json!(19)),
        ] {
            response["broad"][key] = value;
            assert!(parse_station("fixture", &response).unwrap().restricted);
            response["broad"].as_object_mut().unwrap().remove(key);
        }
        response["profile_image"] = serde_json::json!("https://untrusted.invalid/image");
        assert!(parse_station("fixture", &response)
            .unwrap()
            .recording_profile_url
            .is_none());
    }
    #[test]
    #[ignore = "Explicit anonymous public API smoke test; no login, playback or recording"]
    fn public_station_smoke() {
        let channel = inspect_channel("aflol").unwrap();
        assert_eq!(channel.channel_id, "aflol");
        assert!(!channel.name.is_empty());
        assert!(matches!(channel.status, "live" | "offline" | "unknown"));
        assert!(channel.image.as_ref().is_none_or(|image| image
            .starts_with("data:image/png;base64,")
            && image.len() < 180_000));
        println!(
            "{}",
            serde_json::json!({"kind":"soop_public_station_smoke","channelId":channel.channel_id,"status":channel.status,"profileImageBytes":channel.image.as_ref().map(String::len),"name":channel.name})
        );
    }
    #[test]
    #[ignore = "Explicit anonymous live metadata probe; channel must currently be public/live"]
    fn public_live_station_smoke() {
        let id = std::env::var("ATSUMI_SOOP_LIVE_SMOKE_CHANNEL")
            .expect("explicit live channel required");
        let id = normalize_channel(&id).unwrap();
        let channel = inspect_channel(&id).unwrap();
        assert_eq!(
            channel.status, "live",
            "the selected public channel is no longer live"
        );
        assert!(!channel.name.is_empty());
        assert!(channel.title.is_some());
        assert!(channel
            .image
            .as_ref()
            .is_some_and(|image| (image.starts_with("data:image/png;base64,")
                || image.starts_with("data:image/gif;base64,"))
                && image.len() < 180_000));
        println!(
            "{}",
            serde_json::json!({"kind":"soop_public_live_smoke","channelId":channel.channel_id,"status":channel.status,"profileImageBytes":channel.image.as_ref().map(String::len),"viewers":channel.viewers})
        );
    }
}
