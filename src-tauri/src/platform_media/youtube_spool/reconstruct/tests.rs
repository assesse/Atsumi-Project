use super::super::{
    recovery::SequentialSource, schema::hash, Chunk, Mode, Owner, Runtime, SpoolWriter, Window,
    LIMITS,
};
use super::*;
use std::{cell::Cell, io::Cursor, sync::Arc};

fn identity() -> SpoolIdentity {
    SpoolIdentity {
        job_id: uuid::Uuid::new_v4().to_string(),
        request_id: uuid::Uuid::new_v4().to_string(),
        receiver_id: uuid::Uuid::new_v4().to_string(),
        source_id: uuid::Uuid::new_v4().to_string(),
        document_id: uuid::Uuid::new_v4().to_string(),
        document_epoch: 1,
        source_generation: 1,
        video_id: "abcdefghijk".into(),
        channel_id: format!("UC{}", "a".repeat(22)),
        owner: Owner::Manual,
    }
}
fn descriptor() -> ClockDescriptor {
    ClockDescriptor {
        video_timestamp_scale_ns: 1_000_000,
        audio_timescale: 44_100,
        mode: Mode::Segments,
        append_window: Window::Default,
        video_timestamp_offset: 0,
        audio_timestamp_offset: 0,
    }
}
fn units(track: Track, bytes: &[u8]) -> Vec<Unit> {
    let mut units = Vec::new();
    let result = index::scan(
        if track == Track::Video {
            Format::Vp9Webm
        } else {
            Format::AacMp4
        },
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        if track == Track::Video {
            1_000_000
        } else {
            44_100
        },
        &AtomicBool::new(false),
        &mut |unit| {
            units.push(unit);
            Ok(())
        },
    );
    assert_eq!(result.terminal, index::Terminal::Complete);
    units
}
fn scope(bytes: &[u8], track: Track) -> Scope {
    let mut tracks = std::array::from_fn(|_| TrackExtent {
        bytes: 1,
        sha256: hash(b"x"),
        next_append: 1,
    });
    tracks[track.index()] = TrackExtent {
        bytes: bytes.len() as u64,
        sha256: hash(bytes),
        next_append: 1,
    };
    Scope {
        identity: identity(),
        clock: descriptor(),
        tracks,
        receipts: 2,
        state: PrefixState::Sealed,
        seal_cause: Some(SealCause::Stopped),
        incomplete_append: false,
        torn_journal: false,
        uncommitted_bytes: [0; 2],
    }
}
fn locate(bytes: &[u8], needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .position(|part| part == needle)
        .unwrap()
}
fn ebml(id: u64, body: &[u8]) -> Vec<u8> {
    let id = id.to_be_bytes();
    let first = id.iter().position(|b| *b != 0).unwrap();
    let mut bytes = id[first..].to_vec();
    let width = (1..=8)
        .find(|n| (body.len() as u64) < ((1u64 << (n * 7)) - 1))
        .unwrap();
    let length = (body.len() as u64 | (1u64 << (width * 7))).to_be_bytes();
    bytes.extend_from_slice(&length[8 - width..]);
    bytes.extend_from_slice(body);
    bytes
}
fn video(base: u16, void: usize) -> Vec<u8> {
    let mut bytes = index::authored_pair()[0].clone();
    let tick = locate(&bytes, &[0xe7, 0x82, 0x03, 0xe8]);
    bytes[tick + 2..tick + 4].copy_from_slice(&base.to_be_bytes());
    if void != 0 {
        let at = units(Track::Video, &bytes)[0].facts.init_end as usize;
        bytes.splice(at..at, ebml(0xec, &vec![0; void]));
    }
    bytes
}
fn audio(tick: u64) -> Vec<u8> {
    let mut bytes = index::authored_pair()[1].clone();
    let at = locate(&bytes, b"tfdt") + 8;
    bytes[at..at + 8].copy_from_slice(&tick.to_be_bytes());
    bytes
}
fn pair(track: Track) -> Vec<u8> {
    match track {
        Track::Video => [video(1000, 7), video(1100, 8)].concat(),
        Track::Audio => [audio(0), audio(3072)].concat(),
    }
}

