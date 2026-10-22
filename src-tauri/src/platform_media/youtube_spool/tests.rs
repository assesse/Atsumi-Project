use super::*;
use std::{fs, io::Write, sync::atomic::AtomicBool};

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
fn clock() -> ClockDescriptor {
    ClockDescriptor {
        video_timestamp_scale_ns: 1_000_000,
        audio_timescale: 44100,
        mode: Mode::Segments,
        append_window: Window::Default,
        video_timestamp_offset: 0,
        audio_timestamp_offset: 0,
    }
}
fn limits() -> Limits {
    Limits {
        chunk: 4,
        append: 8,
        raw: 64,
        elapsed_ms: 10000,
        receipts: 20,
        journal: 32768,
    }
}
fn chunk(track: Track, index: u64, part: u32, final_chunk: bool, bytes: &[u8]) -> Chunk {
    Chunk {
        track,
        append_index: index,
        chunk_index: part,
        final_chunk,
        bytes: bytes.to_vec(),
    }
}
struct Fixture {
    root: tempfile::TempDir,
    id: SpoolIdentity,
    limits: Limits,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("YouTube")).unwrap();
        Self {
            root,
            id: identity(),
            limits: limits(),
        }
    }
    fn writer(&self) -> SpoolWriter {
        self.with_runtime(Runtime::default()).unwrap()
    }
    fn with_runtime(&self, mut runtime: Runtime) -> Result<SpoolWriter> {
        // Real disk sync speed must not advance a test's simulated epoch.
        runtime.elapsed.get_or_insert(0);
        SpoolWriter::create(
            self.root.path(),
            self.id.clone(),
            clock(),
            Arc::new(AtomicBool::new(false)),
            self.limits,
            runtime,
        )
    }
    fn dir(&self) -> PathBuf {
        self.root.path().join("YouTube").join(&self.id.job_id)
    }
    fn inspect(&self) -> CommittedPrefix {
        recovery::inspect(
            self.root.path(),
            &self.id,
            &AtomicBool::new(false),
            self.limits,
        )
        .unwrap()
    }
    fn bytes(&self) -> Vec<Vec<u8>> {
        [PROOF, RAW[0], RAW[1], JOURNAL]
            .into_iter()
            .map(|name| fs::read(self.dir().join(name)).unwrap())
            .collect()
    }
}

