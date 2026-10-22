//! Adapt one observed YouTube fragment header, never compressed samples.
//! Construct only after the immutable init validator accepted stsd/trex index 1.
use super::EncodedTrack;
use std::collections::BTreeMap;

const MAX_MOOF: usize = 65_536;
const MAX_BOX: usize = 16 * 1024 * 1024;
const EMIT_CHUNK: usize = 128 * 1024;
const INVALID: &str = "지원하지 않는 fragment 크기입니다.";

#[derive(Default)]
struct Track {
    video_id: Option<u32>,
    header: Vec<u8>,
    moof: Vec<u8>,
    remaining: usize,
}

pub(super) struct Adapter {
    tracks: BTreeMap<u32, Track>,
}
impl Adapter {
    pub(super) fn for_validated_inputs(provider: &str, tracks: &[EncodedTrack]) -> Option<Self> {
        (provider == "youtube").then(|| Self {
            tracks: tracks
                .iter()
                .map(|input| {
                    (
                        input.track_index,
                        Track {
                            video_id: validated_video_id(&input.init),
                            ..Track::default()
                        },
                    )
                })
                .collect(),
        })
    }

    pub(super) fn incomplete(&self) -> bool {
        self.tracks
            .values()
            .any(|track| !track.header.is_empty() || !track.moof.is_empty() || track.remaining != 0)
    }

