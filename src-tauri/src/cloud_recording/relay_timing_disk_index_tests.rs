use super::*;
use serde_json::{json, Value};
use std::io::{BufWriter, Write};

fn owner() -> ExpectedOwner {
    ExpectedOwner::Soop(SoopOwner { version: 1, kind: "soop-relay-chat".into(), sidecar_id: Uuid::new_v4().to_string(),
        binding: SoopBinding { receiver_id: Uuid::new_v4().to_string(), run_id: Uuid::new_v4().to_string(),
            source_id: Uuid::new_v4().to_string(), document_epoch: 1, channel_id: "fixture".into(), broadcast_id: "123".into() },
        timeline_quality: "receive-monotonic-only".into(), media_job: false })
}
fn row(owner: &ExpectedOwner, sequence: u64, kind: &str, data: Value) -> Value {
    let ExpectedOwner::Soop(owner) = owner else { unreachable!() };
    json!({"version":1,"sequence":sequence,"kind":kind,"binding":owner.binding,"data":data})
}
fn event(id: u64, sequence: u64, usable: bool, service: u16) -> Value {
    let seconds = sequence as f64 / 10.;
    json!({"messageIdSha256":format!("{:064x}",id+1),"service":service,"observationSequence":sequence,
        "receivedMonotonicNs":sequence,"sample":{"sourceTimeSeconds":seconds,"currentTimeSeconds":seconds,
        "timestampOffsetSeconds":0.,"performanceNowMs":sequence as f64,"playbackRate":1.,
        "paused":!usable,"seeking":false,"readyState":4},"liveVisible":usable,"usable":usable,
        "quality":"public-receiver-dispatch","presentationUncertainty":"unknown","archiveMapping":null})
}
fn track(start: u64, end: u64, scale: u32, samples: u64) -> Value {
    json!({"timescale":scale,"firstDts":start,"lastDtsEnd":end,"minPts":start,"maxPtsEnd":end,"samples":samples,
        "firstDts90k":start*90_000/u64::from(scale),"lastDtsEnd90k":end*90_000/u64::from(scale),
        "minPts90k":start*90_000/u64::from(scale),"maxPtsEnd90k":end*90_000/u64::from(scale)})
}
fn segment(stream: Uuid, ordinal: u64) -> Value {
    json!({"streamId":stream,"ordinal":ordinal,"nativeBeforeNs":ordinal*10,"nativeAfterNs":ordinal*10+1,
        "bytes":188,"sha256":format!("{:064x}",ordinal+1),"archiveMapping":null,
        "timing":{"originTicks":0,"originTimescale":90_000,"segmentStart90k":ordinal*180_000,"segmentEnd90k":(ordinal+1)*180_000,
            "video":track(ordinal*180_000,(ordinal+1)*180_000,90_000,60),
            "audio":track(ordinal*96_000,(ordinal+1)*96_000,48_000,94)}})
}
fn end(stream: Uuid, count: u64) -> Value {
    json!({"streamId":stream,"segments":count,"nativeBeforeNs":count*10,"nativeAfterNs":count*10+1,
        "origin":{"ticks":0,"timescale":90_000,"videoBase":0,"audioBase":0,"videoRemainder":0,"audioRemainder":0},
        "preroll":{"videoSamples":0,"audioSamples":0,"bytes":0},"archiveMapping":null})
}
struct Fixture { file: tempfile::NamedTempFile, owner: ExpectedOwner, durable: Durable, native_acks: u64, native_end: bool, summary: RelayTimingSummary }
impl Fixture {
    fn new(owner: ExpectedOwner, records: u64, native_acks: u64, native_end: bool, write: impl FnOnce(&mut BufWriter<&mut File>)) -> Self {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        { let mut writer = BufWriter::new(file.as_file_mut()); write(&mut writer); writer.flush().unwrap(); }
        let durable = Durable { records, incomplete: false, sidecar_incomplete: false };
        let mut reader = BufReader::new(File::open(file.path()).unwrap());
        let (summary, _, _) = scan(&mut reader, &owner, durable, native_acks, native_end, &AtomicBool::new(false)).unwrap();
        Self { file, owner, durable, native_acks, native_end, summary }
    }
    fn build(&self, cancel: &AtomicBool, pages: u32) -> Result<DiskIndex> {
        DiskIndex::build(&mut BufReader::new(File::open(self.file.path()).unwrap()), &self.owner,
            self.durable, self.native_acks, self.native_end, &self.summary, cancel, pages)
    }
}
fn write_row(writer: &mut BufWriter<&mut File>, row: Value) { serde_json::to_writer(&mut *writer, &row).unwrap(); writer.write_all(b"\n").unwrap(); }

