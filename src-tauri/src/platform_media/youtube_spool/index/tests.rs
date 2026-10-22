use super::*;
use std::io::Cursor;

fn element(id: u64, body: &[u8]) -> Vec<u8> {
    let encoded = id.to_be_bytes();
    let first = encoded.iter().position(|b| *b != 0).unwrap();
    let mut out = encoded[first..].to_vec();
    let n = (1..=8)
        .find(|n| (body.len() as u64) < ((1u64 << (7 * n)) - 1))
        .unwrap();
    let length = (body.len() as u64) | (1u64 << (7 * n));
    out.extend_from_slice(&length.to_be_bytes()[8 - n..]);
    out.extend_from_slice(body);
    out
}
fn integer(id: u64, value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let at = bytes.iter().position(|b| *b != 0).unwrap_or(7);
    element(id, &bytes[at..])
}
fn unknown(id: u64, body: &[u8]) -> Vec<u8> {
    let mut out = id.to_be_bytes()[4..].to_vec();
    out.push(0xff);
    out.extend_from_slice(body);
    out
}
fn simple(relative: i16, key: bool, payload: &[u8]) -> Vec<u8> {
    let mut b = vec![0x81];
    b.extend_from_slice(&relative.to_be_bytes());
    b.push(if key { 0x80 } else { 0 });
    b.extend_from_slice(payload);
    element(0xa3, &b)
}
fn fixture(base: u64, width: u64, app: &[u8], extra_track: &[u8], finite: bool) -> Vec<u8> {
    let ebml = element(
        0x1a45dfa3,
        &[element(0x4282, b"webm"), integer(0x4285, 2)].concat(),
    );
    let info = element(
        0x1549a966,
        &[integer(0x2ad7b1, 1_000_000), element(0x4d80, app)].concat(),
    );
    let track = element(
        0xae,
        &[
            integer(0xd7, 1),
            integer(0x73c5, 1),
            integer(0x83, 1),
            element(0x86, b"V_VP9"),
            integer(0x23e383, 33_333_333),
            element(0xe0, &[integer(0xb0, width), integer(0xba, 480)].concat()),
            extra_track.to_vec(),
        ]
        .concat(),
    );
    let tracks = element(0x1654ae6b, &track);
    let block = element(0xa1, &[0x81, 0, 66, 0, 0x42]);
    let group = element(
        0xa0,
        &[block, integer(0x9b, 34), element(0xfb, &[0xdf])].concat(),
    );
    // The encoded payload deliberately contains EBML and ftyp magic. It is
    // opaque data, never a reason to start a new initialization unit.
    let body = [
        integer(0xe7, base),
        simple(
            0,
            true,
            &[0x82, 0x1a, 0x45, 0xdf, 0xa3, b'f', b't', b'y', b'p'],
        ),
        simple(33, false, &[0]),
        group,
    ]
    .concat();
    let cluster = if finite {
        element(0x1f43b675, &body)
    } else {
        unknown(0x1f43b675, &body)
    };
    let body = [info, tracks, cluster].concat();
    [
        ebml,
        if finite {
            element(0x18538067, &body)
        } else {
            unknown(0x18538067, &body)
        },
    ]
    .concat()
}
pub(super) fn authored_video() -> Vec<u8> {
    fixture(1000, 854, b"A", &[], false)
}
fn run(bytes: &[u8]) -> (Outcome, Vec<Unit>) {
    let mut result = Vec::new();
    let cancel = AtomicBool::new(false);
    let outcome = scan(
        Format::Vp9Webm,
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        1_000_000,
        &cancel,
        &mut |unit| {
            result.push(unit);
            Ok(())
        },
    );
    (outcome, result)
}
fn relocation_fixture(
    finite_segment: bool,
    finite_cluster: bool,
    extra_info: &[u8],
    before_cluster: &[u8],
    extra_cluster: &[u8],
    after_cluster: &[u8],
) -> Vec<u8> {
    let header = element(0x1a45dfa3, &element(0x4282, b"webm"));
    let info = element(
        0x1549a966,
        &[integer(0x2ad7b1, 1_000_000), extra_info.to_vec()].concat(),
    );
    let track = element(
        0xae,
        &[
            integer(0xd7, 1),
            integer(0x83, 1),
            element(0x86, b"V_VP9"),
            integer(0x23e383, 33_333_333),
            element(0xe0, &[integer(0xb0, 854), integer(0xba, 480)].concat()),
        ]
        .concat(),
    );
    let cluster_body = [
        integer(0xe7, 1000),
        simple(0, true, &[0x82]),
        extra_cluster.to_vec(),
    ]
    .concat();
    let cluster = if finite_cluster {
        element(0x1f43b675, &cluster_body)
    } else {
        unknown(0x1f43b675, &cluster_body)
    };
    let segment = [
        info,
        element(0x1654ae6b, &track),
        before_cluster.to_vec(),
        cluster,
        after_cluster.to_vec(),
    ]
    .concat();
    [
        header,
        if finite_segment {
            element(0x18538067, &segment)
        } else {
            unknown(0x18538067, &segment)
        },
    ]
    .concat()
}
fn webm_reconstruction(unit: &Unit) -> WebmReconstruction {
    match unit.facts.reconstruction {
        ReconstructionFacts::Webm(facts) => facts,
        _ => panic!("wrong reconstruction track"),
    }
}