// Fault fixtures exercise the same private implementation with native scan
// records. The externally reachable builder still accepts only UnitSpan.
fn push<W: Write, R: Read + Seek>(
    builder: &mut TrackBuilder<'_, W>,
    scope: &Scope,
    unit: &Unit,
    reader: &mut R,
    guard: &impl Fn() -> Result<()>,
) -> Result<()> {
    builder.available()?;
    builder.failed = Some(Error::Order);
    let result = builder.push_inner(builder.track, scope, unit, reader, guard);
    builder.failed = result.as_ref().err().copied();
    result
}

#[test]
fn held_source_both_tracks_join_without_mutating_originals() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("YouTube")).unwrap();
    let source = [pair(Track::Video), pair(Track::Audio)];
    let id = identity();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut writer = SpoolWriter::create(
        root.path(),
        id.clone(),
        descriptor(),
        cancel.clone(),
        LIMITS,
        Runtime {
            elapsed: Some(0),
            available: Some(u64::MAX),
            ..Runtime::default()
        },
    )
    .unwrap();
    for (at, track) in [Track::Video, Track::Audio].into_iter().enumerate() {
        writer
            .append(
                &id,
                Chunk {
                    track,
                    append_index: 0,
                    chunk_index: 0,
                    final_chunk: true,
                    bytes: source[at].clone(),
                },
            )
            .unwrap();
    }
    writer.seal(&id, SealCause::Stopped).unwrap();
    drop(writer);
    let mut source_lease = SequentialSource::prepare(root.path(), &id, &cancel).unwrap();
    for (at, track) in [Track::Video, Track::Audio].into_iter().enumerate() {
        let source_units = units(track, &source[at]);
        let mut output = Vec::new();
        let mut builder = TrackBuilder::new(track, &mut output);
        source_lease
            .visit(track, &cancel, |span| builder.push(span, &cancel))
            .unwrap();
        let summary = builder.finish(&cancel).unwrap();
        assert_eq!(builder.finish(&cancel).unwrap_err(), Error::Sealed);
        drop(builder);
        let mut expected = source[at][..source_units[0].facts.end as usize].to_vec();
        let mut tail = source[at][source_units[1].facts.init_end as usize..].to_vec();
        if track == Track::Audio {
            let sequence = locate(&tail, b"mfhd") + 8;
            tail[sequence..sequence + 4].copy_from_slice(&2u32.to_be_bytes());
        }
        expected.extend(tail);
        assert_eq!(output, expected);
        assert_eq!(summary.track, track);
        assert_eq!(summary.bytes, output.len() as u64);
        assert_eq!(summary.sha256, <[u8; 32]>::from(Sha256::digest(&output)));
        assert_eq!(summary.units, 2);
        assert_eq!(summary.fragments, 2);
        assert_eq!(summary.clock.first, source_units[0].facts.clock.first);
        assert_eq!(summary.clock.end, source_units[1].facts.clock.end);
        assert_eq!(summary.state, PrefixState::Sealed);
        assert!(!summary.incomplete_append && !summary.torn_journal);
        assert_eq!(summary.uncommitted_bytes, [0; 2]);
        let derived = units(track, &output);
        assert_eq!(derived.len(), 1);
        assert_eq!(derived[0].facts.clock, summary.clock);
        assert!(derived[0].codec_packet_validation_pending);
        // The shared lease remains readable in a second exact visit.
        source_lease
            .visit(track, &cancel, |mut span| {
                let bounds = span.facts().unit.facts.clone();
                let mut reader = span.take_reader()?;
                reader.seek(SeekFrom::Start(bounds.start)).unwrap();
                let mut bytes = vec![0; (bounds.end - bounds.start) as usize];
                reader.read_exact(&mut bytes).unwrap();
                assert_eq!(
                    bytes,
                    source[at][bounds.start as usize..bounds.end as usize]
                );
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn only_direct_segment_void_is_ignored_in_init_comparison() {
    let a = video(1000, 279);
    let b = video(1100, 278);
    let init_a = &a[..units(Track::Video, &a)[0].facts.init_end as usize];
    let init_b = &b[..units(Track::Video, &b)[0].facts.init_end as usize];
    assert_ne!(init_a, init_b);
    assert_eq!(
        webm_comparison(init_a).unwrap(),
        webm_comparison(init_b).unwrap()
    );
    let base = ebml(0x1a45dfa3, b"header");
    let make = |nested: &[u8], other: &[u8]| {
        [
            base.clone(),
            vec![0x18, 0x53, 0x80, 0x67, 0xff],
            ebml(0x1549a966, nested),
            ebml(0x1654ae6b, other),
        ]
        .concat()
    };
    assert_ne!(
        webm_comparison(&make(&ebml(0xec, b"a"), b"tracks")).unwrap(),
        webm_comparison(&make(&ebml(0xec, b"b"), b"tracks")).unwrap()
    );
    assert_ne!(
        webm_comparison(&make(b"info", b"uid1")).unwrap(),
        webm_comparison(&make(b"info", b"uid2")).unwrap()
    );
    let mut crc = init_a.to_vec();
    crc.extend(ebml(0xbf, &[0; 4]));
    assert_eq!(webm_comparison(&crc), Err(Error::Proof));
    let mut finite = base.clone();
    finite.extend(ebml(0x18538067, &make(b"i", b"t")[base.len() + 5..]));
    assert_eq!(webm_comparison(&finite), Err(Error::Proof));
    let mut wide = base;
    wide.extend([
        0x18, 0x53, 0x80, 0x67, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    ]);
    wide.extend(ebml(0x1549a966, b"i"));
    wide.extend(ebml(0x1654ae6b, b"t"));
    assert!(webm_comparison(&wide).is_ok());
    assert!(webm_comparison(&vec![0; METADATA + 1]).is_err());
}

#[test]
fn unknown_segment_and_all_relocation_risks_are_fail_closed() {
    let bytes = video(1000, 0);
    let source = scope(&bytes, Track::Video);
    let original = units(Track::Video, &bytes).remove(0);
    for at in 0..7 {
        let mut unit = original.clone();
        let ReconstructionFacts::Webm(ref mut r) = unit.facts.reconstruction else {
            unreachable!()
        };
        match at {
            0 => r.unknown_segment = false,
            1 => r.segment_crc = true,
            2 => r.info_duration = true,
            3 => r.cluster_position = true,
            4 => r.cluster_prev_size = true,
            5 => r.unknown_cluster_crc = true,
            _ => r.finite_cluster_crc = true,
        }
        let mut output = Vec::new();
        let builder = TrackBuilder::new(Track::Video, &mut output);
        assert_eq!(
            builder.validate(Track::Video, &source, &unit),
            Err(Error::Proof)
        );
    }
    let bytes = audio(0);
    let source = scope(&bytes, Track::Audio);
    let original = units(Track::Audio, &bytes).remove(0);
    for at in 0..5 {
        let mut unit = original.clone();
        let ReconstructionFacts::Aac(ref mut r) = unit.facts.reconstruction else {
            unreachable!()
        };
        match at {
            0 => r.all_fragments_default_base_is_moof = false,
            1 => r.movie_duration = HeaderDuration::Finite(1),
            2 => r.track_duration = HeaderDuration::UnknownSentinel,
            3 => r.media_duration = HeaderDuration::Finite(1),
            _ => r.last_sequence += 1,
        }
        let mut output = Vec::new();
        let builder = TrackBuilder::new(Track::Audio, &mut output);
        assert_eq!(
            builder.validate(Track::Audio, &source, &unit),
            Err(Error::Proof)
        );
    }
}

#[test]
fn source_scope_order_clock_config_and_init_are_exact() {
    let bytes = pair(Track::Video);
    let source = scope(&bytes, Track::Video);
    let indexed = units(Track::Video, &bytes);
    for change in 0..9 {
        let mut output = Vec::new();
        let mut builder = TrackBuilder::new(Track::Video, &mut output);
        let mut reader = Cursor::new(&bytes);
        push(&mut builder, &source, &indexed[0], &mut reader, &|| Ok(())).unwrap();
        let mut altered = source.clone();
        let mut unit = indexed[1].clone();
        match change {
            0 => altered.identity.request_id = uuid::Uuid::new_v4().to_string(),
            1 => altered.tracks[0].sha256 = "0".repeat(64),
            2 => altered.clock.audio_timescale += 1,
            3 => altered.receipts += 1,
            4 => unit.ordinal += 1,
            5 => unit.facts.start += 1,
            6 => unit.facts.clock.first += 1,
            7 => unit.facts.configuration.width += 1,
            _ => unit.facts.init_sha256[0] ^= 1,
        }
        reader.set_position(5);
        assert!(push(&mut builder, &altered, &unit, &mut reader, &|| Ok(())).is_err());
        assert_eq!(reader.position(), 5);
        assert!(builder.finish(&AtomicBool::new(false)).is_err());
    }
    // MuxingApp is not native decoder config, but remains exact init metadata.
    let first = video(1000, 0);
    let mut second = video(1100, 0);
    let at = locate(&second, &[0x4d, 0x80, 0x81, b'A']);
    second[at + 3] = b'B';
    let bytes = [first, second].concat();
    let indexed = units(Track::Video, &bytes);
    assert_eq!(
        indexed[0].facts.configuration,
        indexed[1].facts.configuration
    );
    let source = scope(&bytes, Track::Video);
    let mut output = Vec::new();
    let mut builder = TrackBuilder::new(Track::Video, &mut output);
    let mut reader = Cursor::new(&bytes);
    push(&mut builder, &source, &indexed[0], &mut reader, &|| Ok(())).unwrap();
    assert_eq!(
        push(&mut builder, &source, &indexed[1], &mut reader, &|| Ok(())),
        Err(Error::Proof)
    );
}

fn box_bytes(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    [
        ((body.len() + 8) as u32).to_be_bytes().to_vec(),
        kind.to_vec(),
        body.to_vec(),
    ]
    .concat()
}
#[test]
fn mfhd_rewrite_changes_only_four_structurally_identified_bytes() {
    let mfhd = box_bytes(
        b"mfhd",
        &[vec![0; 4], 17u32.to_be_bytes().to_vec()].concat(),
    );
    let original = box_bytes(
        b"moof",
        &[mfhd.clone(), box_bytes(b"traf", b"opaque mfhd data")].concat(),
    );
    let mut altered = original.clone();
    rewrite_sequence(&mut altered, 17, 2).unwrap();
    let field = 8 + 8 + 4;
    let mut expected = original.clone();
    expected[field..field + 4].copy_from_slice(&2u32.to_be_bytes());
    assert_eq!(altered, expected);
    for malformed in [
        box_bytes(
            b"moof",
            &[mfhd.clone(), mfhd.clone(), box_bytes(b"traf", b"")].concat(),
        ),
        box_bytes(b"moof", &box_bytes(b"traf", &mfhd)),
        box_bytes(b"moof", &mfhd),
        box_bytes(
            b"moof",
            &[box_bytes(b"mfhd", &[0; 7]), box_bytes(b"traf", b"")].concat(),
        ),
        box_bytes(
            b"moof",
            &[mfhd, box_bytes(b"traf", b""), vec![0; 4]].concat(),
        ),
    ] {
        let mut copy = malformed.clone();
        assert!(rewrite_sequence(&mut copy, 17, 2).is_err());
        assert_eq!(copy, malformed);
    }
    let mut wrong_version = original.clone();
    wrong_version[16] = 1;
    assert!(rewrite_sequence(&mut wrong_version, 17, 2).is_err());
    assert!(rewrite_sequence(&mut original.clone(), 18, 2).is_err());
    assert!(rewrite_sequence(&mut original.clone(), 17, 0).is_err());
    let mut bad_length = original;
    bad_length[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(rewrite_sequence(&mut bad_length, 17, 2).is_err());
}

struct FaultWrite {
    bytes: Vec<u8>,
    fail_after: Option<usize>,
    flush_error: bool,
    cancel: Option<Arc<AtomicBool>>,
}
impl Write for FaultWrite {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.fail_after == Some(self.bytes.len()) {
            return Err(std::io::ErrorKind::Other.into());
        }
        let n = bytes
            .len()
            .min(3)
            .min(self.fail_after.unwrap_or(usize::MAX) - self.bytes.len());
        self.bytes.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
        if self.flush_error {
            Err(std::io::ErrorKind::Other.into())
        } else {
            Ok(())
        }
    }
}
#[test]
fn partial_write_flush_and_post_flush_cancel_latch_without_success() {
    let bytes = audio(0);
    let source = scope(&bytes, Track::Audio);
    let unit = units(Track::Audio, &bytes).remove(0);
    for mode in 0..3 {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut output = FaultWrite {
            bytes: vec![],
            fail_after: (mode == 0).then_some(7),
            flush_error: mode == 1,
            cancel: (mode == 2).then_some(cancel.clone()),
        };
        let mut builder = TrackBuilder::new(Track::Audio, &mut output);
        let mut reader = Cursor::new(&bytes);
        reader.set_position(4);
        let pushed = push(&mut builder, &source, &unit, &mut reader, &|| {
            checkpoint(&cancel)
        });
        assert_eq!(reader.position(), 4);
        let error = if mode == 0 {
            pushed.unwrap_err()
        } else {
            pushed.unwrap();
            builder.finish(&cancel).unwrap_err()
        };
        assert_eq!(
            error,
            if mode == 2 {
                Error::Cancelled
            } else {
                Error::Io
            }
        );
        assert_eq!(builder.finish(&AtomicBool::new(false)).unwrap_err(), error);
        assert_eq!(
            push(&mut builder, &source, &unit, &mut reader, &|| Ok(())),
            Err(error)
        );
    }
}

#[test]
fn read_error_guard_revocation_panic_and_restore_failure_are_terminal() {
    let bytes = audio(0);
    let source = scope(&bytes, Track::Audio);
    let unit = units(Track::Audio, &bytes).remove(0);
    let mut output = Vec::new();
    let mut builder = TrackBuilder::new(Track::Audio, &mut output);
    let mut reader = Cursor::new(&bytes[..bytes.len() - 1]);
    reader.set_position(6);
    assert_eq!(
        push(&mut builder, &source, &unit, &mut reader, &|| Ok(())),
        Err(Error::Io)
    );
    assert_eq!(reader.position(), 6);
    assert_eq!(
        builder.finish(&AtomicBool::new(false)).unwrap_err(),
        Error::Io
    );
    for panic in [false, true] {
        let mut output = Vec::new();
        let mut builder = TrackBuilder::new(Track::Audio, &mut output);
        let mut reader = Cursor::new(&bytes);
        reader.set_position(7);
        let calls = Cell::new(0);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            push(&mut builder, &source, &unit, &mut reader, &|| {
                let n = calls.get() + 1;
                calls.set(n);
                if n == 6 {
                    if panic {
                        panic!("injected");
                    }
                    return Err(Error::Corrupt);
                }
                Ok(())
            })
        }));
        assert_eq!(reader.position(), 7);
        if panic {
            assert!(outcome.is_err());
        } else {
            assert_eq!(outcome.unwrap(), Err(Error::Corrupt));
        }
        assert!(builder.finish(&AtomicBool::new(false)).is_err());
    }
    struct FailingRestore(Cursor<Vec<u8>>);
    impl Seek for FailingRestore {
        fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
            if to == SeekFrom::Start(7) {
                Err(std::io::ErrorKind::Other.into())
            } else {
                self.0.seek(to)
            }
        }
    }
    let mut reader = FailingRestore(Cursor::new(vec![0; 10]));
    reader.0.set_position(7);
    assert_eq!(
        with_restored_cursor(&mut reader, |r| {
            r.seek(SeekFrom::Start(1)).unwrap();
            Ok(())
        }),
        Err(Error::Io)
    );
}

