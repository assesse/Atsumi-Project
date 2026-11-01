//! Exact selected public X thumbnail -> bounded, metadata-free search pixels.
//! No browser profile, session cookies, redirect following or frame fallback.
use reqwest::Url;
use std::{
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_INPUT_SIDE: u32 = 8192;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_OUTPUT_SIDE: u32 = 2048;
const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const ERROR: &str =
    "썸네일을 불러오지 못했습니다. 현재 장면으로 자동 전환하지 않았습니다. 다시 선택해 주세요.";

pub(super) fn selected_url(value: &str) -> Option<Url> {
    if value.is_empty()
        || value.len() > 2048
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
        || value.contains(['\\', '%'])
    {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if url.as_str() != value
        || url.scheme() != "https"
        || url.host_str() != Some("pbs.twimg.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let parts: Vec<_> = url.path().strip_prefix('/')?.split('/').collect();
    let id = |value: &str| {
        !value.is_empty()
            && value.len() <= 25
            && !value.starts_with('0')
            && value.bytes().all(|b| b.is_ascii_digit())
    };
    let file = match parts.as_slice() {
        ["ext_tw_video_thumb", value, "pu", "img", file] if id(value) => *file,
        ["amplify_video_thumb", value, "img", file] if id(value) => *file,
        ["tweet_video_thumb" | "media", file] => *file,
        _ => return None,
    };
    let (stem, extension) = file
        .rsplit_once('.')
        .map_or((file, None), |(stem, ext)| (stem, Some(ext)));
    if stem.is_empty()
        || stem.len() > 256
        || !stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        || extension.is_some_and(|ext| !matches!(ext, "jpg" | "jpeg" | "png" | "webp"))
    {
        return None;
    }
    let mut format = false;
    let mut name = false;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "format" if !format && matches!(value.as_ref(), "jpg" | "jpeg" | "png" | "webp") => {
                if extension.is_some_and(|ext| ext != value) {
                    return None;
                }
                format = true;
            }
            "name"
                if !name
                    && !value.is_empty()
                    && value.len() <= 24
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) =>
            {
                name = true
            }
            _ => return None,
        }
    }
    (extension.is_some() || format).then_some(url)
}

fn read_image(
    mut reader: impl Read,
    length: Option<u64>,
    content_type: &str,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, &'static str> {
    let mime = content_type.split(';').next().unwrap_or("").trim();
    if !matches!(mime, "image/jpeg" | "image/png" | "image/webp")
        || length.is_some_and(|n| n == 0 || n > MAX_BYTES as u64)
    {
        return Err(ERROR);
    }
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 16384];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(ERROR);
        }
        let count = reader.read(&mut buffer).map_err(|_| ERROR)?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > MAX_BYTES {
            return Err(ERROR);
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    if bytes.is_empty()
        || length.is_some_and(|n| n != bytes.len() as u64)
        || cancel.load(Ordering::Acquire)
    {
        return Err(ERROR);
    }
    Ok(bytes)
}

pub(super) fn clean_pixels(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(ERROR);
    }
    let format = image::guess_format(bytes).map_err(|_| ERROR)?;
    if !matches!(
        format,
        image::ImageFormat::Jpeg
            | image::ImageFormat::Png
            | image::ImageFormat::WebP
            | image::ImageFormat::Gif
    ) {
        return Err(ERROR);
    }
    let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| ERROR)?;
    if width == 0
        || height == 0
        || width > MAX_INPUT_SIDE
        || height > MAX_INPUT_SIDE
        || width as u64 * height as u64 > MAX_PIXELS
    {
        return Err(ERROR);
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_INPUT_SIDE);
    limits.max_image_height = Some(MAX_INPUT_SIDE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| ERROR)?;
    // This is an output ceiling, not a target resolution. The image crate's
    // thumbnail operation can upscale; preserve small source pixels instead.
    let decoded = if decoded.width() > MAX_OUTPUT_SIDE || decoded.height() > MAX_OUTPUT_SIDE {
        decoded.thumbnail(MAX_OUTPUT_SIDE, MAX_OUTPUT_SIDE)
    } else {
        decoded
    };
    let image = decoded.to_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
        .encode_image(&image)
        .map_err(|_| ERROR)?;
    if jpeg.len() > MAX_OUTPUT_BYTES {
        return Err(ERROR);
    }
    Ok(jpeg)
}