#[test]
fn identity_descriptor_and_persisted_owner_are_closed() {
    let mut id = identity();
    id.owner = Owner::Scheduled {
        schedule_id: id.channel_id.clone(),
        run_id: uuid::Uuid::new_v4().to_string(),
        broadcast_key: id.video_id.clone(),
        generation: 7,
    };
    assert!(id.validate().is_ok());
    let encoded = serde_json::to_value(&id).unwrap();
    for owner in [Owner::Manual, id.owner.clone()] {
        let value = serde_json::to_value(&owner).unwrap();
        assert_eq!(
            serde_json::from_value::<Owner>(value.clone()).unwrap(),
            owner
        );
        let mut with_extra = value;
        with_extra["extra"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<Owner>(with_extra).is_err());
    }
    assert!(serde_json::from_str::<Owner>(r#"{"kind":"manual","kind":"manual"}"#).is_err());
    assert!(serde_json::from_value::<Owner>(serde_json::json!({"kind":"unknown"})).is_err());
    for field in ["jobId", "requestId", "receiverId", "sourceId", "documentId"] {
        let mut value = encoded.clone();
        value[field] = "not-a-uuid".into();
        assert!(serde_json::from_value::<SpoolIdentity>(value)
            .unwrap()
            .validate()
            .is_err());
    }
    for field in ["documentEpoch", "sourceGeneration"] {
        let mut value = encoded.clone();
        value[field] = 0.into();
        assert!(serde_json::from_value::<SpoolIdentity>(value)
            .unwrap()
            .validate()
            .is_err());
    }
    for (field, value) in [
        ("schedule_id", serde_json::json!("UCwrong")),
        ("run_id", serde_json::json!("bad")),
        ("broadcast_key", serde_json::json!("other")),
        ("generation", serde_json::json!(0)),
    ] {
        let mut bad = encoded.clone();
        bad["owner"][field] = value;
        assert!(serde_json::from_value::<SpoolIdentity>(bad)
            .unwrap()
            .validate()
            .is_err());
    }
    for owner in [
        serde_json::json!({"kind":"manual","extra":1}),
        serde_json::json!({"kind":"scheduled","schedule_id":id.channel_id,"run_id":uuid::Uuid::new_v4().to_string(),"broadcast_key":id.video_id,"generation":7,"extra":1}),
    ] {
        assert!(serde_json::from_value::<Owner>(owner).is_err());
    }
    let mut unknown = encoded;
    unknown["extra"] = true.into();
    assert!(serde_json::from_value::<SpoolIdentity>(unknown).is_err());
    for value in [0, 1_000_000_001, u64::MAX] {
        let mut descriptor = clock();
        descriptor.audio_timescale = value;
        assert_eq!(descriptor.validate(), Err(Error::Proof));
        descriptor = clock();
        descriptor.video_timestamp_scale_ns = value;
        assert_eq!(descriptor.validate(), Err(Error::Proof));
    }
    let mut descriptor = clock();
    descriptor.video_timestamp_offset = 1;
    assert!(descriptor.validate().is_err());
}

#[test]
fn every_ack_including_intermediate_chunk_is_durable_and_seal_is_not_media_proof() {
    let f = Fixture::new();
    let mut w = f.writer();
    let a = w
        .append(&f.id, chunk(Track::Video, 0, 0, false, b"abcd"))
        .unwrap();
    assert_eq!(a.committed_end, 4);
    let b = w
        .append(&f.id, chunk(Track::Video, 0, 1, true, b"ef"))
        .unwrap();
    let c = w
        .append(&f.id, chunk(Track::Audio, 0, 0, true, b"123"))
        .unwrap();
    assert_eq!((b.sequence, c.sequence), (1, 2));
    let seal = w.seal(&f.id, SealCause::Stopped).unwrap();
    assert_eq!(seal.receipts, 3);
    assert!(!seal.incomplete_append);
    assert_eq!(
        w.append(&f.id, chunk(Track::Audio, 1, 0, true, b"x")),
        Err(Error::Sealed)
    );
    drop(w);
    let before = f.bytes();
    let inspected = f.inspect();
    assert_eq!(inspected.state, PrefixState::Sealed);
    assert_eq!(inspected.receipts, 3);
    assert_eq!(inspected.tracks[0].sha256, hash(b"abcdef"));
    assert_eq!(inspected.tracks[1].sha256, hash(b"123"));
    drop(inspected);
    assert_eq!(f.bytes(), before);
}

#[test]
fn only_exact_global_last_receipt_can_repeat_and_foreign_calls_do_not_consume_writer() {
    let f = Fixture::new();
    let mut w = f.writer();
    let mut wrong = f.id.clone();
    wrong.document_epoch += 1;
    assert_eq!(w.seal(&wrong, SealCause::Stopped), Err(Error::Identity));
    assert_eq!(
        w.append(&wrong, chunk(Track::Video, 0, 0, true, b"a")),
        Err(Error::Identity)
    );
    let request = chunk(Track::Video, 0, 0, true, b"a");
    let first = w.append(&f.id, request.clone()).unwrap();
    let steps = w.runtime.steps;
    let duplicate = w.append(&f.id, request.clone()).unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.sequence, first.sequence);
    assert_eq!(w.runtime.steps, steps);
    w.append(&f.id, chunk(Track::Audio, 0, 0, true, b"b"))
        .unwrap();
    assert_eq!(w.append(&f.id, request), Err(Error::Order));
    assert_eq!(
        w.append(&f.id, chunk(Track::Video, 1, 0, true, b"c")),
        Err(Error::Order)
    );
}

#[test]
fn protocol_errors_poison_without_discarding_prior_prefix() {
    let cases = [
        chunk(Track::Audio, 0, 0, true, b"b"),
        chunk(Track::Video, 0, 2, true, b"b"),
        chunk(Track::Video, 1, 0, true, b"b"),
        chunk(Track::Video, 0, 1, false, b"b"),
        chunk(Track::Video, 0, 0, false, b"zzzz"),
    ];
    for bad in cases {
        let f = Fixture::new();
        let mut w = f.writer();
        w.append(&f.id, chunk(Track::Video, 0, 0, false, b"abcd"))
            .unwrap();
        assert!(w.append(&f.id, bad).is_err());
        assert!(w
            .append(&f.id, chunk(Track::Video, 0, 1, true, b"e"))
            .is_err());
        drop(w);
        let p = f.inspect();
        assert_eq!(p.receipts, 1);
        assert!(p.incomplete_append);
        assert_eq!(p.tracks[0].bytes, 4);
    }
}

#[test]
fn crash_windows_preserve_committed_or_uncommitted_bytes_without_false_ack() {
    for fail in [
        Step::BeforeRaw,
        Step::RawWrite,
        Step::RawSync,
        Step::ReceiptWrite,
        Step::ReceiptSync,
        Step::BeforeAck,
    ] {
        let f = Fixture::new();
        let mut w = f.writer();
        w.runtime.fail = Some(fail);
        assert_eq!(
            w.append(&f.id, chunk(Track::Video, 0, 0, true, b"abcd")),
            Err(Error::Io)
        );
        assert_eq!(
            w.append(&f.id, chunk(Track::Video, 0, 0, true, b"abcd")),
            Err(Error::Io)
        );
        drop(w);
        let p = f.inspect();
        let journal_received = matches!(
            fail,
            Step::ReceiptWrite | Step::ReceiptSync | Step::BeforeAck
        );
        assert_eq!(p.receipts, if journal_received { 1 } else { 0 }, "{fail:?}");
        assert_eq!(
            p.uncommitted_bytes[0],
            if matches!(fail, Step::RawWrite | Step::RawSync) {
                4
            } else {
                0
            },
            "{fail:?}"
        );
        assert_eq!(p.state, PrefixState::Interrupted);
    }
}

#[test]
fn cancellation_and_expiry_after_sync_are_not_successful_acks() {
    for step in [Step::RawSync, Step::ReceiptSync, Step::BeforeAck] {
        for expired in [false, true] {
            let f = Fixture::new();
            let mut w = f.writer();
            if expired {
                w.runtime.expire_at = Some(step);
            } else {
                w.runtime.cancel_at = Some(step);
            }
            let expected = if expired {
                Error::Expired
            } else {
                Error::Cancelled
            };
            assert_eq!(
                w.append(&f.id, chunk(Track::Video, 0, 0, true, b"abcd")),
                Err(expected)
            );
            assert_eq!(
                w.append(&f.id, chunk(Track::Video, 0, 0, true, b"abcd")),
                Err(expected)
            );
            drop(w);
            let p = f.inspect();
            assert_eq!(p.receipts, if step == Step::RawSync { 0 } else { 1 });
        }
    }
}

#[test]
fn creation_and_seal_failures_leave_evidence_and_do_not_reopen() {
    for fail in [
        Step::Create(0),
        Step::Create(1),
        Step::Create(2),
        Step::Create(3),
        Step::Create(4),
        Step::InitialRawSync(0),
        Step::InitialRawSync(1),
        Step::ProofWrite,
        Step::ProofSync,
        Step::HeaderWrite,
        Step::HeaderSync,
    ] {
        let f = Fixture::new();
        assert!(matches!(
            f.with_runtime(Runtime {
                fail: Some(fail),
                ..Default::default()
            }),
            Err(Error::Io)
        ));
        assert!(f.dir().is_dir());
        assert!(f.with_runtime(Runtime::default()).is_err());
    }
    for fail in [Step::BeforeSeal, Step::SealWrite, Step::SealSync] {
        let f = Fixture::new();
        let mut w = f.writer();
        w.append(&f.id, chunk(Track::Video, 0, 0, true, b"a"))
            .unwrap();
        w.runtime.fail = Some(fail);
        assert_eq!(w.seal(&f.id, SealCause::Stopped), Err(Error::Io));
        assert_eq!(
            w.append(&f.id, chunk(Track::Video, 1, 0, true, b"a")),
            Err(Error::Io)
        );
        drop(w);
        let p = f.inspect();
        assert_eq!(p.receipts, 1);
        assert_eq!(
            p.state,
            if fail == Step::BeforeSeal {
                PrefixState::Interrupted
            } else {
                PrefixState::Sealed
            }
        );
    }
}

#[test]
fn seal_rechecks_revocation_at_encoding_and_after_final_io() {
    for step in [Step::BeforeSeal, Step::SealSync] {
        for expired in [false, true] {
            let f = Fixture::new();
            let mut w = f.writer();
            w.append(&f.id, chunk(Track::Video, 0, 0, true, b"a"))
                .unwrap();
            if expired {
                w.runtime.expire_at = Some(step);
            } else {
                w.runtime.cancel_at = Some(step);
            }
            let result = w.seal(&f.id, SealCause::Stopped);
            if step == Step::BeforeSeal {
                assert_eq!(
                    result.unwrap().cause,
                    if expired {
                        SealCause::Limit
                    } else {
                        SealCause::Cancelled
                    }
                );
            } else {
                assert_eq!(
                    result,
                    Err(if expired {
                        Error::Expired
                    } else {
                        Error::Cancelled
                    })
                );
            }
            drop(w);
            assert_eq!(f.inspect().receipts, 1);
        }
    }
}

#[test]
fn exact_caps_allow_last_duplicate_but_new_admission_latches_failure() {
    for mode in 0..2 {
        let mut f = Fixture::new();
        if mode == 0 {
            f.limits.raw = 4;
        } else {
            f.limits.receipts = 1;
        }
        let mut w = f.writer();
        let request = chunk(Track::Video, 0, 0, true, b"abcd");
        w.append(&f.id, request.clone()).unwrap();
        assert!(w.append(&f.id, request.clone()).unwrap().duplicate);
        assert_eq!(
            w.append(&f.id, chunk(Track::Audio, 0, 0, true, b"b")),
            Err(Error::Bounds)
        );
        assert_eq!(w.append(&f.id, request), Err(Error::Bounds));
    }
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(&f.id, chunk(Track::Video, 0, 0, false, b"abcd"))
        .unwrap();
    w.append(&f.id, chunk(Track::Video, 0, 1, false, b"efgh"))
        .unwrap();
    assert_eq!(
        w.append(&f.id, chunk(Track::Video, 0, 2, true, b"i")),
        Err(Error::Bounds)
    );
}

#[test]
fn space_includes_raw_receipt_and_reserved_seal_and_unknown_space_stays_closed() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.runtime.available = Some(RESERVE + SEAL_ROOM + 4);
    assert_eq!(
        w.append(&f.id, chunk(Track::Video, 0, 0, true, b"abcd")),
        Err(Error::Space)
    );
    assert_eq!(w.raw[0].file.metadata().unwrap().len(), 0);
    w.runtime.available = Some(u64::MAX);
    assert_eq!(
        w.append(&f.id, chunk(Track::Video, 0, 0, true, b"abcd")),
        Err(Error::Space)
    );
}

#[test]
fn journal_byte_cap_is_independent_and_reserves_terminal_row() {
    let mut f = Fixture::new();
    f.limits.journal = 1200;
    let mut w = f.writer();
    assert_eq!(
        w.append(&f.id, chunk(Track::Video, 0, 0, true, b"a")),
        Err(Error::Bounds)
    );
    assert_eq!(w.progress.receipts, 0);
    let mut f = Fixture::new();
    f.limits.journal = 2048;
    let mut w = f.writer();
    w.append(&f.id, chunk(Track::Video, 0, 0, true, b"a"))
        .unwrap();
    w.seal(&f.id, SealCause::Stopped).unwrap();
    drop(w);
    assert_eq!(f.inspect().state, PrefixState::Sealed);
}