fn initialization_parts(scale: Option<u64>, extra_info: &[u8], extra_track: &[u8]) -> [Vec<u8>; 3] {
    [
        element(0x1a45dfa3, &element(0x4282, b"webm")),
        element(0x1549a966, &[
            scale.map(|value| integer(0x2ad7b1, value)).unwrap_or_default(),
            extra_info.to_vec(),
        ].concat()),
        element(0x1654ae6b, &element(0xae, &[
            integer(0xd7, 1), integer(0x83, 1), element(0x86, b"V_VP9"),
            integer(0x23e383, 33_333_333),
            element(0xe0, &[integer(0xb0, 854), integer(0xba, 480)].concat()),
            extra_track.to_vec(),
        ].concat())),
    ]
}
fn initialization_video(parts: &[Vec<u8>; 3], padding: &[u8]) -> Vec<u8> {
    [parts[0].clone(), unknown(0x18538067, &[
        parts[1].clone(), parts[2].clone(), padding.to_vec(),
    ].concat())].concat()
}

#[test]
fn initialization_facts_match_full_index_without_packet_authority() {
    let cancel = AtomicBool::new(false);
    for (format, bytes, expected, track) in [
        (Format::Vp9Webm, authored_video(), 1_000_000, super::super::Track::Video),
        (Format::AacMp4, mp4::authored_audio(), 44_100, super::super::Track::Audio),
    ] {
        let mut units = Vec::new();
        let outcome = scan(format, &mut Cursor::new(&bytes), bytes.len() as u64,
            expected, &cancel, &mut |unit| { units.push(unit); Ok(()) });
        assert_eq!(outcome.terminal, Terminal::Complete);
        assert_eq!(units.len(), 1);
        let unit = &units[0];
        let prefix = &bytes[..unit.facts.init_end as usize];
        let facts = inspect_initialization(prefix, format, &cancel).unwrap();
        assert_eq!(facts.track(), track);
        assert_eq!(facts.timebase(), expected);
        assert_eq!(facts.len(), prefix.len());
        assert_eq!(facts.init_sha256(), unit.facts.init_sha256);
        assert_eq!(facts.configuration_sha256(), unit.configuration_sha256);
        assert!(unit.codec_packet_validation_pending);
    }
}

#[test]
fn vp9_initialization_accepts_native_scale_unknown_vint8_and_void_only_difference() {
    let cancel = AtomicBool::new(false);
    for scale in [None, Some(1), Some(1_000_000), Some(1_000_000_000)] {
        let parts = initialization_parts(scale, &[], &[]);
        let original = initialization_video(&parts, &[]);
        let facts = inspect_vp9_initialization(&original, &cancel).unwrap();
        assert_eq!(facts.timebase(), scale.unwrap_or(1_000_000));
        let long_segment = [
            parts[0].clone(), vec![0x18, 0x53, 0x80, 0x67, 1, 255, 255, 255, 255, 255, 255, 255],
            parts[1].clone(), parts[2].clone(),
        ].concat();
        let padded = [
            parts[0].clone(), unknown(0x18538067, &[
                element(0xec, &[3; 8]), parts[1].clone(), element(0xec, &[4; 2]),
                parts[2].clone(), element(0xec, &[5; 12]),
            ].concat()),
        ].concat();
        for variation in [long_segment, padded] {
            let other = inspect_vp9_initialization(&variation, &cancel).unwrap();
            assert_eq!(facts.configuration_sha256(), other.configuration_sha256());
            assert_ne!(facts.init_sha256(), other.init_sha256());
        }
    }
}

#[test]
fn vp9_initialization_rejects_partial_duplicate_order_media_and_finite_segment() {
    let cancel = AtomicBool::new(false);
    let parts = initialization_parts(Some(1_000_000), &[], &[]);
    let valid = initialization_video(&parts, &[]);
    for end in 0..valid.len() {
        assert!(inspect_vp9_initialization(&valid[..end], &cancel).is_err(), "prefix {end}");
    }
    for body in [
        [parts[1].clone(), parts[1].clone(), parts[2].clone()].concat(),
        [parts[2].clone(), parts[1].clone()].concat(),
        [parts[1].clone(), parts[2].clone(), parts[2].clone()].concat(),
        parts[1].clone(), parts[2].clone(), Vec::new(),
    ] {
        let bytes = [parts[0].clone(), unknown(0x18538067, &body)].concat();
        assert!(inspect_vp9_initialization(&bytes, &cancel).is_err());
    }
    for tail in [
        unknown(0x1f43b675, &integer(0xe7, 0)), simple(0, true, &[0x82]),
        parts[0].clone(), element(0xbf, &[0; 4]), element(0xa4, &[]), vec![0],
    ] {
        assert!(inspect_vp9_initialization(&[valid.clone(), tail].concat(), &cancel).is_err());
    }
    let finite = [parts[0].clone(), element(0x18538067, &[parts[1].clone(), parts[2].clone()].concat())].concat();
    assert!(inspect_vp9_initialization(&finite, &cancel).is_err());
}

