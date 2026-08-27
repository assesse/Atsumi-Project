use std::panic::{catch_unwind, AssertUnwindSafe};

use bin_rs::reader::BytesReader;
use image::{DynamicImage, RgbaImage};

const MAX_AVIF_DIMENSION: usize = 16_384;
const MAX_AVIF_RGBA_BYTES: usize = 256 * 1024 * 1024;

pub(crate) fn decode_avif_rgba(bytes: &[u8]) -> Result<DynamicImage, &'static str> {
    let info = catch_unwind(AssertUnwindSafe(|| {
        let mut reader = BytesReader::new(bytes);
        avif_rust::parse_info(&mut reader)
    }))
    .map_err(|_| "AVIF metadata parser panicked")?
    .map_err(|_| "AVIF metadata could not be parsed")?;
    let width = usize::try_from(info.width.ok_or("AVIF width is missing")?)
        .map_err(|_| "AVIF width is out of range")?;
    let height = usize::try_from(info.height.ok_or("AVIF height is missing")?)
        .map_err(|_| "AVIF height is out of range")?;
    drop(info);
    validate_rgba_geometry(width, height)?;

    let decoded = catch_unwind(AssertUnwindSafe(|| avif_rust::image_from_bytes(bytes)))
        .map_err(|_| "AVIF decoder panicked")?
        .map_err(|_| "AVIF decoder rejected the payload")?;
    validate_rgba_geometry(decoded.width, decoded.height)?;
    if !matches!(
        (decoded.width, decoded.height),
        (w, h) if (w == width && h == height) || (w == height && h == width)
    ) {
        return Err("AVIF decoded dimensions do not match container metadata");
    }
    let width = u32::try_from(decoded.width).map_err(|_| "AVIF width is out of range")?;
    let height = u32::try_from(decoded.height).map_err(|_| "AVIF height is out of range")?;
    let image = RgbaImage::from_raw(width, height, decoded.rgba)
        .ok_or("AVIF RGBA buffer length does not match its dimensions")?;
    Ok(DynamicImage::ImageRgba8(image))
}

fn validate_rgba_geometry(width: usize, height: usize) -> Result<(), &'static str> {
    if width == 0 || height == 0 || width > MAX_AVIF_DIMENSION || height > MAX_AVIF_DIMENSION {
        return Err("AVIF dimensions exceed the decoder safety boundary");
    }
    let rgba_bytes = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("AVIF RGBA allocation overflows")?;
    if rgba_bytes > MAX_AVIF_RGBA_BYTES {
        return Err("AVIF RGBA allocation exceeds the decoder safety boundary");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_avif_without_panicking() {
        assert!(decode_avif_rgba(b"not an avif").is_err());
    }

    #[test]
    fn geometry_checks_overflow_and_allocation_budget() {
        assert!(validate_rgba_geometry(0, 1).is_err());
        assert!(validate_rgba_geometry(16_384, 16_384).is_err());
        assert!(validate_rgba_geometry(8_192, 8_192).is_ok());
    }
}
