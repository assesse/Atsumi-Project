//! Canonicalize one observed YouTube AAC-LC init padding variant only.
//! Playback buffers and compressed media samples never enter this adapter.
use super::EncodedTrack;

const MAX_INIT: usize = 65_536;
const PATH: [&[u8; 4]; 8] = [
    b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsd", b"mp4a", b"esds",
];

pub(super) fn normalize(provider: &str, inputs: &mut [EncodedTrack]) {
    if provider != "youtube" {
        return;
    }
    for input in inputs {
        let mime = input.mime_type.to_ascii_lowercase().replace([' ', '"'], "");
        if mime != "audio/mp4;codecs=mp4a.40.2" {
            continue;
        }
        if let Some(init) = rewrite_init(&input.init, normalize_esds) {
            input.init = init;
        }
    }
}

// Any unknown/malformed variant is returned untouched to the existing strict
// mux validator. Only a complete, uniquely located matching ASC is rewritten.
fn rewrite_init<F>(input: &[u8], mut transform: F) -> Option<Vec<u8>>
where
    F: FnMut(&[u8]) -> Option<Vec<u8>>,
{
    if input.is_empty() || input.len() > MAX_INIT {
        return None;
    }
    let mut nodes = 0;
    let (result, changes) = rewrite_level(input, 0, &mut nodes, &mut transform)?;
    (changes == 1 && result.len() <= MAX_INIT).then_some(result)
}

fn rewrite_level<F>(
    input: &[u8],
    depth: usize,
    nodes: &mut usize,
    transform: &mut F,
) -> Option<(Vec<u8>, usize)>
where
    F: FnMut(&[u8]) -> Option<Vec<u8>>,
{
    let mut result = Vec::with_capacity(input.len());
    let (mut at, mut changes) = (0usize, 0usize);
    while at < input.len() {
        *nodes += 1;
        if *nodes > 128 {
            return None;
        }
        let header = input.get(at..at.checked_add(8)?)?;
        let size = u32::from_be_bytes(header[..4].try_into().ok()?) as usize;
        if !(8..=MAX_INIT).contains(&size) {
            return None;
        }
        let end = at.checked_add(size)?;
        let full = input.get(at..end)?;
        let kind: &[u8; 4] = header[4..8].try_into().ok()?;
        if kind != PATH[depth] {
            result.extend_from_slice(full);
        } else {
            let body = &full[8..];
            if depth == PATH.len() - 1 {
                if let Some(rewritten) = transform(body) {
                    result.extend(boxed(kind, &rewritten)?);
                    changes += 1;
                } else {
                    result.extend_from_slice(full);
                }
            } else {
                let prefix = match kind {
                    b"stsd" => {
                        if body.get(..8)? != [0, 0, 0, 0, 0, 0, 0, 1] {
                            return None;
                        }
                        8
                    }
                    b"mp4a" => 28,
                    _ => 0,
                };
                let (children, changed) =
                    rewrite_level(body.get(prefix..)?, depth + 1, nodes, transform)?;
                if changed == 0 {
                    result.extend_from_slice(full);
                } else {
                    let mut rewritten = body.get(..prefix)?.to_vec();
                    rewritten.extend(children);
                    result.extend(boxed(kind, &rewritten)?);
                    changes += changed;
                }
            }
        }
        if changes > 1 || result.len() > MAX_INIT {
            return None;
        }
        at = end;
    }
    Some((result, changes))
}

