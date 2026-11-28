use super::*;

fn png() -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(3, 3)
        .write_to(&mut output, ImageFormat::Png)
        .unwrap();
    output.into_inner()
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut result = (data.len() as u32).to_be_bytes().to_vec();
    result.extend_from_slice(kind);
    result.extend_from_slice(data);
    let mut crc = u32::MAX;
    for value in &result[4..] {
        crc ^= u32::from(*value);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    result.extend_from_slice(&(!crc).to_be_bytes());
    result
}

#[test]
fn complete_png_iend_ends_animation_scan_without_changing_response_bytes() {
    let original = png();
    for trailer in [
        vec![0xff; 176],
        vec![0],
        b"not another PNG chunk".to_vec(),
        chunk(b"acTL", &[0xff; 8]),
    ] {
        let mut response = original.clone();
        response.extend_from_slice(&trailer);
        let before = response.clone();
        assert_eq!(validate_raster(&response, Some("image/png")).unwrap(), "image/png");
        assert_eq!(animation_frames(&response, ImageFormat::Png, 3, 3).unwrap(), 0);
        assert_eq!(prepare_raster(response, Some("image/png")).unwrap().0, before);
    }
}

#[test]
fn png_iend_requires_empty_data_complete_crc_and_complete_image() {
    let original = png();
    let iend = original.len() - 12;
    for length in iend..original.len() {
        assert!(validate_raster(&original[..length], Some("image/png")).is_err());
    }
    let mut bad_crc = original.clone();
    *bad_crc.last_mut().unwrap() ^= 1;
    assert!(validate_raster(&bad_crc, Some("image/png")).is_err());
    let mut nonempty = original[..iend].to_vec();
    nonempty.extend_from_slice(&chunk(b"IEND", &[0]));
    assert!(validate_raster(&nonempty, Some("image/png")).is_err());

    // A valid later IEND cannot rescue a broken chunk inside the datastream.
    let mut malformed = original[..33].to_vec();
    malformed.extend_from_slice(&u32::MAX.to_be_bytes());
    malformed.extend_from_slice(b"IDAT");
    malformed.extend_from_slice(&original[iend..]);
    assert!(validate_raster(&malformed, Some("image/png")).is_err());
    assert!(validate_raster(&original, Some("text/html")).is_err());
    let mut oversized = original;
    oversized.resize(MAX_BODY_BYTES + 1, 0);
    assert!(validate_raster(&oversized, Some("image/png")).is_err());
}

#[test]
fn png_animation_counts_and_budgets_are_checked_before_iend() {
    // This directly exercises the animation ledger; image decoding remains a
    // separate mandatory validate_raster step, not a synthetic APNG claim.
    let ledger = |declared: u32, width: u32, height: u32| {
        let mut value = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut actl = declared.to_be_bytes().to_vec();
        actl.extend_from_slice(&0u32.to_be_bytes());
        value.extend_from_slice(&chunk(b"acTL", &actl));
        let mut fctl = vec![0; 26];
        fctl[4..8].copy_from_slice(&width.to_be_bytes());
        fctl[8..12].copy_from_slice(&height.to_be_bytes());
        value.extend_from_slice(&chunk(b"fcTL", &fctl));
        value.extend_from_slice(&chunk(b"IEND", &[]));
        value.extend_from_slice(&chunk(b"fcTL", &fctl));
        value
    };
    assert_eq!(animation_frames(&ledger(1, 3, 3), ImageFormat::Png, 3, 3).unwrap(), 1);
    assert!(animation_frames(&ledger(2, 3, 3), ImageFormat::Png, 3, 3).is_err());
    assert!(animation_frames(&ledger(257, 3, 3), ImageFormat::Png, 3, 3).is_err());
    assert!(animation_frames(&ledger(1, 4, 3), ImageFormat::Png, 3, 3).is_err());
    assert!(animation_frames(&ledger(17, 1024, 1024), ImageFormat::Png, 1024, 1024).is_err());
}