    /// Box headers/moof may cross any MSE/IPC append boundary. Media payload is
    /// forwarded directly, without another media-sized allocation. The caller
    /// poisons the archive on any error; this adapter is not an alternate parser.
    pub(super) fn push(
        &mut self,
        track: u32,
        mut input: &[u8],
        mut emit: impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        let state = self.tracks.get_mut(&track).ok_or(INVALID)?;
        while !input.is_empty() {
            if state.remaining == 0 {
                let take = (8 - state.header.len()).min(input.len());
                state.header.extend_from_slice(&input[..take]);
                input = &input[take..];
                if state.header.len() < 8 {
                    continue;
                }
                let size = read32(&state.header, 0).ok_or(INVALID)? as usize;
                if !(8..=MAX_BOX).contains(&size) {
                    return Err(INVALID.into());
                }
                state.remaining = size - 8;
                if state.header[4..8] == *b"moof" && size <= MAX_MOOF {
                    state.moof = std::mem::take(&mut state.header);
                    if state.remaining == 0 {
                        emit(&state.moof)?;
                        state.moof.clear();
                    }
                } else {
                    emit(&state.header)?;
                    state.header.clear();
                }
            } else {
                let take = state.remaining.min(input.len()).min(EMIT_CHUNK);
                if state.moof.is_empty() {
                    emit(&input[..take])?;
                } else {
                    state.moof.extend_from_slice(&input[..take]);
                }
                state.remaining -= take;
                input = &input[take..];
                if state.remaining == 0 && !state.moof.is_empty() {
                    normalize_moof(&mut state.moof, state.video_id);
                    emit(&state.moof)?;
                    state.moof.clear();
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Atom {
    at: usize,
    size: usize,
    kind: [u8; 4],
}
fn read32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
}
fn children(bytes: &[u8], at: usize, end: usize, nodes: &mut usize) -> Option<Vec<Atom>> {
    let mut result = Vec::new();
    let mut cursor = at;
    while cursor < end {
        *nodes += 1;
        if *nodes > 64 || end > bytes.len() {
            return None;
        }
        let size = read32(bytes, cursor)? as usize;
        let next = cursor.checked_add(size)?;
        if size < 8 || next > end {
            return None;
        }
        result.push(Atom {
            at: cursor,
            size,
            kind: bytes.get(cursor + 4..cursor + 8)?.try_into().ok()?,
        });
        cursor = next;
    }
    Some(result)
}
fn one(atoms: &[Atom], kind: &[u8; 4]) -> Option<Atom> {
    let mut found = atoms.iter().filter(|atom| &atom.kind == kind);
    let value = *found.next()?;
    found.next().is_none().then_some(value)
}

// This is only a binding extracted AFTER strict init admission, not a new
// codec validator. MIME and input position must not identify the video track.
fn validated_video_id(init: &[u8]) -> Option<u32> {
    if init.len() > 65_536 {
        return None;
    }
    let mut nodes = 0;
    let top = children(init, 0, init.len(), &mut nodes)?;
    let moov = one(&top, b"moov")?;
    let movie = children(init, moov.at + 8, moov.at + moov.size, &mut nodes)?;
    let trak = one(&movie, b"trak")?;
    let track = children(init, trak.at + 8, trak.at + trak.size, &mut nodes)?;
    let tkhd = one(&track, b"tkhd")?;
    let id_offset = match init.get(tkhd.at + 8)? {
        0 => 20,
        1 => 28,
        _ => return None,
    };
    if tkhd.size < id_offset + 4 {
        return None;
    }
    let id = read32(init, tkhd.at + id_offset)?;
    let mdia = one(&track, b"mdia")?;
    let media = children(init, mdia.at + 8, mdia.at + mdia.size, &mut nodes)?;
    let hdlr = one(&media, b"hdlr")?;
    (hdlr.size >= 20 && init.get(hdlr.at + 16..hdlr.at + 20)? == b"vide" && id != 0).then_some(id)
}

// Return None without changing any bytes for unknown/ambiguous shapes. The
// original strict validator remains responsible for codec/sample validation.
fn layout(bytes: &[u8]) -> Option<(Atom, Atom, Atom)> {
    if bytes.len() > MAX_MOOF
        || read32(bytes, 0)? as usize != bytes.len()
        || bytes.get(4..8)? != b"moof"
    {
        return None;
    }
    let mut nodes = 0;
    let outer = children(bytes, 8, bytes.len(), &mut nodes)?;
    if outer
        .iter()
        .any(|atom| !matches!(&atom.kind, b"mfhd" | b"traf"))
    {
        return None;
    }
    let mfhd = one(&outer, b"mfhd")?;
    if mfhd.size != 16 || read32(bytes, mfhd.at + 8)? != 0 {
        return None;
    }
    let traf = one(&outer, b"traf")?;
    let inner = children(bytes, traf.at + 8, traf.at + traf.size, &mut nodes)?;
    if inner
        .iter()
        .any(|atom| !matches!(&atom.kind, b"tfhd" | b"tfdt" | b"trun" | b"sdtp"))
        || inner.iter().filter(|atom| atom.kind == *b"sdtp").count() > 1
    {
        return None;
    }
    one(&inner, b"tfdt")?;
    let tfhd = one(&inner, b"tfhd")?;
    let flags = read32(bytes, tfhd.at + 8)?;
    if flags & !0x02003a != 0
        || flags & 0x020002 != 0x020002
        || tfhd.size != 20 + (flags & 0x38).count_ones() as usize * 4
        || read32(bytes, tfhd.at + 16)? != 1
    {
        return None;
    }
    let trun = one(&inner, b"trun")?;
    let full = read32(bytes, trun.at + 8)?;
    let flags = full & 0x00ff_ffff;
    let count = read32(bytes, trun.at + 12)? as usize;
    let width = (flags & 0xf00).count_ones() as usize * 4;
    let expected = 20usize
        .checked_add(if flags & 4 != 0 { 4 } else { 0 })?
        .checked_add(count.checked_mul(width)?)?;
    if full >> 24 > 1 || flags & !0xf05 != 0 || flags & 1 == 0
        || flags & 0x404 == 0x404 || !(1..=120_000).contains(&count)
        || trun.size != expected
        // Only the observed single-traf, adjacent mdat layout is normalized.
        || read32(bytes, trun.at + 16)? as usize != bytes.len() + 8
    {
        return None;
    }
    Some((traf, tfhd, trun))
}
fn normalize_moof(bytes: &mut Vec<u8>, video_id: Option<u32>) -> bool {
    let Some((traf, tfhd, trun)) = layout(bytes) else {
        return false;
    };
    let flags = read32(bytes, tfhd.at + 8).unwrap();
    let offset = read32(bytes, trun.at + 16).unwrap();
    // YouTube's observed first sync sample has dependency=unknown (flags0).
    // Normalize only this exact video shape. The unchanged mux then requires
    // an actual AVC IDR; a non-IDR is rejected, never published as a keyframe.
    if flags == 0x02000a
        && video_id.is_some_and(|id| read32(bytes, tfhd.at + 12) == Some(id))
        && read32(bytes, trun.at + 8) == Some(0x601)
        && read32(bytes, trun.at + 24) == Some(0)
    {
        put32(bytes, trun.at + 24, 0x02000000);
    }
    put32(bytes, tfhd.at + 8, flags & !2);
    put32(bytes, trun.at + 16, offset - 4);
    for (at, size) in [(0, bytes.len()), (traf.at, traf.size), (tfhd.at, tfhd.size)] {
        put32(bytes, at, (size - 4) as u32);
    }
    bytes.drain(tfhd.at + 16..tfhd.at + 20);
    true
}

#[cfg(test)]
pub(super) fn explicit_description(fragment: &[u8]) -> Vec<u8> {
    let mut bytes = fragment.to_vec();
    let moof_size = read32(&bytes, 0).unwrap() as usize;
    let mut nodes = 0;
    let outer = children(&bytes, 8, moof_size, &mut nodes).unwrap();
    let traf = one(&outer, b"traf").unwrap();
    let inner = children(&bytes, traf.at + 8, traf.at + traf.size, &mut nodes).unwrap();
    let tfhd = one(&inner, b"tfhd").unwrap();
    let trun = one(&inner, b"trun").unwrap();
    let flags = read32(&bytes, tfhd.at + 8).unwrap();
    let offset = read32(&bytes, trun.at + 16).unwrap();
    put32(&mut bytes, tfhd.at + 8, flags | 2);
    put32(&mut bytes, trun.at + 16, offset + 4);
    for (at, size) in [(0, moof_size), (traf.at, traf.size), (tfhd.at, tfhd.size)] {
        put32(&mut bytes, at, (size + 4) as u32);
    }
    bytes.splice(tfhd.at + 16..tfhd.at + 16, 1u32.to_be_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::super::fmp4::{fixtures, EncodedMuxer};
    use super::*;
    fn adapt(track: u32, chunks: &[&[u8]]) -> Vec<u8> {
        let mut adapter = Adapter::for_validated_inputs("youtube", &fixtures::inputs()).unwrap();
        let mut output = Vec::new();
        for bytes in chunks {
            adapter
                .push(track, bytes, |bytes| {
                    output.extend_from_slice(bytes);
                    Ok(())
                })
                .unwrap();
        }
        assert!(!adapter.incomplete());
        output
    }
    fn first_sample_fixture(track: u32, idr: bool) -> Vec<u8> {
        fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
            [
                (body.len() as u32 + 8).to_be_bytes().to_vec(),
                kind.to_vec(),
                body.to_vec(),
            ]
            .concat()
        }
        let original = fixtures::fragment(track, 0, 6, idr);
        let source_moof = read32(&original, 0).unwrap() as usize;
        let duration = if track == 0 {
            fixtures::VIDEO_SCALE / 2
        } else {
            fixtures::AUDIO_SCALE / 2
        };
        let tfhd = boxed(
            b"tfhd",
            &[
                0x02000au32.to_be_bytes(),
                7u32.to_be_bytes(),
                1u32.to_be_bytes(),
                duration.to_be_bytes(),
            ]
            .concat(),
        );
        let tfdt = boxed(
            b"tfdt",
            &[
                0x01000000u32.to_be_bytes().as_slice(),
                0u64.to_be_bytes().as_slice(),
            ]
            .concat(),
        );
        let mut run = [
            0x601u32.to_be_bytes(),
            6u32.to_be_bytes(),
            0u32.to_be_bytes(),
        ]
        .concat();
        for index in 0..6 {
            run.extend((if track == 0 { 6u32 } else { 2u32 }).to_be_bytes());
            run.extend((if index == 0 { 0u32 } else { 0x10000u32 }).to_be_bytes());
        }
        let moof_size = 8 + 16 + 8 + tfhd.len() + tfdt.len() + 8 + run.len();
        put32(&mut run, 8, (moof_size + 8) as u32);
        let moof = boxed(
            b"moof",
            &[
                boxed(b"mfhd", &[0; 8]),
                boxed(b"traf", &[tfhd, tfdt, boxed(b"trun", &run)].concat()),
            ]
            .concat(),
        );
        [moof, original[source_moof..].to_vec()].concat()
    }
    fn description_only(source: &[u8]) -> Vec<u8> {
        let size = read32(source, 0).unwrap() as usize;
        let mut moof = source[..size].to_vec();
        assert!(normalize_moof(&mut moof, None));
        moof.extend_from_slice(&source[size..]);
        moof
    }
    #[test]
    fn observed_video_sync_unknown_first_flag_requires_real_idr_and_preserves_payload() {
        let source = first_sample_fixture(0, true);
        let size = read32(&source, 0).unwrap() as usize;
        let (_, _, trun) = layout(&source[..size]).unwrap();
        let output = adapt(0, &[&source]);
        let mut expected = description_only(&source);
        // Removing the earlier tfhd field shifts this field left four bytes.
        put32(&mut expected, trun.at + 20, 0x02000000);
        assert_eq!(output, expected);
        assert_eq!(&output[size - 4..], &source[size..]);
        for split in 0..=source.len() {
            assert_eq!(adapt(0, &[&source[..split], &source[split..]]), output);
        }
        let mut mux = EncodedMuxer::new(fixtures::inputs()).unwrap();
        mux.push(0, &output).unwrap();
        mux.push(1, &fixtures::fragment(1, 0, 6, true)).unwrap();
        assert!(!mux.finish().unwrap().is_empty());
        let non_idr = adapt(0, &[&first_sample_fixture(0, false)]);
        let mut mux = EncodedMuxer::new(fixtures::inputs()).unwrap();
        let error = mux.push(0, &non_idr).err().unwrap();
        assert_eq!(
            error.message,
            "키프레임 표시와 AVC IDR이 일치하지 않습니다."
        );
    }
    #[test]
    fn only_exact_video_identity_shape_and_zero_first_flag_can_change() {
        let source = first_sample_fixture(0, true);
        let size = read32(&source, 0).unwrap() as usize;
        let (_, tfhd, trun) = layout(&source[..size]).unwrap();
        for flags in [1u32, 0x10000, 0x01000000, 0x02000000, 0x03000000] {
            let mut input = source.clone();
            put32(&mut input, trun.at + 24, flags);
            assert_eq!(adapt(0, &[&input]), description_only(&input));
        }
        for variant in 0..2 {
            let mut input = source.clone();
            if variant == 0 {
                put32(&mut input, tfhd.at + 12, 8);
            } else {
                put32(&mut input, trun.at + 8, 0x01000601);
            }
            assert_eq!(adapt(0, &[&input]), description_only(&input));
        }
        let audio = first_sample_fixture(1, true);
        assert_eq!(adapt(1, &[&audio]), description_only(&audio));
        let mut inputs = fixtures::inputs();
        inputs[0].track_index = 1;
        inputs[1].track_index = 0;
        inputs[1].mime_type = "video/mp4;codecs=avc1.42001e".into();
        let adapter = Adapter::for_validated_inputs("youtube", &inputs).unwrap();
        assert_eq!(adapter.tracks[&1].video_id, Some(7));
        assert_eq!(adapter.tracks[&0].video_id, None);
        assert_eq!(validated_video_id(&[0; 12]), None);
    }
    #[test]
    fn every_split_preserves_mdat_and_matches_accepted_implicit_header() {
        for track in 0..2 {
            let implicit = fixtures::fragment(track, 0, 6, true);
            let explicit = explicit_description(&implicit);
            let unchanged_source = explicit.clone();
            for split in 0..=explicit.len() {
                assert_eq!(
                    adapt(track, &[&explicit[..split], &explicit[split..]]),
                    implicit
                );
            }
            let chunks: Vec<_> = explicit.chunks(1).collect();
            assert_eq!(adapt(track, &chunks), implicit);
            assert_eq!(explicit, unchanged_source);
            assert_eq!(adapt(track, &[&implicit]), implicit);
        }
    }
    #[test]
    fn two_tracks_interleave_and_multiple_fragments_pass_original_strict_mux() {
        let mut adapter = Adapter::for_validated_inputs("youtube", &fixtures::inputs()).unwrap();
        let mut mux = EncodedMuxer::new(fixtures::inputs()).unwrap();
        let mut outputs = [Vec::new(), Vec::new()];
        let expected: Vec<Vec<u8>> = (0..2)
            .map(|track| {
                let scale = if track == 0 {
                    fixtures::VIDEO_SCALE
                } else {
                    fixtures::AUDIO_SCALE
                };
                [
                    fixtures::fragment(track, 0, 4, true),
                    fixtures::fragment(track, 2 * scale as u64, 4, true),
                ]
                .concat()
            })
            .collect();
        let encoded: Vec<Vec<u8>> = (0..2)
            .map(|track| {
                let scale = if track == 0 {
                    fixtures::VIDEO_SCALE
                } else {
                    fixtures::AUDIO_SCALE
                };
                [
                    explicit_description(&fixtures::fragment(track, 0, 4, true)),
                    explicit_description(&fixtures::fragment(track, 2 * scale as u64, 4, true)),
                ]
                .concat()
            })
            .collect();
        for at in 0..encoded.iter().map(Vec::len).max().unwrap() {
            for track in 0..2usize {
                if let Some(byte) = encoded[track].get(at) {
                    adapter
                        .push(track as u32, &[*byte], |bytes| {
                            outputs[track].extend_from_slice(bytes);
                            mux.push(track as u32, bytes).map_err(|e| e.message)?;
                            Ok(())
                        })
                        .unwrap();
                }
            }
        }
        assert!(!adapter.incomplete());
        assert_eq!(outputs.to_vec(), expected);
        assert!(!mux.finish().unwrap().is_empty());
    }
    #[test]
    fn actual_shape_02000a_0601_reduces_1296_1304_by_exactly_four() {
        fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
            [
                (body.len() as u32 + 8).to_be_bytes().to_vec(),
                kind.to_vec(),
                body.to_vec(),
            ]
            .concat()
        }
        let tfhd = boxed(
            b"tfhd",
            &[
                0x02000au32.to_be_bytes(),
                7u32.to_be_bytes(),
                1u32.to_be_bytes(),
                960u32.to_be_bytes(),
            ]
            .concat(),
        );
        let tfdt = boxed(
            b"tfdt",
            &[
                0x01000000u32.to_be_bytes().as_slice(),
                0u64.to_be_bytes().as_slice(),
            ]
            .concat(),
        );
        let mut run = [
            0x601u32.to_be_bytes(),
            150u32.to_be_bytes(),
            1304u32.to_be_bytes(),
        ]
        .concat();
        for _ in 0..150 {
            run.extend([2u32.to_be_bytes(), 0x02000000u32.to_be_bytes()].concat());
        }
        let moof = boxed(
            b"moof",
            &[
                boxed(b"mfhd", &[0; 8]),
                boxed(b"traf", &[tfhd, tfdt, boxed(b"trun", &run)].concat()),
            ]
            .concat(),
        );
        assert_eq!(moof.len(), 1296);
        let mdat = boxed(b"mdat", &[0x21, 0].repeat(150));
        let source = [moof, mdat.clone()].concat();
        let result = adapt(1, &[&source]);
        assert_eq!(read32(&result, 0), Some(1292));
        assert_eq!(&result[1292..], mdat);
        let mut nodes = 0;
        let outer = children(&result, 8, 1292, &mut nodes).unwrap();
        let traf = one(&outer, b"traf").unwrap();
        let inner = children(&result, traf.at + 8, traf.at + traf.size, &mut nodes).unwrap();
        let tfhd = one(&inner, b"tfhd").unwrap();
        let trun = one(&inner, b"trun").unwrap();
        assert_eq!(read32(&result, tfhd.at + 8), Some(0x020008));
        assert_eq!(read32(&result, trun.at + 16), Some(1300));
        let mut original = EncodedMuxer::new(fixtures::inputs()).unwrap();
        assert!(original.push(1, &source).is_err());
        let mut accepted = EncodedMuxer::new(fixtures::inputs()).unwrap();
        accepted
            .push(0, &fixtures::fragment(0, 0, 6, true))
            .unwrap();
        accepted.push(1, &result).unwrap();
        assert!(!accepted.finish().unwrap().is_empty());
    }
    #[test]
    fn unsupported_tfhd_offsets_versions_indices_and_shapes_remain_rejected() {
        let source = explicit_description(&fixtures::fragment(0, 0, 4, true));
        let size = read32(&source, 0).unwrap() as usize;
        let (traf, tfhd, trun) = layout(&source[..size]).unwrap();
        for variant in 0..10 {
            let mut bytes = source.clone();
            match variant {
                0 => put32(&mut bytes, tfhd.at + 16, 0),
                1 => put32(&mut bytes, tfhd.at + 16, 2),
                2 => put32(&mut bytes, tfhd.at + 8, 0x020003),
                3 => put32(&mut bytes, tfhd.at + 8, 0x01020002),
                4 => put32(&mut bytes, tfhd.at + 8, 0x030002),
                5 => put32(&mut bytes, tfhd.at + 8, 2),
                6 => put32(&mut bytes, trun.at + 16, u32::MAX),
                7 => put32(&mut bytes, trun.at + 16, size as u32),
                8 => bytes[traf.at + 4..traf.at + 8].copy_from_slice(b"uuid"),
                _ => put32(&mut bytes, trun.at + 8, 0x02000f01),
            }
            let output = adapt(0, &[&bytes]);
            assert_eq!(output, bytes, "variant {variant}");
            let mut mux = EncodedMuxer::new(fixtures::inputs()).unwrap();
            assert!(mux.push(0, &output).is_err(), "variant {variant}");
        }
    }
    #[test]
    fn bounds_incomplete_state_and_provider_gates_never_disappear() {
        for provider in ["soop", "chzzk", "YouTube", ""] {
            assert!(Adapter::for_validated_inputs(provider, &fixtures::inputs()).is_none());
        }
        let source = explicit_description(&fixtures::fragment(0, 0, 4, true));
        for length in 1..source.len() {
            let mut adapter =
                Adapter::for_validated_inputs("youtube", &fixtures::inputs()[..1]).unwrap();
            adapter.push(0, &source[..length], |_| Ok(())).unwrap();
            let moof_end = read32(&source, 0).unwrap() as usize;
            assert_eq!(adapter.incomplete(), length != moof_end);
        }
        let mut large = vec![0; MAX_MOOF + 1];
        let size = large.len() as u32;
        put32(&mut large, 0, size);
        large[4..8].copy_from_slice(b"moof");
        assert_eq!(adapt(0, &[&large]), large);
        for size in [0u32, 1, 7, MAX_BOX as u32 + 1] {
            let input = [size.to_be_bytes().to_vec(), b"mdat".to_vec()].concat();
            let mut adapter =
                Adapter::for_validated_inputs("youtube", &fixtures::inputs()[..1]).unwrap();
            assert!(adapter.push(0, &input, |_| Ok(())).is_err());
        }
        let mut adapter =
            Adapter::for_validated_inputs("youtube", &fixtures::inputs()[..1]).unwrap();
        assert!(adapter.push(1, &source, |_| Ok(())).is_err());
    }
    #[test]
    fn duplicate_traf_trun_and_truncated_fields_are_not_normalized() {
        let source = explicit_description(&fixtures::fragment(0, 0, 4, true));
        let size = read32(&source, 0).unwrap() as usize;
        let (traf, _, trun) = layout(&source[..size]).unwrap();
        for duplicate in [traf, trun] {
            let mut bytes = source[..size].to_vec();
            let copied = bytes[duplicate.at..duplicate.at + duplicate.size].to_vec();
            let insertion = duplicate.at + duplicate.size;
            bytes.splice(insertion..insertion, copied);
            put32(&mut bytes, 0, (size + duplicate.size) as u32);
            if duplicate.kind == *b"trun" {
                put32(&mut bytes, traf.at, (traf.size + duplicate.size) as u32);
            }
            let before = bytes.clone();
            assert!(!normalize_moof(&mut bytes, None));
            assert_eq!(bytes, before);
            bytes.extend_from_slice(&source[size..]);
            let mut mux = EncodedMuxer::new(fixtures::inputs()).unwrap();
            assert!(mux.push(0, &bytes).is_err());
        }
        for end in 0..size {
            let mut bytes = source[..end].to_vec();
            if end >= 4 {
                put32(&mut bytes, 0, end as u32);
            }
            let before = bytes.clone();
            assert!(!normalize_moof(&mut bytes, None));
            assert_eq!(bytes, before);
        }
    }
}