#[test]
fn finish_requires_full_extent_and_preserves_interrupted_evidence() {
    let bytes = pair(Track::Video);
    let mut source = scope(&bytes, Track::Video);
    source.state = PrefixState::Interrupted;
    source.seal_cause = None;
    source.incomplete_append = true;
    source.torn_journal = true;
    source.uncommitted_bytes = [7, 9];
    let indexed = units(Track::Video, &bytes);
    let mut output = Vec::new();
    let mut builder = TrackBuilder::new(Track::Video, &mut output);
    let mut reader = Cursor::new(&bytes);
    push(&mut builder, &source, &indexed[0], &mut reader, &|| Ok(())).unwrap();
    assert_eq!(
        builder.finish(&AtomicBool::new(false)).unwrap_err(),
        Error::Proof
    );
    let mut output = Vec::new();
    let mut builder = TrackBuilder::new(Track::Video, &mut output);
    for unit in &indexed {
        push(&mut builder, &source, unit, &mut reader, &|| Ok(())).unwrap();
    }
    let summary = builder.finish(&AtomicBool::new(false)).unwrap();
    assert_eq!(summary.state, PrefixState::Interrupted);
    assert_eq!(summary.seal_cause, None);
    assert!(summary.incomplete_append && summary.torn_journal);
    assert_eq!(summary.uncommitted_bytes, [7, 9]);
}