#[test]
fn vp9_initialization_preserves_unsafe_configuration_and_reconstruction_rejections() {
    let cancel = AtomicBool::new(false);
    for scale in [0, 1_000_000_001] {
        assert!(inspect_vp9_initialization(&initialization_video(
            &initialization_parts(Some(scale), &[], &[]), &[]), &cancel).is_err());
    }
    for extra in [
        element(0x23314f, &2f64.to_be_bytes()), integer(0x56aa, 1), integer(0x56bb, 1),
        element(0x6d80, &[]), element(0x3cb923, &[1]), element(0xa4, &[]),
        integer(0xd7, 2), element(0x86, b"V_VP8"),
    ] {
        assert!(inspect_vp9_initialization(&initialization_video(
            &initialization_parts(Some(1_000_000), &[], &extra), &[]), &cancel).is_err());
    }
    let duration = initialization_parts(Some(1_000_000), &element(0x4489, &1f64.to_be_bytes()), &[]);
    assert!(inspect_vp9_initialization(&initialization_video(&duration, &[]), &cancel).is_err());
    let mut two = initialization_parts(Some(1_000_000), &[], &[]);
    // Both entries are separately well-formed, not an unknown nested element.
    let track = element(0xae, &[
        integer(0xd7, 1), integer(0x83, 1), element(0x86, b"V_VP9"),
        element(0xe0, &[integer(0xb0, 854), integer(0xba, 480)].concat()),
    ].concat());
    two[2] = element(0x1654ae6b, &[track.clone(), track].concat());
    assert!(inspect_vp9_initialization(&initialization_video(&two, &[]), &cancel).is_err());
}

#[test]
fn initialization_byte_limit_and_cancel_are_fail_closed() {
    let cancel = AtomicBool::new(false);
    let parts = initialization_parts(None, &[], &[]);
    let base = initialization_video(&parts, &[]);
    // Void's one-byte ID plus three-byte size VINT exactly fills the cap.
    let padding = element(0xec, &vec![0; METADATA - base.len() - 4]);
    let exact = initialization_video(&parts, &padding);
    assert_eq!(exact.len(), METADATA);
    assert_eq!(inspect_vp9_initialization(&exact, &cancel).unwrap().len(), METADATA);
    for inspect in [inspect_vp9_initialization, inspect_aac_initialization] {
        assert_eq!(inspect(&[], &cancel).unwrap_err(), super::super::Error::Proof);
        assert_eq!(inspect(&vec![0; METADATA + 1], &cancel).unwrap_err(), super::super::Error::Bounds);
        assert_eq!(inspect(&base, &AtomicBool::new(true)).unwrap_err(), super::super::Error::Cancelled);
    }
}

#[test]
fn reconstruction_segment_and_cluster_sizes_do_not_change_scan_admission() {
    for segment in [false, true] {
        for cluster in [false, true] {
            let bytes = relocation_fixture(segment, cluster, &[], &[], &[], &[]);
            let (outcome, units) = run(&bytes);
            assert_eq!(outcome.terminal, Terminal::Complete);
            assert_eq!(units.len(), 1);
            assert_eq!(units[0].facts.end, bytes.len() as u64);
            assert_eq!(units[0].facts.clock.first, 1_000_000_000);
            assert_eq!(
                webm_reconstruction(&units[0]),
                WebmReconstruction {
                    unknown_segment: !segment,
                    ..WebmReconstruction::default()
                }
            );
            assert!(units[0].codec_packet_validation_pending);
        }
    }
}