pub(super) fn load(selected: &Url, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    if cancel.load(Ordering::Acquire) || selected_url(selected.as_str()).as_ref() != Some(selected)
    {
        return Err(ERROR.into());
    }
    // A fresh client has no cookie jar/default authorization. Do not reuse the
    // authenticated X transport or follow a CDN redirect to another resource.
    let client = reqwest::blocking::Client::builder()
        .user_agent("Atsumi/2.0")
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .build()
        .map_err(|_| ERROR)?;
    let response = client
        .get(selected.clone())
        .header(reqwest::header::ACCEPT, "image/jpeg,image/png,image/webp")
        .send()
        .map_err(|_| ERROR)?;
    if response.status() != reqwest::StatusCode::OK || response.url() != selected {
        return Err(ERROR.into());
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let length = response.content_length();
    let bytes = read_image(response, length, &content_type, cancel)?;
    let jpeg = clean_pixels(&bytes)?;
    if cancel.load(Ordering::Acquire) {
        return Err(ERROR.into());
    }
    Ok(jpeg)
}

/// The exact selected still-image URL, admitted by the parent media selector.
/// Only the Lens branch uses this after the new explicit two-provider consent.
pub(super) fn load_source_image(selected: &Url, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    if cancel.load(Ordering::Acquire)
        || super::image_url(selected.as_str()).as_ref() != Some(selected)
    {
        return Err(ERROR.into());
    }
    let client = reqwest::blocking::Client::builder()
        .user_agent("Atsumi/2.0")
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .build()
        .map_err(|_| ERROR)?;
    let mut response = client
        .get(selected.clone())
        .header(
            reqwest::header::ACCEPT,
            "image/jpeg,image/png,image/webp,image/gif",
        )
        .send()
        .map_err(|_| ERROR)?;
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if response.status() != reqwest::StatusCode::OK
        || response.url() != selected
        || !matches!(
            mime,
            "image/jpeg" | "image/png" | "image/webp" | "image/gif"
        )
        || response
            .content_length()
            .is_some_and(|n| n == 0 || n > MAX_BYTES as u64)
    {
        return Err(ERROR.into());
    }
    let length = response.content_length();
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 16384];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(ERROR.into());
        }
        let count = response.read(&mut buffer).map_err(|_| ERROR)?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > MAX_BYTES {
            return Err(ERROR.into());
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    if bytes.is_empty()
        || length.is_some_and(|n| n != bytes.len() as u64)
        || cancel.load(Ordering::Acquire)
    {
        return Err(ERROR.into());
    }
    clean_pixels(&bytes).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_public_poster_urls_only_and_query_is_not_upgraded() {
        for value in [
            "https://pbs.twimg.com/ext_tw_video_thumb/123/pu/img/Poster.jpg?name=small",
            "https://pbs.twimg.com/amplify_video_thumb/123/img/Ab_-?format=webp&name=large",
            "https://pbs.twimg.com/tweet_video_thumb/a.png",
            "https://pbs.twimg.com/media/Observed.jpg",
        ] {
            assert_eq!(selected_url(value).unwrap().as_str(), value);
        }
        for value in [
            "https://evil.test/a.jpg",
            "http://pbs.twimg.com/tweet_video_thumb/a.jpg",
            "https://pbs.twimg.com/profile_images/123/a.jpg",
            "https://pbs.twimg.com/tweet_video_thumb/a.svg",
            "https://pbs.twimg.com/tweet_video_thumb/a.gif",
            "https://pbs.twimg.com/tweet_video_thumb/a.jpg?token=secret",
            "https://pbs.twimg.com/tweet_video_thumb/a.jpg?name=small&name=large",
            "https://pbs.twimg.com/tweet_video_thumb/a.jpg?format=png",
            "https://pbs.twimg.com/tweet_video_thumb/a%2ejpg",
            "https://pbs.twimg.com/tweet_video_thumb/a.jpg#fragment",
            "https://user@pbs.twimg.com/tweet_video_thumb/a.jpg",
            "https://pbs.twimg.com:443/tweet_video_thumb/a.jpg",
            "https://pbs.twimg.com:444/tweet_video_thumb/a.jpg",
            "https://pbs.twimg.com//tweet_video_thumb/a.jpg",
            "https://pbs.twimg.com/amplify_video_thumb/0/img/a.jpg",
        ] {
            assert!(selected_url(value).is_none(), "{value}");
        }
    }
    #[test]
    fn response_bytes_are_bounded_typed_complete_and_cancellable() {
        let cancel = AtomicBool::new(false);
        assert_eq!(
            read_image(&b"abc"[..], Some(3), "image/jpeg", &cancel).unwrap(),
            b"abc"
        );
        for (length, mime) in [
            (Some(0), "image/jpeg"),
            (Some(4), "image/jpeg"),
            (Some(MAX_BYTES as u64 + 1), "image/png"),
            (None, "text/html"),
        ] {
            assert!(read_image(&b"abc"[..], length, mime, &cancel).is_err());
        }
        assert!(read_image(
            std::io::repeat(0).take(MAX_BYTES as u64 + 1),
            None,
            "image/png",
            &cancel
        )
        .is_err());
        cancel.store(true, Ordering::Release);
        assert!(read_image(&b"abc"[..], Some(3), "image/jpeg", &cancel).is_err());
        assert!(load(
            &selected_url("https://pbs.twimg.com/tweet_video_thumb/a.jpg").unwrap(),
            &cancel
        )
        .is_err());
    }
    #[test]
    fn export_is_bounded_jpeg_without_original_trailing_metadata() {
        let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2400,
            1200,
            image::Rgb([215, 20, 45]),
        ));
        let mut png = std::io::Cursor::new(Vec::new());
        source.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let marker = b"DO_NOT_SEND_ORIGINAL_THUMBNAIL_METADATA";
        let mut bytes = png.into_inner();
        bytes.extend_from_slice(marker);
        let clean = clean_pixels(&bytes).unwrap();
        assert!(clean.starts_with(&[0xff, 0xd8, 0xff]));
        assert!(!clean.windows(marker.len()).any(|chunk| chunk == marker));
        let decoded = image::load_from_memory(&clean).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2048, 1024));
        assert!(clean_pixels(b"<html>not a thumbnail</html>").is_err());
        assert!(clean_pixels(&vec![0; MAX_BYTES + 1]).is_err());
    }
    #[test]
    fn normalization_preserves_small_source_dimensions_instead_of_upscaling() {
        for (width, height) in [(48, 32), (32, 48), (1, 1), (2048, 16), (16, 2048)] {
            let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                width,
                height,
                image::Rgb([25, 120, 210]),
            ));
            let mut png = std::io::Cursor::new(Vec::new());
            source.write_to(&mut png, image::ImageFormat::Png).unwrap();
            let clean = clean_pixels(&png.into_inner()).unwrap();
            let decoded = image::load_from_memory(&clean).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (width, height));
            assert!(clean.starts_with(&[0xff, 0xd8, 0xff]));
        }
    }
}
