//! Anonymous, bounded raster image loading for untrusted rich-chat metadata.
//! Call `fetch_chat_asset` on a blocking worker; it never writes to disk.

use std::{
    collections::VecDeque,
    io::{Cursor, Read},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use image::{ImageFormat, ImageReader, Limits};
use reqwest::{blocking::Client, header::CONTENT_TYPE, redirect::Policy, Url};
use serde::Serialize;

use super::model::StreamError;

const MAX_URL_BYTES: usize = 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_DIMENSION: u32 = 1024;
const MAX_FRAMES: usize = 256;
const MAX_ANIMATION_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 256;
const MAX_CONCURRENT: usize = 4;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const DNS_TIMEOUT: Duration = Duration::from_secs(2);

static CACHE: OnceLock<Mutex<AssetCache>> = OnceLock::new();
static NETWORK_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static DNS_ACTIVE: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatAsset {
    pub data_url: String,
}

/// The same allowlist is used when accepting metadata and immediately before I/O.
pub(crate) fn sanitize_chat_asset_url(input: &str) -> Option<String> {
    if input.len() > MAX_URL_BYTES
        || input
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        || input.contains('\\')
    {
        return None;
    }
    let url = Url::parse(input).ok()?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("ssl.pstatic.net" | "nng-phinf.pstatic.net")
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
        || url.fragment().is_some()
        || url.as_str().len() > MAX_URL_BYTES
    {
        return None;
    }
    if let Some(query) = url.query() {
        // Naver's image CDN uses type= for thumbnail size presets. Arbitrary
        // query parameters (including credentials or remote URLs) are excluded.
        let value = query.strip_prefix("type=")?;
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
        {
            return None;
        }
    }
    Some(url.into())
}

pub fn fetch_chat_asset(input: &str) -> Result<ChatAsset, StreamError> {
    let key = sanitize_chat_asset_url(input).ok_or_else(invalid_asset)?;
    let cache = CACHE.get_or_init(|| Mutex::new(AssetCache::default()));
    if let Some(asset) = cache.lock().map_err(|_| unavailable())?.get(&key) {
        return Ok(asset);
    }

    // Fail promptly instead of queuing an unbounded number of network workers.
    let _permit = Permit::acquire(&NETWORK_ACTIVE)?;
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let url = Url::parse(&key).map_err(|_| invalid_asset())?;
    let host = url.host_str().ok_or_else(invalid_asset)?;
    let addresses = resolve_public_host(host)?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(unavailable)?;
    let client = Client::builder()
        .https_only(true)
        .redirect(Policy::none())
        .no_proxy()
        .connect_timeout(remaining.min(Duration::from_secs(3)))
        .timeout(remaining)
        // Pin the validated addresses to prevent a second DNS lookup/rebinding.
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| unavailable())?;
    let response = client
        .get(url)
        .header(
            reqwest::header::ACCEPT,
            "image/png,image/jpeg,image/gif,image/webp",
        )
        .send()
        .map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(unavailable());
    }
    let declared_type = response
        .headers()
        .get(CONTENT_TYPE)
        .map(|value| value.to_str().map(str::to_owned))
        .transpose()
        .map_err(|_| invalid_asset())?;
    let content_length = response.content_length();
    let bytes = read_bounded(response, content_length)?;
    let mime = validate_raster(&bytes, declared_type.as_deref())?;
    let asset = ChatAsset {
        data_url: format!("data:{mime};base64,{}", STANDARD.encode(bytes)),
    };
    // Network and decoding happen outside the cache lock.
    cache
        .lock()
        .map_err(|_| unavailable())?
        .insert(key, asset.clone());
    Ok(asset)
}