#[test]
fn reconstruction_duration_positions_and_crc_scopes_are_only_facts() {
    let crc = element(0xbf, &[0; 4]);
    for variant in 0..8 {
        let mut expected = WebmReconstruction {
            unknown_segment: true,
            ..WebmReconstruction::default()
        };
        let bytes = match variant {
            0 => {
                expected.segment_crc = true;
                relocation_fixture(false, true, &[], &crc, &[], &[])
            }
            1 => {
                expected.segment_crc = true;
                relocation_fixture(false, true, &[], &[], &[], &crc)
            }
            2 => {
                expected.info_duration = true;
                relocation_fixture(
                    false,
                    false,
                    &element(0x4489, &1f64.to_be_bytes()),
                    &[],
                    &[],
                    &[],
                )
            }
            3 => {
                expected.cluster_position = true;
                relocation_fixture(false, false, &[], &[], &integer(0xa7, 100), &[])
            }
            4 => {
                expected.cluster_prev_size = true;
                relocation_fixture(false, false, &[], &[], &integer(0xab, 50), &[])
            }
            5 => {
                expected.unknown_cluster_crc = true;
                relocation_fixture(false, false, &[], &[], &crc, &[])
            }
            6 => {
                expected.finite_cluster_crc = true;
                relocation_fixture(false, true, &[], &[], &crc, &[])
            }
            // A CRC inside fixed-size Info is not a Segment or Cluster CRC.
            _ => relocation_fixture(false, false, &crc, &[], &[], &[]),
        };
        let (outcome, units) = run(&bytes);
        assert_eq!(outcome.terminal, Terminal::Complete, "variant {variant}");
        assert_eq!(
            webm_reconstruction(&units[0]),
            expected,
            "variant {variant}"
        );
        assert_eq!(units[0].facts.end, bytes.len() as u64);
        assert!(units[0].codec_packet_validation_pending);
    }
}