fn append_bytes(path: &Path, bytes: &[u8]) {
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
}
#[test]
fn recovery_distinguishes_only_final_torn_line_from_interior_corruption() {
    for tail in [
        b"{\"record\":".as_slice(),
        b"{}\n".as_slice(),
        b"{bad}\n{}\n".as_slice(),
    ] {
        let f = Fixture::new();
        let mut w = f.writer();
        w.append(&f.id, chunk(Track::Video, 0, 0, true, b"a"))
            .unwrap();
        drop(w);
        append_bytes(&f.dir().join(JOURNAL), tail);
        let before = f.bytes();
        let p = f.inspect();
        assert_eq!(
            p.state,
            if tail.ends_with(b"\n") {
                PrefixState::Corrupt
            } else {
                PrefixState::Interrupted
            }
        );
        assert_eq!(p.torn_journal, !tail.ends_with(b"\n"));
        assert_eq!(p.receipts, 1);
        drop(p);
        assert_eq!(before, f.bytes());
    }
    let f = Fixture::new();
    let w = f.writer();
    drop(w);
    append_bytes(&f.dir().join(JOURNAL), &vec![b'a'; 1025]);
    assert_eq!(f.inspect().state, PrefixState::Corrupt);
}

#[test]
fn recovery_rejects_hash_order_missing_bytes_extra_after_seal_and_unknown_proof() {
    for mode in 0..5 {
        let f = Fixture::new();
        let mut w = f.writer();
        w.append(&f.id, chunk(Track::Video, 0, 0, true, b"abcd"))
            .unwrap();
        if mode == 3 {
            w.seal(&f.id, SealCause::Stopped).unwrap();
        }
        drop(w);
        match mode {
            0 => {
                fs::write(f.dir().join(RAW[0]), b"xbcd").unwrap();
            }
            1 => {
                fs::write(f.dir().join(RAW[0]), b"ab").unwrap();
            }
            2 => {
                let bytes = fs::read(f.dir().join(JOURNAL)).unwrap();
                let changed = String::from_utf8(bytes)
                    .unwrap()
                    .replace("\"sequence\":0", "\"sequence\":1");
                fs::write(f.dir().join(JOURNAL), changed).unwrap();
            }
            3 => append_bytes(&f.dir().join(JOURNAL), b"{}\n"),
            _ => {
                let path = f.dir().join(PROOF);
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                value["extra"] = true.into();
                fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
            }
        }
        if mode == 4 {
            assert!(matches!(
                recovery::inspect(f.root.path(), &f.id, &AtomicBool::new(false), f.limits),
                Err(Error::Proof)
            ));
        } else {
            assert_eq!(f.inspect().state, PrefixState::Corrupt);
        }
    }
}

#[test]
fn partial_raw_tail_cancelled_inspection_and_cancelled_seal_are_honest() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(&f.id, chunk(Track::Video, 0, 0, false, b"abcd"))
        .unwrap();
    let seal = w.seal(&f.id, SealCause::Stopped).unwrap();
    assert_eq!(seal.cause, SealCause::Interrupted);
    drop(w);
    assert_eq!(f.inspect().state, PrefixState::Interrupted);
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(&f.id, chunk(Track::Video, 0, 0, true, b"a"))
        .unwrap();
    w.cancel.store(true, Ordering::Release);
    assert_eq!(
        w.seal(&f.id, SealCause::Stopped).unwrap().cause,
        SealCause::Cancelled
    );
    drop(w);
    assert_eq!(f.inspect().state, PrefixState::Cancelled);
    let p = recovery::inspect(f.root.path(), &f.id, &AtomicBool::new(true), f.limits).unwrap();
    assert_eq!(p.state, PrefixState::Cancelled);
}

#[test]
fn recovery_is_streaming_and_immutable_with_many_logical_appends() {
    let mut f = Fixture::new();
    f.limits.receipts = 1001;
    f.limits.raw = 1001;
    f.limits.journal = 1024 * 1024;
    let mut w = f.writer();
    for index in 0..1000 {
        w.append(&f.id, chunk(Track::Video, index, 0, true, b"x"))
            .unwrap();
    }
    drop(w);
    let before = f.bytes();
    let p = f.inspect();
    assert_eq!(p.receipts, 1000);
    assert_eq!(p.tracks[0].bytes, 1000);
    drop(p);
    assert_eq!(before, f.bytes());
}

#[test]
fn recovery_physical_raw_ceiling_includes_uncommitted_tail() {
    let mut f = Fixture::new();
    f.limits.raw = 4;
    let w = f.writer();
    drop(w);
    fs::write(f.dir().join(RAW[0]), b"abcd").unwrap();
    let p = f.inspect();
    assert_eq!(p.state, PrefixState::Interrupted);
    assert_eq!(p.uncommitted_bytes, [4, 0]);
    drop(p);
    append_bytes(&f.dir().join(RAW[0]), b"e");
    assert!(matches!(
        recovery::inspect(f.root.path(), &f.id, &AtomicBool::new(false), f.limits),
        Err(Error::Bounds)
    ));
    assert_eq!(fs::read(f.dir().join(RAW[0])).unwrap(), b"abcde");
}

#[test]
fn guarded_files_reject_links_replacement_and_existing_directory() {
    let f = Fixture::new();
    let w = f.writer();
    assert!(f.with_runtime(Runtime::default()).is_err());
    drop(w);
    let path = f.dir().join(RAW[0]);
    let link = f.root.path().join("linked.raw");
    fs::hard_link(&path, &link).unwrap();
    assert!(matches!(
        recovery::inspect(f.root.path(), &f.id, &AtomicBool::new(false), f.limits),
        Err(Error::Path)
    ));
}

#[cfg(windows)]
#[test]
fn windows_held_handles_deny_other_writes_deletes_and_reparse_roots() {
    let f = Fixture::new();
    let w = f.writer();
    assert!(fs::OpenOptions::new()
        .write(true)
        .open(f.dir().join(RAW[0]))
        .is_err());
    assert!(fs::remove_file(f.dir().join(RAW[0])).is_err());
    assert!(fs::rename(f.dir(), f.root.path().join("moved")).is_err());
    drop(w);
    let p = f.inspect();
    assert!(fs::OpenOptions::new()
        .write(true)
        .open(f.dir().join(RAW[0]))
        .is_err());
    drop(p);
    for path in [
        Path::new(r"\\server\share"),
        Path::new(r"\\?\C:\test"),
        Path::new(r"C:\test:stream"),
    ] {
        assert!(SpoolWriter::create_new(
            path,
            identity(),
            clock(),
            Arc::new(AtomicBool::new(false))
        )
        .is_err());
    }
}

#[cfg(unix)]
#[test]
fn unix_symlink_ancestors_are_rejected() {
    let f = Fixture::new();
    let linked = f.root.path().join("link");
    std::os::unix::fs::symlink(f.root.path(), &linked).unwrap();
    assert!(SpoolWriter::create_new(
        &linked,
        identity(),
        clock(),
        Arc::new(AtomicBool::new(false))
    )
    .is_err());
}