struct Permit(&'static AtomicUsize);

impl Permit {
    fn acquire(counter: &'static AtomicUsize) -> Result<Self, StreamError> {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_CONCURRENT).then_some(active + 1)
            })
            .map(|_| Self(counter))
            .map_err(|_| unavailable())
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn resolve_public_host(host: &str) -> Result<Vec<SocketAddr>, StreamError> {
    let permit = Permit::acquire(&DNS_ACTIVE)?;
    let host = host.to_owned();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("chat-asset-dns".into())
        .spawn(move || {
            // OS DNS cannot be cancelled. Keep this permit until it returns,
            // even after the receiver times out, to bound lingering threads.
            let _permit = permit;
            let addresses = (host.as_str(), 443)
                .to_socket_addrs()
                .map(|addresses| addresses.take(16).collect::<Vec<_>>());
            let _ = sender.send(addresses);
        })
        .map_err(|_| unavailable())?;
    let addresses = receiver
        .recv_timeout(DNS_TIMEOUT)
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
    if addresses.is_empty() || !addresses.iter().all(|address| is_public_ip(address.ip())) {
        return Err(invalid_asset());
    }
    Ok(addresses)
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || a == 0
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(ip) => {
            // Only ordinary global unicast is needed for these public CDNs.
            // Exclude mapped IPv4, local ranges, translation/tunnelling and docs.
            let parts = ip.segments();
            (parts[0] & 0xe000) == 0x2000
                && parts[0] != 0x2002
                && !(parts[0] == 0x2001 && parts[1] < 0x0200)
                && !(parts[0] == 0x2001 && parts[1] == 0x0db8)
                && !(parts[0] == 0x3fff && parts[1] < 0x1000)
        }
    }
}

fn read_bounded(reader: impl Read, content_length: Option<u64>) -> Result<Vec<u8>, StreamError> {
    if content_length.is_some_and(|length| length > MAX_BODY_BYTES as u64) {
        return Err(invalid_asset());
    }
    let mut bytes = Vec::new();
    reader
        .take((MAX_BODY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.is_empty() || bytes.len() > MAX_BODY_BYTES {
        return Err(invalid_asset());
    }
    Ok(bytes)
}

pub(crate) fn validate_raster(
    bytes: &[u8],
    declared_type: Option<&str>,
) -> Result<&'static str, StreamError> {
    if bytes.is_empty() || bytes.len() > MAX_BODY_BYTES {
        return Err(invalid_asset());
    }
    let format = image::guess_format(bytes).map_err(|_| invalid_asset())?;
    let mime = match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        _ => return Err(invalid_asset()),
    };
    if let Some(declared) = declared_type {
        if declared.len() > 128 {
            return Err(invalid_asset());
        }
        let declared = declared.split(';').next().unwrap_or_default().trim();
        if !declared.eq_ignore_ascii_case(mime)
            && !declared.eq_ignore_ascii_case("application/octet-stream")
        {
            return Err(invalid_asset());
        }
    }
    if format == ImageFormat::Gif {
        validate_gif(bytes)?;
    } else {
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_DIMENSION);
        limits.max_image_height = Some(MAX_DIMENSION);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode().map_err(|_| invalid_asset())?;
        validate_dimensions(image.width(), image.height())?;
        validate_animation(bytes, format, image.width(), image.height())?;
    }
    Ok(mime)
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), StreamError> {
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(invalid_asset());
    }
    Ok(())
}

fn validate_animation_pixels(width: u32, height: u32, frames: usize) -> Result<(), StreamError> {
    if frames > MAX_FRAMES
        || u64::from(width) * u64::from(height) * frames as u64 > MAX_ANIMATION_PIXELS
    {
        return Err(invalid_asset());
    }
    Ok(())
}

fn validate_animation(
    bytes: &[u8],
    format: ImageFormat,
    width: u32,
    height: u32,
) -> Result<(), StreamError> {
    // Decode() validates only the initial animation frame. Also bound every
    // declared frame before giving the original APNG/WebP bytes to the webview.
    if !matches!(format, ImageFormat::Png | ImageFormat::WebP) {
        return Ok(());
    }
    let png = format == ImageFormat::Png;
    let mut offset = if png { 8 } else { 12 };
    let mut frames = 0;
    let mut declared_frames = None;
    while offset < bytes.len() {
        let header = bytes.get(offset..offset + 8).ok_or_else(invalid_asset)?;
        let (kind, length) = if png {
            (
                &header[4..8],
                u32::from_be_bytes(header[..4].try_into().unwrap()) as usize,
            )
        } else {
            (
                &header[..4],
                u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize,
            )
        };
        let start = offset + 8;
        let end = start.checked_add(length).ok_or_else(invalid_asset)?;
        let data = bytes.get(start..end).ok_or_else(invalid_asset)?;
        if png && kind == b"acTL" {
            if data.len() != 8 || declared_frames.is_some() {
                return Err(invalid_asset());
            }
            let count = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
            if count == 0 {
                return Err(invalid_asset());
            }
            validate_animation_pixels(width, height, count)?;
            declared_frames = Some(count);
        }
        if (png && kind == b"fcTL") || (!png && kind == b"ANMF") {
            let (frame_width, frame_height, left, top) = if png {
                if data.len() != 26 {
                    return Err(invalid_asset());
                }
                let integer = |at| u32::from_be_bytes(data[at..at + 4].try_into().unwrap());
                (integer(4), integer(8), integer(12), integer(16))
            } else {
                if data.len() < 16 {
                    return Err(invalid_asset());
                }
                let integer = |at| u32::from_le_bytes([data[at], data[at + 1], data[at + 2], 0]);
                (
                    integer(6) + 1,
                    integer(9) + 1,
                    integer(0) * 2,
                    integer(3) * 2,
                )
            };
            validate_dimensions(frame_width, frame_height)?;
            if u64::from(left) + u64::from(frame_width) > u64::from(width)
                || u64::from(top) + u64::from(frame_height) > u64::from(height)
            {
                return Err(invalid_asset());
            }
            frames += 1;
            validate_animation_pixels(width, height, frames)?;
        }
        offset = end
            .checked_add(if png { 4 } else { length & 1 })
            .ok_or_else(invalid_asset)?;
    }
    if offset != bytes.len() || (png && declared_frames.unwrap_or(0) != frames) {
        return Err(invalid_asset());
    }
    Ok(())
}