#[test]
fn reconstruction_facts_reset_at_each_initialization_and_remain_provisional() {
    let first = relocation_fixture(false, false, &[], &[], &element(0xbf, &[0; 4]), &[]);
    let second = relocation_fixture(false, false, &[], &[], &[], &[]);
    let bytes = [first.clone(), second.clone()].concat();
    let (outcome, units) = run(&bytes);
    assert_eq!(outcome.terminal, Terminal::Complete);
    assert!(webm_reconstruction(&units[0]).unknown_cluster_crc);
    assert!(!webm_reconstruction(&units[1]).unknown_cluster_crc);
    assert_eq!(units[1].init_relation, InitRelation::IdenticalValidatedInit);
    assert_eq!(units[1].clock_relation, ClockRelation::OverlapOrReset);
    assert_eq!(units[0].facts.end, first.len() as u64);
    let partial = [first, second[..second.len() - 1].to_vec()].concat();
    let (outcome, units) = run(&partial);
    assert!(matches!(outcome.terminal, Terminal::PartialTail { .. }));
    assert_eq!(units.len(), 1);
    assert!(units[0].codec_packet_validation_pending);
}
struct Short<'a> {
    inner: Cursor<&'a [u8]>,
    chunk: usize,
}
impl Read for Short<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let count = bytes.len().min(self.chunk);
        self.inner.read(&mut bytes[..count])
    }
}
impl Seek for Short<'_> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[test]
fn webm_exact_offsets_clock_and_opaque_payload() {
    let bytes = fixture(1000, 854, b"A", &[], false);
    let (outcome, units) = run(&bytes);
    assert_eq!(outcome.terminal, Terminal::Complete);
    assert_eq!(units.len(), 1);
    let u = &units[0];
    assert_eq!(u.facts.start, 0);
    assert_eq!(u.facts.end, bytes.len() as u64);
    assert_eq!(
        u.facts.clock,
        Clock {
            first: 1_000_000_000,
            last: 1_066_000_000,
            end: 1_100_000_000,
            ticks_per_second: 1_000_000_000
        }
    );
    assert_eq!(u.facts.items, 3);
    assert_eq!(u.facts.references, 1);
    assert!(u.facts.rap_candidate);
    assert!(u.codec_packet_validation_pending);
    assert_eq!(u.facts.boundary, Boundary::CommittedEof);
}
#[test]
fn every_short_read_size_is_equivalent() {
    let bytes = fixture(1000, 854, b"A", &[], false);
    let expected = run(&bytes);
    for chunk in 1..=bytes.len() {
        let mut reader = Short {
            inner: Cursor::new(bytes.as_slice()),
            chunk,
        };
        let mut units = Vec::new();
        let out = scan(
            Format::Vp9Webm,
            &mut reader,
            bytes.len() as u64,
            1_000_000,
            &AtomicBool::new(false),
            &mut |u| {
                units.push(u);
                Ok(())
            },
        );
        assert_eq!((out, units), expected, "chunk {chunk}");
    }
}
#[test]
fn repeated_init_equivalence_change_and_unproved_difference_are_separate() {
    let bytes = [
        fixture(1000, 854, b"A", &[], false),
        fixture(1100, 854, b"A", &[], false),
        fixture(1200, 854, b"B", &[], false),
        fixture(1300, 1280, b"B", &[], false),
    ]
    .concat();
    let (out, units) = run(&bytes);
    assert_eq!(out.terminal, Terminal::Complete);
    assert_eq!(units.len(), 4);
    assert_eq!(units[1].init_relation, InitRelation::IdenticalValidatedInit);
    assert_eq!(units[2].init_relation, InitRelation::UnprovedInitDifference);
    assert_eq!(
        units[3].init_relation,
        InitRelation::ChangedParsedConfiguration
    );
    assert!(units[1..]
        .iter()
        .all(|u| u.clock_relation == ClockRelation::Contiguous));
    assert_eq!(units[0].facts.boundary, Boundary::NextInitialization);
}
#[test]
fn finite_segment_and_cluster_end_without_successor() {
    let (out, units) = run(&fixture(1000, 854, b"A", &[], true));
    assert_eq!(out.terminal, Terminal::Complete);
    assert_eq!(units[0].facts.boundary, Boundary::FiniteEnd);
}
#[test]
fn unsupported_clock_encryption_and_duplicate_track_fields_fail() {
    for extra in [
        element(0x23314f, &2f64.to_be_bytes()),
        integer(0x56aa, 1),
        element(0x6d80, &[]),
        integer(0xd7, 2),
    ] {
        let (out, units) = run(&fixture(1000, 854, b"A", &extra, false));
        assert!(matches!(out.terminal, Terminal::Failed(_)));
        assert!(units.is_empty());
    }
}
#[test]
fn partial_tail_is_not_interior_corruption_or_short_backing_file() {
    let complete = fixture(1000, 854, b"A", &[], true);
    let mut partial = complete.clone();
    partial.extend([0x1a, 0x45]);
    let (out, units) = run(&partial);
    assert_eq!(out.complete_units, 1);
    assert_eq!(units.len(), 1);
    assert!(matches!(out.terminal, Terminal::PartialTail { .. }));
    let mut malformed = complete;
    malformed.extend([0, 0, 0, 0]);
    let (out, _) = run(&malformed);
    assert!(matches!(
        out.terminal,
        Terminal::Failed(Error {
            kind: Failure::Malformed,
            ..
        })
    ));
    let mut backing = Cursor::new(vec![0x1a]);
    let out = scan(
        Format::Vp9Webm,
        &mut backing,
        100,
        1_000_000,
        &AtomicBool::new(false),
        &mut |_| Ok(()),
    );
    assert!(matches!(
        out.terminal,
        Terminal::Failed(Error {
            kind: Failure::Io,
            ..
        })
    ));
}
#[test]
fn partial_final_block_is_never_emitted() {
    let bytes = fixture(1000, 854, b"A", &[], false);
    for remove in 1..=8 {
        let (out, units) = run(&bytes[..bytes.len() - remove]);
        assert!(!matches!(out.terminal, Terminal::Complete));
        assert!(units.is_empty());
    }
}
#[test]
fn source_timebase_mismatch_cancellation_and_sink_failure_stay_terminal() {
    let bytes = fixture(1000, 854, b"A", &[], false);
    let out = scan(
        Format::Vp9Webm,
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        44100,
        &AtomicBool::new(false),
        &mut |_| Ok(()),
    );
    assert!(matches!(
        out.terminal,
        Terminal::Failed(Error {
            kind: Failure::Clock,
            ..
        })
    ));
    let out = scan(
        Format::Vp9Webm,
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        1_000_000,
        &AtomicBool::new(true),
        &mut |_| Ok(()),
    );
    assert!(matches!(
        out.terminal,
        Terminal::Failed(Error {
            kind: Failure::Cancelled,
            ..
        })
    ));
    let out = scan(
        Format::Vp9Webm,
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        1_000_000,
        &AtomicBool::new(false),
        &mut |_| Err(()),
    );
    assert!(matches!(
        out.terminal,
        Terminal::Failed(Error {
            kind: Failure::Sink,
            ..
        })
    ));
    assert_eq!(out.complete_units, 0);
}
#[test]
fn exact_committed_limit_ignores_uncommitted_backing_tail() {
    let complete = fixture(1000, 854, b"A", &[], true);
    let mut bytes = complete.clone();
    bytes.extend([0; 32]);
    let mut units = 0;
    let out = scan(
        Format::Vp9Webm,
        &mut Cursor::new(bytes),
        complete.len() as u64,
        1_000_000,
        &AtomicBool::new(false),
        &mut |_| {
            units += 1;
            Ok(())
        },
    );
    assert_eq!(out.terminal, Terminal::Complete);
    assert_eq!(units, 1);
}
#[test]
fn long_repeated_sequence_streams_without_accumulating_records() {
    let bytes = (0..2000)
        .flat_map(|i| fixture(1000 + i * 100, 854, b"A", &[], false))
        .collect::<Vec<_>>();
    let mut count = 0u64;
    let out = scan(
        Format::Vp9Webm,
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        1_000_000,
        &AtomicBool::new(false),
        &mut |u| {
            assert_eq!(u.ordinal, count);
            count += 1;
            Ok(())
        },
    );
    assert_eq!(out.terminal, Terminal::Complete);
    assert_eq!(count, 2000);
}
#[test]
fn cross_unit_gap_and_reset_are_facts_not_hidden_normalization() {
    let bytes = [
        fixture(1000, 854, b"A", &[], false),
        fixture(1200, 854, b"A", &[], false),
        fixture(100, 854, b"A", &[], false),
    ]
    .concat();
    let (out, units) = run(&bytes);
    assert_eq!(out.terminal, Terminal::Complete);
    assert_eq!(units[1].clock_relation, ClockRelation::Gap);
    assert_eq!(units[2].clock_relation, ClockRelation::OverlapOrReset);
}