#[test]
fn checked_limits_empty_finish_and_aac_sequence_overflow_reject() {
    let bytes = audio(0);
    let source = scope(&bytes, Track::Audio);
    let unit = units(Track::Audio, &bytes).remove(0);
    for limit in 0..4 {
        let mut output = Vec::new();
        let mut builder = TrackBuilder::new(Track::Audio, &mut output);
        match limit {
            0 => builder.units = MAX_UNITS,
            1 => builder.bytes = MAX_BYTES,
            2 => builder.fragments = u64::from(u32::MAX),
            _ => builder.boxes = MAX_BOXES,
        }
        assert_eq!(
            push(
                &mut builder,
                &source,
                &unit,
                &mut Cursor::new(&bytes),
                &|| Ok(())
            ),
            Err(Error::Bounds)
        );
        assert_eq!(
            builder.finish(&AtomicBool::new(false)).unwrap_err(),
            Error::Bounds
        );
    }
    let mut output = Vec::new();
    let mut builder = TrackBuilder::new(Track::Audio, &mut output);
    assert_eq!(
        builder.finish(&AtomicBool::new(false)).unwrap_err(),
        Error::Proof
    );
    let mut output = Vec::new();
    let mut builder = TrackBuilder::new(Track::Audio, &mut output);
    let cancel = AtomicBool::new(true);
    assert_eq!(
        push(
            &mut builder,
            &source,
            &unit,
            &mut Cursor::new(&bytes),
            &|| checkpoint(&cancel)
        ),
        Err(Error::Cancelled)
    );
    drop(builder);
    assert!(output.is_empty());
}