fn authored_spool(
    pair: [Vec<u8>; 2],
    descriptor: ClockDescriptor,
    cause: Option<SealCause>,
    incomplete_audio: bool,
) -> Fixture {
    let mut f = Fixture::new();
    f.limits = LIMITS;
    if incomplete_audio {
        // Only this fixture needs a full-size non-final chunk small enough to
        // end exactly at an authored complete audio unit.
        f.limits = limits();
        f.limits.chunk = pair[0].len().max(pair[1].len()) as u64;
        assert_eq!(pair[1].len() as u64, f.limits.chunk);
        f.limits.append = f.limits.chunk * 2;
        f.limits.raw = (pair[0].len() + pair[1].len()) as u64 + 4096;
    }
    let mut writer = SpoolWriter::create(
        f.root.path(),
        f.id.clone(),
        descriptor,
        Arc::new(AtomicBool::new(false)),
        f.limits,
        Runtime {
            elapsed: Some(0),
            ..Runtime::default()
        },
    )
    .unwrap();
    for (track, bytes) in [(Track::Video, &pair[0]), (Track::Audio, &pair[1])] {
        writer
            .append(
                &f.id,
                chunk(
                    track,
                    0,
                    0,
                    !(incomplete_audio && track == Track::Audio),
                    bytes,
                ),
            )
            .unwrap();
    }
    if let Some(cause) = cause {
        writer.seal(&f.id, cause).unwrap();
    }
    drop(writer);
    f
}
impl Fixture {
    fn sequential(&self, cancel: &AtomicBool) -> Result<recovery::SequentialSource> {
        if self.limits == LIMITS {
            recovery::SequentialSource::prepare(self.root.path(), &self.id, cancel)
        } else {
            recovery::SequentialSource::prepare_fixture(
                self.root.path(),
                &self.id,
                cancel,
                self.limits,
            )
        }
    }
    fn select(
        &self,
        cancel: &AtomicBool,
        track: Track,
        ordinal: u64,
    ) -> Result<recovery::SelectedUnit> {
        if self.limits == LIMITS {
            recovery::select_unit(self.root.path(), &self.id, cancel, track, ordinal)
        } else {
            recovery::select_unit_fixture(
                self.root.path(),
                &self.id,
                cancel,
                track,
                ordinal,
                self.limits,
            )
        }
    }
    fn index(
        &self,
        cancel: &AtomicBool,
        sink: &mut dyn FnMut(Track, index::Unit) -> std::result::Result<(), ()>,
    ) -> Result<(CommittedPrefix, [Option<index::Outcome>; 2])> {
        if self.limits == LIMITS {
            recovery::inspect_and_index(self.root.path(), &self.id, cancel, sink)
        } else {
            recovery::inspect_and_index_fixture(
                self.root.path(),
                &self.id,
                cancel,
                self.limits,
                sink,
            )
        }
    }
}
fn assert_both_structurally_complete(outcomes: [Option<index::Outcome>; 2]) {
    for outcome in outcomes {
        assert_eq!(outcome.map(|o| o.terminal), Some(index::Terminal::Complete));
    }
}

#[test]
fn index_wrapper_binds_two_authored_tracks_and_retains_held_source() {
    let pair = index::authored_pair();
    let expected = [pair[0].len() as u64, pair[1].len() as u64];
    let f = authored_spool(pair, clock(), Some(SealCause::Stopped), false);
    let before = f.bytes();
    let mut seen = Vec::new();
    let (prefix, outcomes) = f
        .index(&AtomicBool::new(false), &mut |track, unit| {
            seen.push((
                track,
                unit.facts.configuration.format,
                unit.facts.configuration.timebase,
                unit.facts.end,
            ));
            assert!(unit.codec_packet_validation_pending);
            Ok(())
        })
        .unwrap();
    assert_both_structurally_complete(outcomes);
    assert_eq!(prefix.identity, f.id);
    assert_eq!(prefix.state, PrefixState::Sealed);
    assert_eq!(prefix.seal_cause, Some(SealCause::Stopped));
    assert_eq!(
        seen,
        vec![
            (Track::Video, index::Format::Vp9Webm, 1_000_000, expected[0]),
            (Track::Audio, index::Format::AacMp4, 44100, expected[1]),
        ]
    );
    #[cfg(windows)]
    for name in [PROOF, JOURNAL, RAW[0], RAW[1]] {
        assert!(fs::OpenOptions::new()
            .write(true)
            .open(f.dir().join(name))
            .is_err());
    }
    drop(prefix);
    assert_eq!(before, f.bytes());
}

#[test]
fn index_wrapper_uses_proof_clocks_without_guessing_or_scanning_after_failure() {
    for track in [Track::Video, Track::Audio] {
        let mut descriptor = clock();
        match track {
            Track::Video => descriptor.video_timestamp_scale_ns = 2_000_000,
            Track::Audio => descriptor.audio_timescale = 48000,
        }
        let f = authored_spool(
            index::authored_pair(),
            descriptor,
            Some(SealCause::Stopped),
            false,
        );
        let before = f.bytes();
        let mut seen = Vec::new();
        let (prefix, outcomes) = f
            .index(&AtomicBool::new(false), &mut |track, _| {
                seen.push(track);
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            outcomes[track.index()],
            Some(index::Outcome {
                terminal: index::Terminal::Failed(index::Error {
                    kind: index::Failure::Clock,
                    ..
                }),
                ..
            })
        ));
        if track == Track::Video {
            assert_eq!(outcomes[1], None);
            assert!(seen.is_empty());
        } else {
            assert_eq!(seen, vec![Track::Video]);
        }
        drop(prefix);
        assert_eq!(before, f.bytes());
    }
}

#[test]
fn index_wrapper_rejects_corrupt_cancelled_and_foreign_source_before_sink() {
    for cause in [None, Some(SealCause::Cancelled)] {
        let f = authored_spool(index::authored_pair(), clock(), cause, false);
        if cause.is_none() {
            append_bytes(&f.dir().join(JOURNAL), b"{}\n");
        }
        let before = f.bytes();
        let result = f.index(&AtomicBool::new(false), &mut |_, _| {
            panic!("inadmissible source emitted")
        });
        assert!(matches!(result, Err(Error::Corrupt | Error::Cancelled)));
        assert_eq!(before, f.bytes());
    }
    let f = authored_spool(
        index::authored_pair(),
        clock(),
        Some(SealCause::Stopped),
        false,
    );
    let mut foreign = f.id.clone();
    foreign.document_epoch += 1;
    assert!(matches!(
        recovery::inspect_and_index_fixture(
            f.root.path(),
            &foreign,
            &AtomicBool::new(false),
            f.limits,
            &mut |_, _| panic!("foreign source emitted"),
        ),
        Err(Error::Proof)
    ));
    assert!(matches!(
        f.index(&AtomicBool::new(true), &mut |_, _| panic!(
            "cancelled source emitted"
        )),
        Err(Error::Cancelled)
    ));
}

#[test]
fn index_wrapper_never_promotes_interruption_or_reads_uncommitted_raw_tails() {
    let f = authored_spool(index::authored_pair(), clock(), None, true);
    let tails = [
        b"not a valid WebM tail".as_slice(),
        b"not a valid MP4 tail".as_slice(),
    ];
    append_bytes(&f.dir().join(RAW[0]), tails[0]);
    append_bytes(&f.dir().join(RAW[1]), tails[1]);
    append_bytes(&f.dir().join(JOURNAL), b"{");
    let before = f.bytes();
    let (prefix, outcomes) = f
        .index(&AtomicBool::new(false), &mut |_, _| Ok(()))
        .unwrap();
    assert_both_structurally_complete(outcomes);
    assert_eq!(prefix.state, PrefixState::Interrupted);
    assert_eq!(prefix.seal_cause, None);
    assert!(prefix.incomplete_append && prefix.torn_journal);
    assert_eq!(
        prefix.uncommitted_bytes,
        tails.map(|tail| tail.len() as u64)
    );
    drop(prefix);
    assert_eq!(before, f.bytes());
    for cause in [SealCause::Interrupted, SealCause::Limit] {
        let f = authored_spool(index::authored_pair(), clock(), Some(cause), false);
        let (prefix, outcomes) = f
            .index(&AtomicBool::new(false), &mut |_, _| Ok(()))
            .unwrap();
        assert_both_structurally_complete(outcomes);
        assert_eq!(prefix.state, PrefixState::Interrupted);
        assert_eq!(prefix.seal_cause, Some(cause));
    }
}