#[test]
fn eight_byte_unknown_and_finite_ebml_sizes_do_not_shift_u8_by_eight() {
    for finite in [false, true] {
        let mut bytes = fixture(1000, 854, b"A", &[], false);
        let at = bytes
            .windows(4)
            .position(|w| w == [0x18, 0x53, 0x80, 0x67])
            .unwrap()
            + 4;
        let size = if finite {
            (1u64 << 56) | (bytes.len() - at - 1) as u64
        } else {
            0x01ff_ffff_ffff_ffff
        };
        bytes.splice(at..at + 1, size.to_be_bytes());
        let (out, units) = run(&bytes);
        assert_eq!(out.terminal, Terminal::Complete);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].facts.end, bytes.len() as u64);
    }
    let mut bytes = fixture(1000, 854, b"A", &[], false);
    let at = bytes
        .windows(4)
        .position(|w| w == [0x1f, 0x43, 0xb6, 0x75])
        .unwrap()
        + 4;
    bytes.splice(at..at + 1, 0x01ff_ffff_ffff_ffffu64.to_be_bytes());
    assert_eq!(run(&bytes).0.terminal, Terminal::Complete);
}

#[test]
fn oversized_declaration_is_limit_not_recoverable_eof_tail() {
    let mut bytes = fixture(1000, 854, b"A", &[], false);
    let at = bytes
        .windows(4)
        .position(|w| w == [0x18, 0x53, 0x80, 0x67])
        .unwrap()
        + 4;
    bytes.splice(at..at + 1, ((1u64 << 56) | (MAX_RAW + 1)).to_be_bytes());
    assert!(matches!(
        run(&bytes).0.terminal,
        Terminal::Failed(Error {
            kind: Failure::Limit,
            ..
        })
    ));
    let mut bytes = fixture(1000, 854, b"A", &[], false);
    let marker = simple(33, false, &[0]);
    let at = bytes
        .windows(marker.len())
        .position(|w| w == marker)
        .unwrap()
        + 1;
    bytes.splice(at..at + 1, ((1u64 << 56) | (MAX_ITEM + 1)).to_be_bytes());
    assert!(matches!(
        run(&bytes).0.terminal,
        Terminal::Failed(Error {
            kind: Failure::Limit,
            ..
        })
    ));
}

#[test]
fn missing_cluster_timecode_and_wrong_crc_cannot_hide_at_sibling_boundary() {
    let mut bytes = fixture(1000, 854, b"A", &[], false);
    let at = bytes
        .windows(4)
        .position(|w| w == [0x1f, 0x43, 0xb6, 0x75])
        .unwrap();
    bytes.splice(at..at, unknown(0x1f43b675, &[]));
    assert!(matches!(
        run(&bytes).0.terminal,
        Terminal::Failed(Error {
            kind: Failure::Malformed,
            ..
        })
    ));
    let mut bytes = fixture(1000, 854, b"A", &[], false);
    let at = bytes
        .windows(4)
        .position(|w| w == [0x1f, 0x43, 0xb6, 0x75])
        .unwrap()
        + 5;
    bytes.splice(at..at, element(0xbf, &[0; 3]));
    assert!(matches!(
        run(&bytes).0.terminal,
        Terminal::Failed(Error {
            kind: Failure::Malformed,
            ..
        })
    ));
}

#[test]
fn empty_input_and_late_cancel_do_not_report_complete() {
    assert_eq!(run(&[]).0.terminal, Terminal::PartialTail { offset: 0 });
    let bytes = fixture(1000, 854, b"A", &[], false);
    let cancel = AtomicBool::new(false);
    let out = scan(
        Format::Vp9Webm,
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        1_000_000,
        &cancel,
        &mut |_| {
            cancel.store(true, Ordering::Release);
            Ok(())
        },
    );
    assert_eq!(out.complete_units, 1);
    assert!(matches!(
        out.terminal,
        Terminal::Failed(Error {
            kind: Failure::Cancelled,
            ..
        })
    ));
}

#[test]
fn skipped_payload_still_requires_full_committed_backing_extent() {
    let mut bytes = fixture(1000, 854, b"A", &[], false);
    let group = element(
        0xa0,
        &[
            element(0xa1, &[0x81, 0, 66, 0, 0x42]),
            integer(0x9b, 34),
            element(0xfb, &[0xdf]),
        ]
        .concat(),
    );
    assert!(bytes.ends_with(&group));
    bytes.truncate(bytes.len() - group.len());
    bytes.extend(simple(66, false, &[0x42; 64]));
    let declared = bytes.len() as u64;
    bytes.truncate(bytes.len() - 16);
    let out = scan(
        Format::Vp9Webm,
        &mut Cursor::new(bytes),
        declared,
        1_000_000,
        &AtomicBool::new(false),
        &mut |_| Ok(()),
    );
    assert!(matches!(
        out.terminal,
        Terminal::Failed(Error {
            kind: Failure::Io,
            ..
        })
    ));
    assert_eq!(out.complete_units, 0);
}