fn validate_gif(bytes: &[u8]) -> Result<(), StreamError> {
    // GIF decoding is not an enabled image-crate feature. Check the complete
    // bounded container without allocating frame buffers; the webview decodes it.
    if bytes.len() < 14 || !matches!(&bytes[..6], b"GIF87a" | b"GIF89a") {
        return Err(invalid_asset());
    }
    let dimension = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]) as u32;
    let (width, height) = (dimension(6), dimension(8));
    validate_dimensions(width, height)?;
    let mut offset = 13;
    if bytes[10] & 0x80 != 0 {
        offset += 3 * (1usize << ((bytes[10] & 7) + 1));
    }
    let mut frames = 0;
    loop {
        match bytes.get(offset).copied() {
            Some(0x3b) if frames > 0 && offset + 1 == bytes.len() => return Ok(()),
            Some(0x21) => {
                if bytes.get(offset + 1).is_none() {
                    return Err(invalid_asset());
                }
                offset += 2;
                skip_gif_blocks(bytes, &mut offset)?;
            }
            Some(0x2c) => {
                if offset + 10 > bytes.len() {
                    return Err(invalid_asset());
                }
                let (left, top) = (dimension(offset + 1), dimension(offset + 3));
                let (frame_width, frame_height) = (dimension(offset + 5), dimension(offset + 7));
                validate_dimensions(frame_width, frame_height)?;
                if left + frame_width > width || top + frame_height > height {
                    return Err(invalid_asset());
                }
                let packed = bytes[offset + 9];
                offset += 10;
                if packed & 0x80 != 0 {
                    offset += 3 * (1usize << ((packed & 7) + 1));
                }
                if !bytes.get(offset).is_some_and(|size| (2..=8).contains(size)) {
                    return Err(invalid_asset());
                }
                offset += 1;
                if bytes.get(offset).is_none_or(|size| *size == 0) {
                    return Err(invalid_asset());
                }
                skip_gif_blocks(bytes, &mut offset)?;
                frames += 1;
                validate_animation_pixels(width, height, frames)?;
            }
            _ => return Err(invalid_asset()),
        }
    }
}

fn skip_gif_blocks(bytes: &[u8], offset: &mut usize) -> Result<(), StreamError> {
    loop {
        let size = *bytes.get(*offset).ok_or_else(invalid_asset)? as usize;
        *offset += 1;
        if size == 0 {
            return Ok(());
        }
        *offset += size;
        if *offset > bytes.len() {
            return Err(invalid_asset());
        }
    }
}

#[derive(Default)]
struct AssetCache {
    entries: VecDeque<(String, ChatAsset)>,
    bytes: usize,
}

impl AssetCache {
    fn get(&mut self, key: &str) -> Option<ChatAsset> {
        let index = self.entries.iter().position(|(entry, _)| entry == key)?;
        let entry = self.entries.remove(index)?;
        let asset = entry.1.clone();
        self.entries.push_back(entry);
        Some(asset)
    }

    fn insert(&mut self, key: String, asset: ChatAsset) {
        let size = key.len() + asset.data_url.len();
        if size > MAX_CACHE_BYTES {
            return;
        }
        if let Some(index) = self.entries.iter().position(|(entry, _)| *entry == key) {
            if let Some((previous_key, previous)) = self.entries.remove(index) {
                self.bytes -= previous_key.len() + previous.data_url.len();
            }
        }
        while self.entries.len() >= MAX_CACHE_ENTRIES || self.bytes + size > MAX_CACHE_BYTES {
            if let Some((old_key, old)) = self.entries.pop_front() {
                self.bytes -= old_key.len() + old.data_url.len();
            } else {
                break;
            }
        }
        self.bytes += size;
        self.entries.push_back((key, asset));
    }
}