#[test]
fn index_wrapper_preserves_partial_tail_and_reports_unrun_audio_explicitly() {
    let mut pair = index::authored_pair();
    pair[0].extend([0x1a, 0x45]);
    let f = authored_spool(pair, clock(), Some(SealCause::Stopped), false);
    let before = f.bytes();
    let (prefix, outcomes) = f
        .index(&AtomicBool::new(false), &mut |_, _| Ok(()))
        .unwrap();
    assert!(matches!(
        outcomes[0],
        Some(index::Outcome {
            terminal: index::Terminal::PartialTail { .. },
            ..
        })
    ));
    assert_eq!(outcomes[1].unwrap().terminal, index::Terminal::Complete);
    drop(prefix);
    assert_eq!(before, f.bytes());

    for failed_track in [Track::Video, Track::Audio] {
        let mut pair = index::authored_pair();
        // A complete unsupported atom, not a four-byte truncated MP4 header.
        pair[failed_track.index()] = [8u32.to_be_bytes().as_slice(), b"zzzz"].concat();
        let f = authored_spool(pair, clock(), Some(SealCause::Stopped), false);
        let mut seen = Vec::new();
        let (prefix, outcomes) = f
            .index(&AtomicBool::new(false), &mut |track, _| {
                seen.push(track);
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            outcomes[failed_track.index()],
            Some(index::Outcome {
                terminal: index::Terminal::Failed(_),
                ..
            })
        ));
        if failed_track == Track::Video {
            assert_eq!(outcomes[1], None);
            assert!(seen.is_empty());
        } else {
            assert_eq!(seen, vec![Track::Video]);
        }
        drop(prefix);
    }
}

#[test]
fn index_wrapper_sink_failure_and_late_cancellation_never_return_joint_completion() {
    let f = authored_spool(
        index::authored_pair(),
        clock(),
        Some(SealCause::Stopped),
        false,
    );
    let before = f.bytes();
    let mut seen = Vec::new();
    let (prefix, outcomes) = f
        .index(&AtomicBool::new(false), &mut |track, _| {
            seen.push(track);
            Err(())
        })
        .unwrap();
    assert_eq!(seen, vec![Track::Video]);
    assert!(matches!(
        outcomes[0],
        Some(index::Outcome {
            terminal: index::Terminal::Failed(index::Error {
                kind: index::Failure::Sink,
                ..
            }),
            ..
        })
    ));
    assert_eq!(outcomes[1], None);
    drop(prefix);
    for stop_after in [Track::Video, Track::Audio] {
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let result = f.index(&cancel, &mut |track, _| {
            seen.push(track);
            if track == stop_after {
                cancel.store(true, Ordering::Release);
            }
            Ok(())
        });
        assert!(matches!(result, Err(Error::Cancelled)));
        assert_eq!(
            seen,
            if stop_after == Track::Video {
                vec![Track::Video]
            } else {
                vec![Track::Video, Track::Audio]
            }
        );
    }
    assert_eq!(before, f.bytes());
}

#[cfg(windows)]
#[test]
fn index_wrapper_requires_writer_drop_even_after_seal() {
    let f = authored_spool(
        index::authored_pair(),
        clock(),
        Some(SealCause::Stopped),
        false,
    );
    // A separate fixture deliberately keeps its original write handles alive.
    let mut live = Fixture::new();
    live.limits = f.limits;
    let mut writer = live.writer();
    let pair = index::authored_pair();
    for (track, bytes) in [(Track::Video, &pair[0]), (Track::Audio, &pair[1])] {
        writer
            .append(&live.id, chunk(track, 0, 0, true, bytes))
            .unwrap();
    }
    writer.seal(&live.id, SealCause::Stopped).unwrap();
    assert!(matches!(
        live.index(&AtomicBool::new(false), &mut |_, _| panic!(
            "writer still owns files"
        )),
        Err(Error::Path)
    ));
    drop(writer);
    let (_, outcomes) = live
        .index(&AtomicBool::new(false), &mut |_, _| Ok(()))
        .unwrap();
    assert_both_structurally_complete(outcomes);
}

#[test]
fn index_wrapper_rechecks_file_links_between_tracks_and_after_final_sink() {
    for change_after in [Track::Video, Track::Audio] {
        let f = authored_spool(
            index::authored_pair(),
            clock(),
            Some(SealCause::Stopped),
            false,
        );
        let before = f.bytes();
        let link = f.root.path().join("late-link.raw");
        let mut linked = false;
        let mut seen = Vec::new();
        let result = f.index(&AtomicBool::new(false), &mut |track, _| {
            seen.push(track);
            if track == change_after {
                // Some hosts deny this metadata mutation while the file is held.
                // If admitted, the immediately following guard must reject it.
                linked = fs::hard_link(f.dir().join(RAW[0]), &link).is_ok();
                if !linked {
                    assert!(!link.exists());
                }
            }
            Ok(())
        });
        if linked {
            assert!(matches!(result, Err(Error::Path)));
            assert_eq!(
                seen,
                if change_after == Track::Video {
                    vec![Track::Video]
                } else {
                    vec![Track::Video, Track::Audio]
                }
            );
        } else {
            let (prefix, outcomes) = result.unwrap();
            assert_both_structurally_complete(outcomes);
            drop(prefix);
        }
        assert_eq!(before, f.bytes());
    }
}

#[test]
fn selected_unit_binds_exact_ordinal_range_source_and_one_held_cursor() {
    use std::io::{Read, Seek, SeekFrom};

    let pair = index::authored_pair();
    let doubled = pair
        .clone()
        .map(|bytes| [bytes.as_slice(), bytes.as_slice()].concat());
    let f = authored_spool(doubled, clock(), Some(SealCause::Stopped), false);
    let before = f.bytes();
    let cancel = AtomicBool::new(false);
    for track in [Track::Video, Track::Audio] {
        for ordinal in [0, 1] {
            let expected = &pair[track.index()];
            let mut selected = f.select(&cancel, track, ordinal).unwrap();
            let read = selected
                .with_span(&cancel, |mut span| {
                    let facts = span.facts();
                    assert_eq!(facts.identity, &f.id);
                    assert_eq!(facts.clock, &clock());
                    assert_eq!(facts.track, track);
                    assert_eq!(facts.receipts, 2);
                    assert_eq!(facts.state, PrefixState::Sealed);
                    assert_eq!(facts.seal_cause, Some(SealCause::Stopped));
                    assert!(!facts.incomplete_append && !facts.torn_journal);
                    assert_eq!(facts.uncommitted_bytes, [0, 0]);
                    assert_eq!(facts.unit.ordinal, ordinal);
                    assert!(facts.unit.codec_packet_validation_pending);
                    assert_eq!(facts.tracks[track.index()].bytes, 2 * expected.len() as u64);
                    let start = facts.unit.facts.start;
                    let end = facts.unit.facts.end;
                    assert_eq!(start, ordinal * expected.len() as u64);
                    assert_eq!(end, start + expected.len() as u64);
                    assert!(start < facts.unit.facts.init_end && facts.unit.facts.init_end < end);

                    let mut reader = span.take_reader().unwrap();
                    assert!(matches!(span.take_reader(), Err(Error::Order)));
                    reader.seek(SeekFrom::Start(start)).unwrap();
                    let at = reader.stream_position().unwrap();
                    span.check().unwrap();
                    assert_eq!(reader.stream_position().unwrap(), at);
                    let mut bytes = vec![0; (end - start) as usize];
                    reader.read_exact(&mut bytes).unwrap();
                    span.check().unwrap();
                    assert_eq!(reader.stream_position().unwrap(), end);
                    bytes
                })
                .unwrap();
            assert_eq!(&read, expected);
            // A later synchronous operation is allowed, never a second live lease.
            assert_eq!(
                selected.with_span(&cancel, |span| span.facts().unit.ordinal),
                Ok(ordinal)
            );
            drop(selected);
        }
    }
    assert_eq!(before, f.bytes());
}