#[test]
fn all_65537_event_identities_survive_and_late_other_service_duplicate_is_ambiguous() {
    let owner = owner();
    let fixture = Fixture::new(owner.clone(), 65_538, 0, false, |writer| {
        for i in 0..65_537 { write_row(writer, row(&owner, i, "soop_event_clock", event(i, i, true, 5))); }
        write_row(writer, row(&owner, 65_537, "soop_event_clock", event(0, 65_537, false, 121)));
    });
    let before = fs::metadata(fixture.file.path()).unwrap().len();
    let cancel = AtomicBool::new(false);
    let index = fixture.build(&cancel, MAX_PAGES).unwrap();
    let ids = [format!("{:064x}",1), format!("{:064x}",32_769), format!("{:064x}",65_537)];
    let found = index.events(&ids.iter().map(|id|Some(id.as_str())).collect::<Vec<_>>(), &cancel).unwrap();
    assert_eq!(found[0], EventPosition::Unknown("ambiguous_event_source_clock"));
    assert_eq!(found[1], EventPosition::Source { ticks90k: 32_768*9000, service:5, observation_sequence:32_768 });
    assert_eq!(found[2], EventPosition::Source { ticks90k: 65_536*9000, service:5, observation_sequence:65_536 });
    assert_eq!(index.events(&[None,Some("bad"),Some(&"ff".repeat(32))], &cancel).unwrap(),
        vec![EventPosition::Unknown("missing_event_identity"),EventPosition::Unknown("invalid_event_identity"),EventPosition::Unknown("missing_event_source_clock")]);
    assert_eq!(before, fs::metadata(fixture.file.path()).unwrap().len());
}

#[test]
fn all_65537_ack_ranges_support_reverse_queries_and_exact_native_provenance() {
    let owner = owner(); let stream = Uuid::new_v4(); let count = 65_537u64;
    let fixture = Fixture::new(owner.clone(), count+1, count, true, |writer| {
        for i in 0..count { write_row(writer,row(&owner,i,"segment_ack",segment(stream,i))); }
        write_row(writer,row(&owner,count,"end_ack",end(stream,count)));
    });
    let cancel=AtomicBool::new(false); let index=fixture.build(&cancel,MAX_PAGES).unwrap();
    let ordinals=[65_536,32_768,0]; let source=ordinals.map(|i|i*180_000+90_000);
    let found=index.points(&fixture.summary,&source,&cancel).unwrap();
    for (point,ordinal) in found.into_iter().zip(ordinals) {
        let point=point.unwrap(); assert_eq!(point.ordinal,ordinal);
        assert_eq!(point.transport_ticks90k,ordinal*180_000+90_000);
        assert_eq!(point.segment_sha256,format!("{:064x}",ordinal+1));
    }
    assert!(index.points(&fixture.summary,&[0,180_000,count*180_000],&cancel).unwrap().iter().all(Option::is_none));
}

#[test]
fn gapped_native_three_ack_authority_is_not_reconstructed_as_two_ack_records() {
    let owner=owner();let stream=Uuid::new_v4();
    let fixture=Fixture::new(owner.clone(),3,3,true,|writer| {
        write_row(writer,row(&owner,0,"segment_ack",segment(stream,0)));
        write_row(writer,row(&owner,1,"segment_ack",segment(stream,2)));
        write_row(writer,row(&owner,2,"end_ack",end(stream,3)));
    });
    assert_eq!(fixture.summary.acknowledged_segments,2);assert_eq!(fixture.summary.prefix_segments,1);
    let cancel=AtomicBool::new(false);let index=fixture.build(&cancel,MAX_PAGES).unwrap();
    let found=index.points(&fixture.summary,&[90_000,270_000,450_000],&cancel).unwrap();
    assert_eq!(found[0].as_ref().unwrap().ordinal,0);assert!(found[1].is_none()&&found[2].is_none());
}