fn invalid_asset() -> StreamError {
    StreamError::new(
        "CHAT_ASSET_INVALID",
        "지원하지 않는 채팅 이미지입니다.",
        false,
    )
}

fn unavailable() -> StreamError {
    StreamError::new(
        "CHAT_ASSET_UNAVAILABLE",
        "채팅 이미지를 불러오지 못했습니다.",
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIF: &[u8] = &[
        b'G', b'I', b'F', b'8', b'9', b'a', 1, 0, 1, 0, 0x80, 0, 0, 0, 0, 0, 255, 255, 255, 0x2c,
        0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 1, 0, 0x3b,
    ];

    #[test]
    fn accepts_only_exact_public_cdn_urls_and_bounded_resize_queries() {
        for input in [
            "https://ssl.pstatic.net/static/badge.png",
            "https://nng-phinf.pstatic.net/emoticon/a.gif?type=f160_160",
        ] {
            assert_eq!(sanitize_chat_asset_url(input).as_deref(), Some(input));
        }
        for input in [
            "http://ssl.pstatic.net/a.png",
            "https://ssl.pstatic.net.evil.example/a.png",
            "https://evil.ssl.pstatic.net/a.png",
            "https://ssl.pstatic.net./a.png",
            "https://127.0.0.1/a.png",
            "https://[::1]/a.png",
            "https://user:secret@ssl.pstatic.net/a.png",
            "https://ssl.pstatic.net:444/a.png",
            "https://ssl.pstatic.net/a.png#fragment",
            "https://ssl.pstatic.net/a.png?token=secret",
            "https://ssl.pstatic.net/a.png?type=f160&url=https://localhost/",
            "https://ssl.pstatic.net/a.png?type=%66",
            "https://ssl.pstatic.net/a.png?type=",
            " https://ssl.pstatic.net/a.png",
            "https://ssl.pstatic.net\\@localhost/a.png",
        ] {
            assert!(sanitize_chat_asset_url(input).is_none(), "{input}");
        }
        assert!(
            sanitize_chat_asset_url(&format!("https://ssl.pstatic.net/{}", "a".repeat(1024)))
                .is_none()
        );
    }

    #[test]
    fn dns_rejects_private_and_special_ranges_in_both_families() {
        for ip in [
            "0.1.2.3",
            "10.0.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.1.2",
            "192.168.0.1",
            "100.64.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "ff02::1",
            "64:ff9b::a00:1",
            "2001:db8::1",
            "2002:a00:1::",
            "3fff::1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "2606:4700::1111",
            "2001:4860:4860::8888",
        ] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn body_limit_applies_even_when_content_length_is_missing_or_false() {
        assert_eq!(read_bounded(Cursor::new([1, 2]), None).unwrap(), vec![1, 2]);
        assert!(read_bounded(Cursor::new([]), None).is_err());
        assert!(read_bounded(Cursor::new([1]), Some(MAX_BODY_BYTES as u64 + 1)).is_err());
        let bytes = vec![0; MAX_BODY_BYTES + 2];
        assert!(read_bounded(Cursor::new(&bytes), None).is_err());
        assert!(read_bounded(Cursor::new(bytes), Some(1)).is_err());
    }

    #[test]
    fn raster_magic_and_decoding_override_untrusted_mime() {
        for (format, mime) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::Jpeg, "image/jpeg"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let mut output = Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(2, 2)
                .write_to(&mut output, format)
                .unwrap();
            assert_eq!(validate_raster(output.get_ref(), Some(mime)).unwrap(), mime);
            assert_eq!(validate_raster(output.get_ref(), None).unwrap(), mime);
            assert!(validate_raster(output.get_ref(), Some("text/html")).is_err());
            assert!(validate_raster(output.get_ref(), Some("image/svg+xml")).is_err());
            assert!(validate_raster(&output.get_ref()[..16], Some(mime)).is_err());
        }
        assert!(validate_raster(
            b"<svg xmlns='http://www.w3.org/2000/svg'/>",
            Some("image/png")
        )
        .is_err());
        assert!(validate_raster(
            b"<!doctype html><script>alert(1)</script>",
            Some("image/png")
        )
        .is_err());
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(MAX_DIMENSION + 1, 1)
            .write_to(&mut output, ImageFormat::Png)
            .unwrap();
        assert!(validate_raster(output.get_ref(), Some("image/png")).is_err());
    }

    #[test]
    fn gif_requires_complete_bounded_frames() {
        assert_eq!(
            validate_raster(GIF, Some("image/gif")).unwrap(),
            "image/gif"
        );
        for length in 0..GIF.len() {
            assert!(validate_raster(&GIF[..length], Some("image/gif")).is_err());
        }
        let mut oversized = GIF.to_vec();
        oversized[6..8].copy_from_slice(&1025u16.to_le_bytes());
        assert!(validate_raster(&oversized, None).is_err());
        let mut out_of_bounds = GIF.to_vec();
        out_of_bounds[20] = 1;
        assert!(validate_raster(&out_of_bounds, None).is_err());
        let mut animated = GIF[..19].to_vec();
        for _ in 0..=MAX_FRAMES {
            animated.extend_from_slice(&GIF[19..GIF.len() - 1]);
        }
        animated.push(0x3b);
        assert!(validate_raster(&animated, None).is_err());
    }

    #[test]
    fn animation_budget_covers_frame_count_and_combined_canvas_pixels() {
        assert!(validate_animation_pixels(80, 80, 256).is_ok());
        assert!(validate_animation_pixels(1024, 1024, 16).is_ok());
        assert!(validate_animation_pixels(1024, 1024, 17).is_err());
        assert!(validate_animation_pixels(1, 1, 257).is_err());
        let mut apng = b"\x89PNG\r\n\x1a\n".to_vec();
        apng.extend_from_slice(&8u32.to_be_bytes());
        apng.extend_from_slice(b"acTL");
        apng.extend_from_slice(&257u32.to_be_bytes());
        apng.extend_from_slice(&[0; 8]);
        assert!(validate_animation(&apng, ImageFormat::Png, 1, 1).is_err());
        let mut webp = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
        for _ in 0..257 {
            webp.extend_from_slice(b"ANMF");
            webp.extend_from_slice(&16u32.to_le_bytes());
            webp.extend_from_slice(&[0; 16]);
        }
        assert!(validate_animation(&webp, ImageFormat::WebP, 1, 1).is_err());
    }

    #[test]
    fn cache_is_lru_and_enforces_entry_and_byte_budgets() {
        let mut cache = AssetCache::default();
        for index in 0..MAX_CACHE_ENTRIES {
            cache.insert(
                index.to_string(),
                ChatAsset {
                    data_url: "x".into(),
                },
            );
        }
        assert!(cache.get("0").is_some());
        cache.insert(
            "new".into(),
            ChatAsset {
                data_url: "x".into(),
            },
        );
        assert!(cache.get("1").is_none());
        assert!(cache.get("0").is_some());
        assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
        for index in 0..40 {
            cache.insert(
                format!("large-{index}"),
                ChatAsset {
                    data_url: "x".repeat(MAX_BODY_BYTES),
                },
            );
        }
        assert!(cache.bytes <= MAX_CACHE_BYTES);
        assert!(cache.entries.len() < 32);
        let count = cache.entries.len();
        cache.insert(
            "large-39".into(),
            ChatAsset {
                data_url: "replaced".into(),
            },
        );
        assert_eq!(cache.entries.len(), count);
        assert_eq!(
            cache.bytes,
            cache
                .entries
                .iter()
                .map(|(key, asset)| key.len() + asset.data_url.len())
                .sum::<usize>()
        );
    }

    #[test]
    fn concurrency_permits_fail_promptly_and_release_on_drop() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let permits = (0..MAX_CONCURRENT)
            .map(|_| Permit::acquire(&COUNTER).unwrap())
            .collect::<Vec<_>>();
        assert!(Permit::acquire(&COUNTER).is_err());
        drop(permits);
        assert_eq!(COUNTER.load(Ordering::Acquire), 0);
        assert!(Permit::acquire(&COUNTER).is_ok());
    }

    #[test]
    fn dto_and_errors_do_not_expose_remote_urls() {
        let error = fetch_chat_asset("https://user:secret@ssl.pstatic.net/a.png").unwrap_err();
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("pstatic"));
        assert_eq!(
            serde_json::to_value(ChatAsset {
                data_url: "data:image/png;base64,eA==".into()
            })
            .unwrap()["dataUrl"],
            "data:image/png;base64,eA=="
        );
    }
}