#[test]
fn selected_unit_rejects_absent_ordinal_foreign_identity_corruption_and_cancel() {
    let f = authored_spool(
        index::authored_pair(),
        clock(),
        Some(SealCause::Stopped),
        false,
    );
    let before = f.bytes();
    for ordinal in [1, 100_000, u64::MAX] {
        assert!(matches!(
            f.select(&AtomicBool::new(false), Track::Video, ordinal),
            Err(Error::Bounds)
        ));
    }
    let mut foreign = f.id.clone();
    foreign.source_generation += 1;
    assert!(matches!(
        recovery::select_unit(
            f.root.path(),
            &foreign,
            &AtomicBool::new(false),
            Track::Video,
            0,
        ),
        Err(Error::Proof)
    ));
    assert!(matches!(
        f.select(&AtomicBool::new(true), Track::Video, 0),
        Err(Error::Cancelled)
    ));
    assert_eq!(before, f.bytes());
    for cause in [None, Some(SealCause::Cancelled)] {
        let f = authored_spool(index::authored_pair(), clock(), cause, false);
        if cause.is_none() {
            append_bytes(&f.dir().join(JOURNAL), b"{}\n");
        }
        let before = f.bytes();
        assert!(matches!(
            f.select(&AtomicBool::new(false), Track::Video, 0),
            Err(Error::Corrupt | Error::Cancelled)
        ));
        assert_eq!(before, f.bytes());
    }
}

#[test]
fn selected_draft_never_escapes_opposite_failure_partial_tail_or_clock_mismatch() {
    for failed_track in [Track::Video, Track::Audio] {
        for tail in [false, true] {
            let mut pair = index::authored_pair();
            if tail {
                // Preserve a complete emitted unit, then leave its next header partial.
                pair[failed_track.index()].extend(if failed_track == Track::Video {
                    [0x1a, 0x45]
                } else {
                    [0, 0]
                });
            } else {
                pair[failed_track.index()] = [8u32.to_be_bytes().as_slice(), b"zzzz"].concat();
            }
            let f = authored_spool(pair, clock(), Some(SealCause::Stopped), false);
            let before = f.bytes();
            for selected_track in [Track::Video, Track::Audio] {
                assert!(matches!(
                    f.select(&AtomicBool::new(false), selected_track, 0),
                    Err(Error::Proof)
                ));
            }
            assert_eq!(before, f.bytes());
        }
        let mut descriptor = clock();
        if failed_track == Track::Video {
            descriptor.video_timestamp_scale_ns = 2_000_000;
        } else {
            descriptor.audio_timescale = 48000;
        }
        let f = authored_spool(
            index::authored_pair(),
            descriptor,
            Some(SealCause::Stopped),
            false,
        );
        assert!(matches!(
            f.select(&AtomicBool::new(false), Track::Video, 0),
            Err(Error::Proof)
        ));
    }
}

#[test]
fn selected_unit_preserves_interruption_flags_and_excludes_uncommitted_tails() {
    use std::io::{Read, Seek, SeekFrom};

    let pair = index::authored_pair();
    let f = authored_spool(pair.clone(), clock(), None, true);
    let tails = [
        b"uncommitted video".as_slice(),
        b"uncommitted audio".as_slice(),
    ];
    for track in [Track::Video, Track::Audio] {
        append_bytes(&f.dir().join(RAW[track.index()]), tails[track.index()]);
    }
    append_bytes(&f.dir().join(JOURNAL), b"{");
    let before = f.bytes();
    let cancel = AtomicBool::new(false);
    for track in [Track::Video, Track::Audio] {
        let mut selected = f.select(&cancel, track, 0).unwrap();
        let bytes = selected
            .with_span(&cancel, |mut span| {
                let facts = span.facts();
                assert_eq!(facts.state, PrefixState::Interrupted);
                assert_eq!(facts.seal_cause, None);
                assert!(facts.incomplete_append && facts.torn_journal);
                assert_eq!(facts.uncommitted_bytes, tails.map(|tail| tail.len() as u64));
                let start = facts.unit.facts.start;
                let length = facts.unit.facts.end - start;
                assert_eq!(facts.unit.facts.end, facts.tracks[track.index()].bytes);
                let mut reader = span.take_reader().unwrap();
                reader.seek(SeekFrom::Start(start)).unwrap();
                let mut bytes = vec![0; length as usize];
                reader.read_exact(&mut bytes).unwrap();
                bytes
            })
            .unwrap();
        assert_eq!(bytes, pair[track.index()]);
        drop(selected);
    }
    assert_eq!(before, f.bytes());
    for cause in [SealCause::Interrupted, SealCause::Limit] {
        let f = authored_spool(index::authored_pair(), clock(), Some(cause), false);
        let mut selected = f.select(&cancel, Track::Video, 0).unwrap();
        selected
            .with_span(&cancel, |span| {
                assert_eq!(span.facts().state, PrefixState::Interrupted);
                assert_eq!(span.facts().seal_cause, Some(cause));
            })
            .unwrap();
    }
}

#[test]
fn selected_span_wraps_work_result_with_final_cancellation_checks() {
    let f = authored_spool(
        index::authored_pair(),
        clock(),
        Some(SealCause::Stopped),
        false,
    );
    let before = f.bytes();
    let cancel = AtomicBool::new(false);
    let mut selected = f.select(&cancel, Track::Video, 0).unwrap();
    let already_cancelled = AtomicBool::new(true);
    assert!(matches!(
        selected.with_span(&already_cancelled, |_| panic!("cancelled work ran")),
        Err(Error::Cancelled)
    ));
    assert_eq!(
        selected.with_span(&cancel, |_| Err::<(), _>(Error::Io)),
        Ok(Err(Error::Io))
    );
    assert!(matches!(
        selected.with_span(&cancel, |span| {
            span.check().unwrap();
            cancel.store(true, Ordering::Release);
            assert_eq!(span.check(), Err(Error::Cancelled));
            Ok::<(), Error>(())
        }),
        Err(Error::Cancelled)
    ));
    drop(selected);

    let cancel = AtomicBool::new(false);
    let mut selected = f.select(&cancel, Track::Audio, 0).unwrap();
    assert!(matches!(
        selected.with_span(&cancel, |_| {
            // Work has completed; the outer guard must still supersede its result.
            cancel.store(true, Ordering::Release);
            Err::<(), _>(Error::Io)
        }),
        Err(Error::Cancelled)
    ));
    drop(selected);
    assert_eq!(before, f.bytes());
}

#[test]
fn selected_span_rechecks_held_file_links_after_work() {
    let f = authored_spool(
        index::authored_pair(),
        clock(),
        Some(SealCause::Stopped),
        false,
    );
    let before = f.bytes();
    let cancel = AtomicBool::new(false);
    let mut selected = f.select(&cancel, Track::Video, 0).unwrap();
    let link = f.root.path().join("span-late-link.raw");
    let mut linked = false;
    let result = selected.with_span(&cancel, |_| {
        linked = fs::hard_link(f.dir().join(RAW[0]), &link).is_ok();
        if !linked {
            assert!(!link.exists());
        }
        7
    });
    if linked {
        assert_eq!(result, Err(Error::Path));
    } else {
        assert_eq!(result, Ok(7));
    }
    drop(selected);
    assert_eq!(before, f.bytes());
}