#[test]
fn all_u64_blob_keys_roundtrip_and_sort_across_signed_boundary() {
    let cancel=AtomicBool::new(false);let index=DiskIndex::create(MAX_PAGES,&cancel).unwrap();
    let db=lock(&index.connection,&cancel).unwrap();
    let values=[0,i64::MAX as u64,i64::MAX as u64+1,u64::MAX];
    for value in values {
        let range=points::VideoAckRange {source_min:value,source_end:u64::MAX,transport_min:value,transport_end:u64::MAX,sha256:[0xab;32]};
        DiskIndex::insert_ack(&db,value,&range,&cancel).unwrap();
    }
    let encoded=sql(&db,&cancel,|db| {
        let mut stmt=db.prepare("SELECT transport_min,ordinal,source_min,source_end FROM acks ORDER BY transport_min")?;
        let rows=stmt.query_map([],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,Vec<u8>>(3)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    }).unwrap();
    for ((key,ordinal,min,end),value) in encoded.into_iter().zip(values) {
        assert_eq!(number(key).unwrap(),value);assert_eq!(number(ordinal).unwrap(),value);
        assert_eq!(number(min).unwrap(),value);assert_eq!(number(end).unwrap(),u64::MAX);
    }
}

#[test]
fn full_index_fails_without_partial_publication_and_same_source_retries_cleanly() {
    let owner=owner();let fixture=Fixture::new(owner.clone(),1024,0,false,|writer| {
        for i in 0..1024 {write_row(writer,row(&owner,i,"soop_event_clock",event(i,i,true,5)));}
    });
    let cancel=AtomicBool::new(false);
    assert_eq!(fixture.build(&cancel,3).err(),Some(TimingReadError::Limit));
    // Also hit FULL after at least one complete 256-row transaction. A failed
    // private derivative can contain committed pages without becoming a lease capability.
    let partial=DiskIndex::create(12,&cancel).unwrap();let partial_path=partial._directory.path().to_owned();
    {
        let db=lock(&partial.connection,&cancel).unwrap();let mut committed=0;let mut full=false;
        sql(&db,&cancel,|db|db.execute_batch("BEGIN IMMEDIATE")).unwrap();
        for i in 0..1024 {
            let event=serde_json::from_value(event(i,i,true,5)).unwrap();
            if let Err(error)=DiskIndex::insert_event(&db,&event,&cancel) {
                assert_eq!(error,TimingReadError::Limit);full=true;break;
            }
            if (i+1)%TRANSACTION_ROWS==0 {
                sql(&db,&cancel,|db|db.execute_batch("COMMIT; BEGIN IMMEDIATE")).unwrap();committed+=1;
            }
        }
        assert!(full&&committed>=1);
    }
    drop(partial);assert!(!partial_path.exists());
    assert_eq!(fixture.build(&cancel,12).err(),Some(TimingReadError::Limit));
    let index=fixture.build(&cancel,MAX_PAGES).unwrap();
    assert!(matches!(index.events(&[Some(&format!("{:064x}",1024))],&cancel).unwrap()[0],EventPosition::Source{observation_sequence:1023,..}));
    let directory=index._directory.path().to_owned();drop(index);assert!(!directory.exists());
}

#[test]
fn invalid_last_row_or_summary_never_yields_an_index_and_cancel_precedes_reading() {
    let owner=owner();let fixture=Fixture::new(owner.clone(),2,0,false,|writer| {
        for i in 0..2 {write_row(writer,row(&owner,i,"soop_event_clock",event(i,i,true,5)));}
    });
    let mut raw=fs::read(fixture.file.path()).unwrap();raw.extend_from_slice(b"{}\n");
    let mut cursor=std::io::Cursor::new(raw);
    assert!(DiskIndex::build(&mut cursor,&owner,fixture.durable,0,false,&fixture.summary,&AtomicBool::new(false),MAX_PAGES).is_err());
    cursor.set_position(0);
    assert_eq!(DiskIndex::build(&mut cursor,&owner,fixture.durable,0,false,&fixture.summary,&AtomicBool::new(true),MAX_PAGES).err(),Some(TimingReadError::Cancelled));
    assert_eq!(cursor.position(),0);
    let mut wrong=fixture.summary.clone();wrong.records+=1;
    assert_eq!(DiskIndex::build(&mut BufReader::new(File::open(fixture.file.path()).unwrap()),&owner,fixture.durable,0,false,&wrong,&AtomicBool::new(false),MAX_PAGES).err(),Some(TimingReadError::Identity));
}

#[test]
fn mutex_wait_observes_real_cancellation_without_waiting_for_holder() {
    let mutex=Mutex::new(());let held=mutex.lock().unwrap();let cancel=AtomicBool::new(false);
    std::thread::scope(|scope| {
        let worker=scope.spawn(||lock(&mutex,&cancel).err());
        std::thread::sleep(Duration::from_millis(15));cancel.store(true,Ordering::Release);
        assert_eq!(worker.join().unwrap(),Some(TimingReadError::Cancelled));
    });
    drop(held);
}

#[test]
fn cancel_during_ledger_build_discards_transactions_and_same_source_can_retry() {
    let owner=owner();let fixture=Fixture::new(owner.clone(),1024,0,false,|writer| {
        for i in 0..1024 {write_row(writer,row(&owner,i,"soop_event_clock",event(i,i,true,5)));}
    });
    let before: [u8;32]=Sha256::digest(fs::read(fixture.file.path()).unwrap()).into();
    let cancel=AtomicBool::new(false);
    struct CancellingReader<'a> { inner:BufReader<File>, cancel:&'a AtomicBool, consumed:usize, threshold:usize }
    impl Read for CancellingReader<'_> {
        fn read(&mut self,bytes:&mut [u8])->std::io::Result<usize>{self.inner.read(bytes)}
    }
    impl BufRead for CancellingReader<'_> {
        fn fill_buf(&mut self)->std::io::Result<&[u8]>{self.inner.fill_buf()}
        fn consume(&mut self,bytes:usize){self.inner.consume(bytes);self.consumed+=bytes;if self.consumed>self.threshold{self.cancel.store(true,Ordering::Release);}}
    }
    let threshold=fs::metadata(fixture.file.path()).unwrap().len() as usize/2;
    let mut reader=CancellingReader{inner:BufReader::new(File::open(fixture.file.path()).unwrap()),cancel:&cancel,consumed:0,threshold};
    assert_eq!(DiskIndex::build(&mut reader,&owner,fixture.durable,0,false,&fixture.summary,&cancel,MAX_PAGES).err(),Some(TimingReadError::Cancelled));
    assert!(reader.consumed>0&&reader.consumed<fs::metadata(fixture.file.path()).unwrap().len() as usize);
    cancel.store(false,Ordering::Release);
    let index=fixture.build(&cancel,MAX_PAGES).unwrap();
    assert!(matches!(index.events(&[Some(&format!("{:064x}",1024))],&cancel).unwrap()[0],EventPosition::Source{observation_sequence:1023,..}));
    let after: [u8;32]=Sha256::digest(fs::read(fixture.file.path()).unwrap()).into();assert_eq!(before,after);
}