#[test]
fn unknown_structural_id_is_unsupported_even_when_its_body_is_short() {
    let mut bytes = fixture(1000, 854, b"A", &[], true);
    bytes.extend([0x81, 0x8a]); // Unknown structural ID, declares10 missing bytes.
    let (out, units) = run(&bytes);
    assert_eq!(units.len(), 1);
    assert!(matches!(
        out.terminal,
        Terminal::Failed(Error {
            kind: Failure::Unsupported,
            ..
        })
    ));
}

#[test]
#[ignore = "Explicit root-only read-only test of the approved fixed public capture; missing evidence is not a pass"]
fn actual_raw_pair_is_indexed_without_copy_or_modification() {
    use super::super::files::{Directories, GuardedFile};
    use std::path::Path;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace");
    let directory =
        workspace.join(".runtime/youtube-webm-raw-pair-qa/0ab34128-d5cf-43b2-8fad-2d2fe9cc89c2");
    let directories =
        Directories::hold(&directory).expect("approved fixture directories unavailable");
    for (name, length, expected, format, timebase, count, init) in [
        (
            "unit-1.webm",
            124392u64,
            "0c4a795839e97419442f0a03d734e11c98f5381b3be4a121ecb0ab47a837119f",
            Format::Vp9Webm,
            1_000_000,
            150,
            452,
        ),
        (
            "unit-2.mp4",
            82189u64,
            "5baa9ceeebe7a370079c3f639951679983e5efede39ae0e9cf5f81940c6c75a5",
            Format::AacMp4,
            44100,
            216,
            632,
        ),
    ] {
        let mut file = GuardedFile::open(&directory.join(name), false)
            .expect("approved fixed fixture unavailable");
        assert_eq!(file.check().unwrap(), length);
        directories.check().unwrap();
        let cancel = AtomicBool::new(false);
        let before = {
            let mut input = Input {
                reader: &mut file.file,
                len: length,
                cancel: &cancel,
            };
            input.hash_range(0, length).unwrap()
        };
        assert_eq!(
            before
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            expected
        );
        let mut units = Vec::new();
        let outcome = scan(
            format,
            &mut file.file,
            length,
            timebase,
            &cancel,
            &mut |u| {
                units.push(u);
                Ok(())
            },
        );
        assert_eq!(outcome.terminal, Terminal::Complete, "{name}: {outcome:?}");
        assert_eq!(units.len(), 1);
        let u = &units[0];
        println!("fixed {name} reconstruction: {:?}", u.facts.reconstruction);
        assert_eq!(u.facts.end, length);
        assert_eq!(u.facts.init_end, init);
        assert_eq!(u.facts.items, count);
        if format == Format::Vp9Webm {
            assert_eq!(
                (u.facts.configuration.width, u.facts.configuration.height),
                (854, 480)
            );
            assert_eq!(u.facts.clock.first, 46_800_001_000_000);
            assert_eq!(u.facts.clock.end, 46_805_001_000_000);
        } else {
            assert_eq!(
                (
                    u.facts.configuration.sample_rate,
                    u.facts.configuration.channels
                ),
                (44100, 2)
            );
            assert_eq!(u.facts.clock.first, 2_063_879_393);
            assert_eq!(u.facts.clock.end, 2_063_879_393 + 216 * 1024);
        }
        let after = {
            let mut input = Input {
                reader: &mut file.file,
                len: length,
                cancel: &cancel,
            };
            input.hash_range(0, length).unwrap()
        };
        assert_eq!(before, after);
        assert_eq!(file.check().unwrap(), length);
        directories.check().unwrap();
    }
}

