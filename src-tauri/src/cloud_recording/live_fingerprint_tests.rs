use super::*;
fn window(ordinal:u64,base:u64,dynamic:bool)->ReferenceWindow {
    ReferenceWindow {ordinal,segment_sha256:"a".repeat(64),source_origin_ticks:12,source_origin_timescale:90000,source_video_timescale:90000,
        frames:(0..24).map(|i|FrameReference {pts90k:base+i*7500,feature:FrameFeature {gradient:if dynamic {[((i+ordinal+1)*7919)as u32,((i*23+ordinal+1)*31)as u32,1,2]}else{[0;4]},mean:100,spread:if dynamic {32}else{0}}}).collect()}
}
#[test] fn static_or_repeated_windows_do_not_claim_dynamic_reference() {
    assert!(!dynamic(&window(0,0,false)));assert!(dynamic(&window(0,0,true)));
    let mut out=Vec::new();let first=window(1,0,true);retain(&mut out,first.clone());retain(&mut out,first);assert_eq!(out.len(),1);
}
#[test] fn long_run_retains_beginning_middle_tail_with_fixed_memory() {
    let mut out=Vec::new();for ordinal in 0..1000 {retain(&mut out,window(ordinal,ordinal*10*90000,true));assert!(out.len()<=MAX_WINDOWS);}
    let selected=choose(&out);assert_eq!(selected.len(),3);assert_eq!(selected[0].ordinal,0);assert_eq!(selected[2].ordinal,999);
    assert!(selected[1].ordinal>200&&selected[1].ordinal<800);
    let encoded=serde_json::to_string(&selected).unwrap();assert!(!encoded.contains("pixels"));assert!(encoded.len()<MAX_FILE);
}
#[test] fn same_image_has_stable_compact_numbers_and_bad_size_is_rejected() {
    let p:Vec<u8>=(0..WIDTH*HEIGHT).map(|i|(i%WIDTH*7)as u8).collect();let a=feature(&p).unwrap();assert_eq!(a,feature(&p).unwrap());
    assert!(a.spread>8);assert!(feature(&p[..10]).is_err());assert_eq!(feature(&vec![0;WIDTH*HEIGHT]).unwrap().spread,0);
}
#[test] fn persistence_is_new_lightweight_file_without_overwrite() {
    let dir=tempfile::tempdir().unwrap();let root=fs::canonicalize(dir.path()).unwrap();
    let set=ReferenceSet {version:1,run_id:Uuid::new_v4(),source_id:Uuid::new_v4(),stream_id:Some(Uuid::new_v4()),windows:vec![window(0,0,true)],acknowledged_segments:1,sampled_segments:1,queue_dropped:1,decode_failures:0,static_windows:0,cancelled:false,incomplete:true,remote_anchor:"unknown".into()};
    persist(&root,&set).unwrap();let bytes=fs::read(root.join(FILE_NAME)).unwrap();assert!(bytes.len()<MAX_FILE);
    persist(&root,&set).unwrap();assert_eq!(bytes,fs::read(root.join(FILE_NAME)).unwrap());
    let mut changed=set.clone();changed.source_id=Uuid::new_v4();assert!(persist(&root,&changed).is_err());
    assert_eq!(bytes,fs::read(root.join(FILE_NAME)).unwrap());assert_eq!(fs::read_dir(&root).unwrap().count(),1);
    let read:ReferenceSet=serde_json::from_slice(&bytes).unwrap();assert!(read.incomplete);assert_eq!(read.remote_anchor,"unknown");
}
#[test] fn partial_pending_never_becomes_final_and_does_not_remove_another_generation() {
    let dir=tempfile::tempdir().unwrap();let root=fs::canonicalize(dir.path()).unwrap();
    let old=root.join(format!(".relay-frame-reference-{}.pending",Uuid::new_v4()));fs::write(&old,b"older incomplete").unwrap();
    let pending=root.join(format!(".relay-frame-reference-{}.pending",Uuid::new_v4()));
    let mut file=OpenOptions::new().write(true).create_new(true).open(&pending).unwrap();
    let owner=Pending(pending.clone());file.write_all(b"{\"version\":").unwrap();drop(file);drop(owner);
    assert!(!pending.exists());assert!(!root.join(FILE_NAME).exists());assert_eq!(fs::read(old).unwrap(),b"older incomplete");
}
#[test] fn joined_proof_survives_persist_failure_and_retries_without_decoder() {
    let dir=tempfile::tempdir().unwrap();let root=fs::canonicalize(dir.path()).unwrap();
    fs::write(root.join(FILE_NAME),b"different existing generation").unwrap();
    let set=ReferenceSet {version:1,run_id:Uuid::new_v4(),source_id:Uuid::new_v4(),stream_id:Some(Uuid::new_v4()),windows:vec![window(0,0,true)],acknowledged_segments:1,sampled_segments:1,queue_dropped:0,decode_failures:0,static_windows:0,cancelled:false,incomplete:true,remote_anchor:"unknown".into()};
    let (sender,_receiver)=mpsc::sync_channel(QUEUE);
    let shared=Arc::new(Shared {closed:AtomicBool::new(true),cancel:Arc::new(AtomicBool::new(false)),dropped:AtomicU64::new(0),acks:AtomicU64::new(1),schedule:Mutex::new(Schedule {elapsed:0,next:0,stream:set.stream_id,ordinal:1})});
    let returned=set.clone();let handle=thread::spawn(move ||returned);
    while !handle.is_finished() {thread::yield_now();}
    let mut worker=FingerprintWorker {sink:AckSink {sender,shared:shared.clone()},shared,handle:Some(handle),result:None,published_root:None};
    assert!(worker.poll_finish_into(&root).is_err());assert!(worker.handle.is_none());assert!(worker.result.is_some());assert!(worker.published_root.is_none());
    assert_eq!(fs::read(root.join(FILE_NAME)).unwrap(),b"different existing generation");
    // This is a test-owned collision only. Production never removes a final.
    fs::remove_file(root.join(FILE_NAME)).unwrap();
    let recovered=worker.poll_finish_into(&root).unwrap().unwrap();
    assert_eq!(recovered.run_id,set.run_id);assert_eq!(recovered.source_id,set.source_id);assert_eq!(recovered.stream_id,set.stream_id);
    assert_eq!(serde_json::to_vec(&recovered).unwrap(),fs::read(root.join(FILE_NAME)).unwrap());
    assert!(worker.poll_finish_into(&root).unwrap().is_some());
    let other=tempfile::tempdir().unwrap();assert!(worker.poll_finish_into(&fs::canonicalize(other.path()).unwrap()).is_err());
}