#[test]
fn sequential_visits_are_ordered_repeatable_and_restore_the_one_held_cursor() {
    use std::io::{Read, Seek, SeekFrom};

    let pair = index::authored_pair();
    let repeated = pair.clone().map(|bytes| [bytes.as_slice(); 3].concat());
    let f = authored_spool(repeated, clock(), Some(SealCause::Stopped), false);
    let before = f.bytes();
    let cancel = AtomicBool::new(false);
    let mut source = f.sequential(&cancel).unwrap();
    source.check(&cancel).unwrap();
    for track in [Track::Video, Track::Audio] {
        let mut prior = None;
        let mut address = None;
        for _ in 0..2 {
            let mut seen = Vec::new();
            let mut restored = None;
            source
                .visit(track, &cancel, |mut span| {
                    let facts = span.facts();
                    assert_eq!(facts.identity, &f.id);
                    assert_eq!(facts.clock, &clock());
                    assert_eq!(facts.track, track);
                    assert_eq!(facts.state, PrefixState::Sealed);
                    assert!(facts.unit.codec_packet_validation_pending);
                    // Repeated authored clocks overlap: this is observation, not
                    // an accidental new clock/codec/relocation acceptance gate.
                    if facts.unit.ordinal > 0 {
                        assert_eq!(
                            facts.unit.clock_relation,
                            index::ClockRelation::OverlapOrReset
                        );
                    }
                    let unit = facts.unit.clone();
                    let start = unit.facts.start;
                    let end = unit.facts.end;
                    assert_eq!(start, unit.ordinal * pair[track.index()].len() as u64);
                    let mut reader = span.take_reader()?;
                    assert!(matches!(span.take_reader(), Err(Error::Order)));
                    let current_address = reader as *const std::fs::File as usize;
                    if let Some(previous) = address {
                        assert_eq!(current_address, previous);
                    }
                    address = Some(current_address);
                    restored = Some(reader.stream_position().unwrap());
                    reader.seek(SeekFrom::Start(start)).unwrap();
                    let mut bytes = vec![0; (end - start) as usize];
                    reader.read_exact(&mut bytes).unwrap();
                    assert_eq!(bytes, pair[track.index()]);
                    reader.seek(SeekFrom::Start(0)).unwrap();
                    span.check()?;
                    assert_eq!(reader.stream_position().unwrap(), 0);
                    seen.push(unit);
                    Ok(())
                })
                .unwrap();
            assert_eq!(
                seen.iter().map(|u| u.ordinal).collect::<Vec<_>>(),
                [0, 1, 2]
            );
            assert_eq!(source.cursor_fixture(track).unwrap(), restored.unwrap());
            if let Some(previous) = &prior {
                assert_eq!(&seen, previous);
            }
            prior = Some(seen);
        }
    }
    drop(source);
    assert_eq!(before, f.bytes());
}

#[test]
fn sequential_preflight_rejects_either_failed_or_partial_track_before_any_work() {
    for track in [Track::Video, Track::Audio] {
        for partial in [false, true] {
            let mut pair = index::authored_pair();
            if partial {
                pair[track.index()].extend(if track == Track::Video {
                    [0x1a, 0x45]
                } else {
                    [0, 0]
                });
            } else {
                pair[track.index()] = [8u32.to_be_bytes().as_slice(), b"zzzz"].concat();
            }
            let f = authored_spool(pair, clock(), Some(SealCause::Stopped), false);
            let before = f.bytes();
            let result = f.sequential(&AtomicBool::new(false));
            assert!(matches!(result, Err(Error::Proof)));
            // No work callback is accepted until a private source exists.
            assert_eq!(before, f.bytes());
        }
        let mut descriptor = clock();
        if track == Track::Video {
            descriptor.video_timestamp_scale_ns = 2_000_000;
        } else {
            descriptor.audio_timescale = 48000;
        }
        let f = authored_spool(
            index::authored_pair(),
            descriptor,
            Some(SealCause::Stopped),
            false,
        );
        assert!(matches!(
            f.sequential(&AtomicBool::new(false)),
            Err(Error::Proof)
        ));
    }
    for cause in [None, Some(SealCause::Cancelled)] {
        let f = authored_spool(index::authored_pair(), clock(), cause, false);
        if cause.is_none() {
            append_bytes(&f.dir().join(JOURNAL), b"{}\n");
        }
        assert!(matches!(
            f.sequential(&AtomicBool::new(false)),
            Err(Error::Corrupt | Error::Cancelled)
        ));
    }
    let f = authored_spool(
        index::authored_pair(),
        clock(),
        Some(SealCause::Stopped),
        false,
    );
    let mut foreign = f.id.clone();
    foreign.document_epoch += 1;
    assert!(matches!(
        recovery::SequentialSource::prepare(f.root.path(), &foreign, &AtomicBool::new(false)),
        Err(Error::Proof)
    ));
    assert!(matches!(
        f.sequential(&AtomicBool::new(true)),
        Err(Error::Cancelled)
    ));
}

#[test]
fn sequential_callback_failure_cancel_and_unwind_are_terminal_not_eof() {
    use std::io::{Seek, SeekFrom};
    let repeated = index::authored_pair().map(|bytes| [bytes.as_slice(); 3].concat());
    let f = authored_spool(repeated, clock(), Some(SealCause::Stopped), false);
    let before = f.bytes();
    let cancelled = AtomicBool::new(false);
    let mut never_started = f.sequential(&cancelled).unwrap();
    cancelled.store(true, Ordering::Release);
    assert_eq!(
        never_started.visit(Track::Video, &cancelled, |_| panic!("cancelled work began")),
        Err(Error::Cancelled)
    );
    drop(never_started);
    // Cancel at either the first or the final emitted unit; successful earlier
    // callbacks must never turn the whole visit into success.
    for target in [0, 2] {
        let cancel = AtomicBool::new(false);
        let mut source = f.sequential(&cancel).unwrap();
        let mut seen = 0;
        let mut position = 0;
        assert_eq!(
            source.visit(Track::Video, &cancel, |mut span| {
                let ordinal = span.facts().unit.ordinal;
                let mut file = span.take_reader()?;
                position = file.stream_position().unwrap();
                file.seek(SeekFrom::Start(0)).unwrap();
                seen += 1;
                if ordinal == target {
                    cancel.store(true, Ordering::Release);
                }
                Ok(())
            }),
            Err(Error::Cancelled)
        );
        assert_eq!(seen, target + 1);
        assert_eq!(source.cursor_fixture(Track::Video).unwrap(), position);
        cancel.store(false, Ordering::Release);
        assert_eq!(source.check(&cancel), Err(Error::Cancelled));
        assert_eq!(
            source.visit(Track::Audio, &cancel, |_| panic!(
                "cancelled context reused"
            )),
            Err(Error::Cancelled)
        );
    }
    let cancel = AtomicBool::new(false);
    let mut source = f.sequential(&cancel).unwrap();
    let mut calls = 0;
    let mut position = 0;
    assert_eq!(
        source.visit(Track::Audio, &cancel, |mut span| {
            let mut file = span.take_reader()?;
            position = file.stream_position().unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            calls += 1;
            Err(Error::Io)
        }),
        Err(Error::Io)
    );
    assert_eq!(calls, 1);
    assert_eq!(source.check(&cancel), Err(Error::Io));
    assert_eq!(source.cursor_fixture(Track::Audio).unwrap(), position);
    assert_eq!(
        source.visit(Track::Video, &cancel, |_| panic!("failed context reused")),
        Err(Error::Io)
    );
    drop(source);

    let mut source = f.sequential(&cancel).unwrap();
    let mut position = 0;
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = source.visit(Track::Video, &cancel, |mut span| {
            let mut file = span.take_reader().unwrap();
            position = file.stream_position().unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            panic!("authored synchronous callback panic")
        });
    }));
    assert!(panic.is_err());
    assert_eq!(source.check(&cancel), Err(Error::Order));
    assert_eq!(source.cursor_fixture(Track::Video).unwrap(), position);
    assert_eq!(
        source.visit(Track::Audio, &cancel, |_| panic!("unwound context reused")),
        Err(Error::Order)
    );
    drop(source);
    assert_eq!(before, f.bytes());
}