#[test]
fn sqlite_memory_and_query_work_are_bounded_and_interrupt_is_not_fake_user_cancel() {
    let cancel=AtomicBool::new(false);let index=DiskIndex::create(MAX_PAGES,&cancel).unwrap();
    let db=lock(&index.connection,&cancel).unwrap();
    for (pragma,expected) in [("cache_size",-2048),("mmap_size",0),("temp_store",1),("busy_timeout",0),("max_page_count",i64::from(MAX_PAGES))] {
        assert_eq!(db.query_row(&format!("PRAGMA {pragma}"),[],|r|r.get::<_,i64>(0)).unwrap(),expected);
    }
    let mut plan=db.prepare(&format!("EXPLAIN QUERY PLAN {ACK_QUERY}")).unwrap();
    let details=plan.query_map([&0u64.to_be_bytes()[..]],|r|r.get::<_,String>(3)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(details.iter().filter(|s|s.contains("SEARCH acks USING PRIMARY KEY")).count(),2);
    assert!(!details.iter().any(|s|s.contains("SCAN acks")||s.contains("USE TEMP B-TREE")));drop(plan);
    // This deliberately unbounded SQL exists only in this test. Production
    // statements are fixed insert/PK/LIMIT3 queries and <=256-row commits.
    let started=Instant::now();
    let result=sql(&db,&cancel,|db|db.query_row("WITH RECURSIVE n(x) AS(VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n",[],|r|r.get::<_,i64>(0)));
    assert_eq!(result.err(),Some(TimingReadError::Io));assert!(!cancel.load(Ordering::Acquire));
    assert!(started.elapsed()<Duration::from_secs(15));
    assert_eq!(sql(&db,&cancel,|db|db.query_row("SELECT 1",[],|r|r.get::<_,i64>(0))).unwrap(),1);
    let result=sql(&db,&cancel,|db|{cancel.store(true,Ordering::Release);db.query_row("SELECT 1",[],|r|r.get::<_,i64>(0))});
    assert_eq!(result.err(),Some(TimingReadError::Cancelled));
}