fn boxed(kind: &[u8; 4], body: &[u8]) -> Option<Vec<u8>> {
    let size = body.len().checked_add(8)?;
    if size > MAX_INIT {
        return None;
    }
    let mut output = Vec::with_capacity(size);
    output.extend_from_slice(&(size as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(body);
    Some(output)
}

fn descriptor<'a>(bytes: &'a [u8], at: &mut usize) -> Option<(u8, &'a [u8])> {
    let tag = *bytes.get(*at)?;
    *at += 1;
    let mut length = 0usize;
    for index in 0..4 {
        let value = *bytes.get(*at)?;
        *at += 1;
        length = length
            .checked_mul(128)?
            .checked_add((value & 127) as usize)?;
        if length > MAX_INIT {
            return None;
        }
        if value & 128 == 0 {
            let end = at.checked_add(length)?;
            let payload = bytes.get(*at..end)?;
            *at = end;
            return Some((tag, payload));
        }
        if index == 3 {
            return None;
        }
    }
    None
}

fn encode_descriptor(tag: u8, payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > MAX_INIT {
        return None;
    }
    let mut length = payload.len();
    let mut digits = vec![(length & 127) as u8];
    length >>= 7;
    while length > 0 {
        digits.push(((length & 127) as u8) | 128);
        length >>= 7;
    }
    let mut output = vec![tag];
    output.extend(digits.into_iter().rev());
    output.extend_from_slice(payload);
    Some(output)
}

fn normalize_esds(body: &[u8]) -> Option<Vec<u8>> {
    if body.get(..4)? != [0; 4] {
        return None;
    }
    let mut at = 4;
    let (tag, es) = descriptor(body, &mut at)?;
    if tag != 3 || at != body.len() || *es.get(2)? != 0 {
        return None;
    }
    let mut es_at = 3;
    let (tag, decoder) = descriptor(es, &mut es_at)?;
    if tag != 4 || decoder.get(..2)? != [0x40, 0x15] {
        return None;
    }
    // Preserve the observed standard SLConfig descriptor, if present. No URL,
    // dependent-stream configuration or unknown descriptor is normalized.
    let suffix = es.get(es_at..)?;
    if !suffix.is_empty() {
        let mut sl_at = 0;
        let (tag, sl) = descriptor(suffix, &mut sl_at)?;
        if tag != 6 || sl != [2] || sl_at != suffix.len() {
            return None;
        }
    }
    let mut decoder_at = 13;
    let (tag, asc) = descriptor(decoder, &mut decoder_at)?;
    if tag != 5 || decoder_at != decoder.len() || asc.len() != 16 {
        return None;
    }
    let frequency = ((asc[0] & 7) << 1) | (asc[1] >> 7);
    let channels = (asc[1] >> 3) & 15;
    if asc[0] >> 3 != 2
        || frequency > 12
        || !(1..=2).contains(&channels)
        || asc[1] & 7 != 0
        || asc[2..].iter().any(|value| *value != 0)
    {
        return None;
    }
    let mut normalized_decoder = decoder.get(..13)?.to_vec();
    normalized_decoder.extend(encode_descriptor(5, &asc[..2])?);
    let mut normalized_es = es.get(..3)?.to_vec();
    normalized_es.extend(encode_descriptor(4, &normalized_decoder)?);
    normalized_es.extend_from_slice(suffix);
    let mut normalized = body[..4].to_vec();
    normalized.extend(encode_descriptor(3, &normalized_es)?);
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::super::fmp4::{fixtures, EncodedMuxer};
    use super::*;

    fn esds(asc: &[u8]) -> Vec<u8> {
        let mut decoder = vec![0x40, 0x15];
        decoder.extend([0; 11]);
        decoder.extend(encode_descriptor(5, asc).unwrap());
        let mut es = vec![0, 7, 0];
        es.extend(encode_descriptor(4, &decoder).unwrap());
        es.extend([6, 1, 2]);
        let mut body = vec![0; 4];
        body.extend(encode_descriptor(3, &es).unwrap());
        body
    }
    fn padded() -> Vec<EncodedTrack> {
        let mut inputs = fixtures::inputs();
        let mut asc = vec![0x12, 0x10];
        asc.extend([0; 14]);
        inputs[1].init = rewrite_init(&inputs[1].init, |_| Some(esds(&asc))).unwrap();
        inputs
    }
    #[test]
    fn observed_padding_is_equivalent_lc_and_immutable_parser_accepts() {
        let original = padded();
        let source = original[1].init.clone();
        assert!(EncodedMuxer::new(original.clone()).is_err());
        let mut normalized = original.clone();
        normalize("youtube", &mut normalized);
        assert_eq!(original[1].init, source);
        assert_eq!(normalized[0].init, original[0].init);
        assert_eq!(normalized[1].init.len() + 14, source.len());
        let expected =
            rewrite_init(&fixtures::inputs()[1].init, |_| Some(esds(&[0x12, 0x10]))).unwrap();
        assert_eq!(normalized[1].init, expected);
        let mut mux = EncodedMuxer::new(normalized).unwrap();
        for index in 0..2 {
            mux.push(index, &fixtures::fragment(index, 0, 4, true))
                .unwrap();
        }
        assert!(!mux.finish().unwrap().is_empty());
    }
    #[test]
    fn provider_and_mime_boundaries_leave_other_inputs_unchanged() {
        let source = padded();
        for provider in ["soop", "chzzk", "YouTube", ""] {
            let mut inputs = source.clone();
            normalize(provider, &mut inputs);
            assert_eq!(inputs[1].init, source[1].init);
        }
        let mut inputs = source.clone();
        inputs[1].mime_type = "audio/mp4;codecs=mp4a.40.5".into();
        normalize("youtube", &mut inputs);
        assert_eq!(inputs[1].init, source[1].init);
    }
    #[test]
    fn unknown_extensions_codecs_and_ga_flags_are_never_trimmed() {
        for variant in 0..9 {
            let mut asc = vec![0x12, 0x10];
            asc.extend([0; 14]);
            match variant {
                0 => asc[15] = 1,
                1 => asc[0] = 5 << 3,
                2 => asc[0] = 29 << 3,
                3 => asc[1] |= 1,
                4 => asc[1] = 0,
                5 => {
                    asc[0] = (2 << 3) | 7;
                    asc[1] |= 128;
                }
                6 => {
                    asc.pop();
                }
                7 => asc.push(0),
                _ => asc.truncate(2),
            }
            assert!(normalize_esds(&esds(&asc)).is_none(), "variant {variant}");
        }
    }
    #[test]
    fn malformed_descriptor_nesting_and_boxes_stay_fail_closed() {
        let mut asc = vec![0x12, 0x10];
        asc.extend([0; 14]);
        let source = esds(&asc);
        for variant in 0..7 {
            let mut value = source.clone();
            match variant {
                0 => value[0] = 1,
                1 => value[4] = 4,
                2 => value[8] = 128,
                3 => value[9] = 5,
                4 => value[11] = 0x41,
                5 => value[24] = 6,
                _ => value[5] = 127,
            }
            assert!(normalize_esds(&value).is_none(), "variant {variant}");
        }
        let inputs = padded();
        let mut bad = inputs[1].init.clone();
        bad[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(rewrite_init(&bad, normalize_esds).is_none());
        assert!(rewrite_init(&inputs[1].init[..13], normalize_esds).is_none());
        assert!(rewrite_init(&vec![0; MAX_INIT + 1], normalize_esds).is_none());
    }
}