#[test]
#[ignore = "Explicit root-only read-only inspection of the fixed V3 four-file capture; missing evidence fails"]
fn actual_v3_four_units_preserve_distinct_init_evidence() {
    use super::super::files::{Directories, GuardedFile};
    use std::path::Path;
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace")
        .join(".runtime/youtube-webm-raw-pair-qa/c26c9147-735f-4ed3-821b-31e92cef9afd");
    let directories = Directories::hold(&directory).expect("approved fixture directories unavailable");
    let cancel = AtomicBool::new(false);
    let hash = |file: &mut GuardedFile, length| {
        Input { reader: &mut file.file, len: length, cancel: &cancel }
            .hash_range(0, length)
            .expect("fixed fixture hash")
    };
    let hex = |bytes: [u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let mut manifest = GuardedFile::open(&directory.join("manifest.json"), false)
        .expect("fixed manifest unavailable");
    let manifest_length = manifest.check().unwrap();
    assert!((1..=16_384).contains(&manifest_length));
    let manifest_hash = hash(&mut manifest, manifest_length);
    assert_eq!(hex(manifest_hash), "c4a92fbff4e9c27870b817e1d2015cb66990eaf9ec47568ead953026c15508ad");
    let fixtures = [
        ("video-1.webm", 95_635u64, "9be82861467e27ed66fa8220db1a7c4f1e2872996755257cbeafe149035070a4", Format::Vp9Webm, 1_000_000, 452),
        ("video-2.webm", 76_753, "7bb5f6e6f3fcd435042dc5b7227ca023799014f7f3e7886e4a99bc732451a4b6", Format::Vp9Webm, 1_000_000, 451),
        ("audio-1.mp4", 81_810, "c349dc8672a8ade3d961d19c7d0b30f09473b08b96c7eb58641cbaceb385779a", Format::AacMp4, 44_100, 632),
        ("audio-2.mp4", 82_185, "b9fa4f91f7dc48bee2a9f7128188f27b3b041b6faf16f91e19b7a3e17daf7457", Format::AacMp4, 44_100, 632),
    ];
    // Keep every read-only handle and its directory guards until all post-checks.
    let mut held = fixtures.iter().map(|(name, ..)| {
        GuardedFile::open(&directory.join(name), false).expect("fixed media unavailable")
    }).collect::<Vec<_>>();
    let mut units = Vec::with_capacity(4);
    for (slot, (file, &(name, length, expected, format, timebase, init))) in
        held.iter_mut().zip(&fixtures).enumerate()
    {
        directories.check().unwrap();
        assert_eq!(file.check().unwrap(), length);
        assert_eq!(hex(hash(file, length)), expected);
        let mut single = None;
        let outcome = scan(format, &mut file.file, length, timebase, &cancel, &mut |unit| {
            if single.is_some() { return Err(()); }
            single = Some(unit);
            Ok(())
        });
        assert_eq!(outcome.terminal, Terminal::Complete, "{name}: {outcome:?}");
        assert_eq!(outcome.complete_units, 1);
        let unit = single.expect("exactly one structural unit");
        assert_eq!(unit.facts.start, 0);
        assert_eq!(unit.facts.end, length);
        assert_eq!(unit.facts.init_end, init);
        assert_eq!(unit.facts.configuration.format, format);
        assert_eq!(unit.facts.configuration.timebase, timebase);
        assert!(unit.codec_packet_validation_pending);
        let clock = unit.facts.clock;
        println!("v3 unit={} items={} fragments={} references={} rap={} clock={},{},{},{}",
            slot + 1, unit.facts.items, unit.facts.fragments, unit.facts.references,
            u8::from(unit.facts.rap_candidate), clock.first, clock.last, clock.end, clock.ticks_per_second);
        match unit.facts.reconstruction {
            ReconstructionFacts::Webm(facts) => println!(
                "v3 unit={} webm_reconstruction={},{},{},{},{},{},{}", slot + 1,
                u8::from(facts.unknown_segment), u8::from(facts.segment_crc), u8::from(facts.info_duration),
                u8::from(facts.cluster_position), u8::from(facts.cluster_prev_size),
                u8::from(facts.unknown_cluster_crc), u8::from(facts.finite_cluster_crc)),
            ReconstructionFacts::Aac(facts) => {
                let duration = |value| match value {
                    HeaderDuration::Zero => (0u8, 0u64),
                    HeaderDuration::UnknownSentinel => (1, 0),
                    HeaderDuration::Finite(ticks) => (2, ticks),
                };
                let movie = duration(facts.movie_duration);
                let track = duration(facts.track_duration);
                let media = duration(facts.media_duration);
                println!("v3 unit={} aac_reconstruction={},{},{},{},{},{},{},{},{}", slot + 1,
                    u8::from(facts.all_fragments_default_base_is_moof), facts.first_sequence,
                    facts.last_sequence, movie.0, movie.1, track.0, track.1, media.0, media.1);
            }
        }
        units.push(unit);
    }
    // Parsed configuration equality is not whole-codec or packet equivalence.
    for (first, second) in [(0, 1), (2, 3)] {
        assert_eq!(units[first].facts.configuration, units[second].facts.configuration);
        assert_eq!(units[first].configuration_sha256, units[second].configuration_sha256);
    }
    assert_ne!(units[0].facts.init_sha256, units[1].facts.init_sha256);
    assert_eq!(units[2].facts.init_sha256, units[3].facts.init_sha256);
    for (file, &(_, length, expected, ..)) in held.iter_mut().zip(&fixtures) {
        assert_eq!(file.check().unwrap(), length);
        assert_eq!(hex(hash(file, length)), expected);
        assert_eq!(file.check().unwrap(), length);
    }
    assert_eq!(manifest.check().unwrap(), manifest_length);
    assert_eq!(hash(&mut manifest, manifest_length), manifest_hash);
    assert_eq!(manifest.check().unwrap(), manifest_length);
    directories.check().unwrap();
}