#[test]
fn sequential_interrupted_prefix_keeps_all_inherited_flags_and_committed_ranges() {
    let pair = index::authored_pair();
    let f = authored_spool(pair.clone(), clock(), None, true);
    let tails = [
        b"uncommitted video".as_slice(),
        b"uncommitted audio".as_slice(),
    ];
    for track in [Track::Video, Track::Audio] {
        append_bytes(&f.dir().join(RAW[track.index()]), tails[track.index()]);
    }
    append_bytes(&f.dir().join(JOURNAL), b"{");
    let before = f.bytes();
    let cancel = AtomicBool::new(false);
    let mut source = f.sequential(&cancel).unwrap();
    for track in [Track::Video, Track::Audio] {
        source
            .visit(track, &cancel, |span| {
                let facts = span.facts();
                assert_eq!(facts.state, PrefixState::Interrupted);
                assert_eq!(facts.seal_cause, None);
                assert!(facts.incomplete_append && facts.torn_journal);
                assert_eq!(facts.uncommitted_bytes, tails.map(|tail| tail.len() as u64));
                assert_eq!(facts.unit.facts.end, pair[track.index()].len() as u64);
                assert_eq!(facts.tracks[track.index()].bytes, facts.unit.facts.end);
                assert!(facts.unit.codec_packet_validation_pending);
                Ok(())
            })
            .unwrap();
    }
    drop(source);
    assert_eq!(before, f.bytes());
    for cause in [SealCause::Interrupted, SealCause::Limit] {
        let f = authored_spool(index::authored_pair(), clock(), Some(cause), false);
        f.sequential(&cancel)
            .unwrap()
            .visit(Track::Video, &cancel, |span| {
                assert_eq!(span.facts().state, PrefixState::Interrupted);
                assert_eq!(span.facts().seal_cause, Some(cause));
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn sequential_checks_all_held_links_after_final_callback_and_latches_revocation() {
    for name in [PROOF, JOURNAL, RAW[0], RAW[1]] {
        let f = authored_spool(
            index::authored_pair(),
            clock(),
            Some(SealCause::Stopped),
            false,
        );
        let before = f.bytes();
        let cancel = AtomicBool::new(false);
        let mut source = f.sequential(&cancel).unwrap();
        let link = f.root.path().join("sequential-late-link.raw");
        let mut linked = false;
        let result = source.visit(Track::Video, &cancel, |_| {
            linked = fs::hard_link(f.dir().join(name), &link).is_ok();
            if !linked {
                assert!(!link.exists());
            }
            Ok(())
        });
        if linked {
            assert_eq!(result, Err(Error::Path));
            assert_eq!(
                source.visit(Track::Audio, &cancel, |_| panic!("revoked source reused")),
                Err(Error::Path)
            );
        } else {
            assert_eq!(result, Ok(()));
        }
        drop(source);
        assert_eq!(before, f.bytes());
    }
}

#[test]
fn sequential_rejects_length_changes_or_os_denies_them_without_cursor_mutation() {
    for name in [PROOF, JOURNAL, RAW[0], RAW[1]] {
        let f = authored_spool(
            index::authored_pair(),
            clock(),
            Some(SealCause::Stopped),
            false,
        );
        let cancel = AtomicBool::new(false);
        let mut source = f.sequential(&cancel).unwrap();
        let mut changed = false;
        let result = source.visit(Track::Video, &cancel, |_| {
            if let Ok(mut file) = fs::OpenOptions::new().append(true).open(f.dir().join(name)) {
                file.write_all(b"x").unwrap();
                file.sync_all().unwrap();
                changed = true;
            }
            Ok(())
        });
        if changed {
            assert_eq!(result, Err(Error::Corrupt));
            assert_eq!(
                source.visit(Track::Audio, &cancel, |_| panic!("changed source reused")),
                Err(Error::Corrupt)
            );
        } else {
            assert_eq!(result, Ok(()));
        }
    }
}

#[test]
fn sequential_fixed_observation_digest_binds_all_fact_categories_and_order() {
    let f = authored_spool(
        index::authored_pair(),
        clock(),
        Some(SealCause::Stopped),
        false,
    );
    let mut units = Vec::new();
    let (prefix, outcomes) = f
        .index(&AtomicBool::new(false), &mut |track, unit| {
            units.push((track, unit));
            Ok(())
        })
        .unwrap();
    assert_both_structurally_complete(outcomes);
    drop(prefix);
    for (track, unit) in units {
        let base = recovery::observation_fixture(track, std::slice::from_ref(&unit)).unwrap();
        assert_eq!(
            base,
            recovery::observation_fixture(track, std::slice::from_ref(&unit)).unwrap()
        );
        let other = if track == Track::Video {
            Track::Audio
        } else {
            Track::Video
        };
        assert_ne!(
            base,
            recovery::observation_fixture(other, std::slice::from_ref(&unit)).unwrap()
        );
        let mut mutations = Vec::new();
        macro_rules! changed {
            ($field:ident $(.$tail:ident)*, $value:expr) => {{
                let mut changed = unit.clone();
                changed.$field$(.$tail)* = $value;
                mutations.push(changed);
            }};
        }
        changed!(facts.start, unit.facts.start + 1);
        changed!(facts.init_end, unit.facts.init_end + 1);
        changed!(facts.end, unit.facts.end + 1);
        changed!(facts.init_sha256, [17; 32]);
        changed!(configuration_sha256, [23; 32]);
        changed!(facts.configuration.semantic_sha256, [29; 32]);
        changed!(
            facts.configuration.timebase,
            unit.facts.configuration.timebase + 1
        );
        changed!(facts.configuration.default_duration_ns, Some(123));
        changed!(facts.clock.first, unit.facts.clock.first - 1);
        changed!(facts.clock.last, unit.facts.clock.last + 1);
        changed!(facts.clock.end, unit.facts.clock.end + 1);
        changed!(
            facts.clock.ticks_per_second,
            unit.facts.clock.ticks_per_second + 1
        );
        changed!(facts.items, unit.facts.items + 1);
        changed!(facts.fragments, unit.facts.fragments + 1);
        changed!(facts.references, unit.facts.references + 1);
        changed!(facts.rap_candidate, !unit.facts.rap_candidate);
        changed!(facts.boundary, index::Boundary::NextInitialization);
        changed!(init_relation, index::InitRelation::UnprovedInitDifference);
        changed!(clock_relation, index::ClockRelation::Gap);
        changed!(
            codec_packet_validation_pending,
            !unit.codec_packet_validation_pending
        );
        match unit.facts.reconstruction {
            index::ReconstructionFacts::Webm(value) => {
                macro_rules! webm {
                    ($field:ident) => {{
                        let mut reconstruction = value;
                        reconstruction.$field = !reconstruction.$field;
                        changed!(
                            facts.reconstruction,
                            index::ReconstructionFacts::Webm(reconstruction)
                        );
                    }};
                }
                webm!(unknown_segment);
                webm!(segment_crc);
                webm!(info_duration);
                webm!(cluster_position);
                webm!(cluster_prev_size);
                webm!(unknown_cluster_crc);
                webm!(finite_cluster_crc);
            }
            index::ReconstructionFacts::Aac(value) => {
                let mut reconstruction = value;
                reconstruction.all_fragments_default_base_is_moof =
                    !value.all_fragments_default_base_is_moof;
                changed!(
                    facts.reconstruction,
                    index::ReconstructionFacts::Aac(reconstruction)
                );
                reconstruction = value;
                reconstruction.first_sequence += 1;
                changed!(
                    facts.reconstruction,
                    index::ReconstructionFacts::Aac(reconstruction)
                );
                reconstruction = value;
                reconstruction.last_sequence += 1;
                changed!(
                    facts.reconstruction,
                    index::ReconstructionFacts::Aac(reconstruction)
                );
                for duration in [
                    index::HeaderDuration::UnknownSentinel,
                    index::HeaderDuration::Finite(7),
                ] {
                    for field in 0..3 {
                        reconstruction = value;
                        match field {
                            0 => reconstruction.movie_duration = duration,
                            1 => reconstruction.track_duration = duration,
                            _ => reconstruction.media_duration = duration,
                        }
                        changed!(
                            facts.reconstruction,
                            index::ReconstructionFacts::Aac(reconstruction)
                        );
                    }
                }
            }
        }
        for changed in mutations {
            assert_ne!(
                base,
                recovery::observation_fixture(track, &[changed]).unwrap()
            );
        }
        assert_eq!(
            recovery::observation_fixture(track, &[unit.clone(), unit.clone()]),
            Err(Error::Proof)
        );
        let mut second = unit.clone();
        second.ordinal = 1;
        assert_ne!(
            base,
            recovery::observation_fixture(track, &[unit, second.clone()]).unwrap()
        );
        assert_eq!(
            recovery::observation_fixture(track, &[second]),
            Err(Error::Proof)
        );
    }
}
