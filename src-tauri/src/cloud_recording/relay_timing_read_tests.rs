use super::*;
use serde_json::{json, Value};
use std::io::Cursor;

const RUN: &str = "11111111-1111-4111-8111-111111111111";
const SESSION: &str = "22222222-2222-4222-8222-222222222222";
const SIDECAR: &str = "33333333-3333-4333-8333-333333333333";
const SOURCE: &str = "44444444-4444-4444-8444-444444444444";
const STREAM: &str = "55555555-5555-4555-8555-555555555555";
fn binding() -> Value { json!({"receiverId":SESSION,"runId":RUN,"sourceId":SOURCE,"documentEpoch":1,"channelId":"fixture","broadcastId":"123"}) }
fn owner_value(d: Dialect) -> Value {
    match d {
        Dialect::Soop => json!({"version":1,"kind":"soop-relay-chat","sidecarId":SIDECAR,"binding":binding(),"timelineQuality":"receive-monotonic-only","mediaJob":false}),
        Dialect::Chzzk => json!({"version":1,"provider":"chzzk","runId":RUN,"sessionId":SESSION,"sourceId":SOURCE,"documentEpoch":1,"accountRevision":7,"channelId":"0123456789abcdef0123456789abcdef","startedAtMs":1000,"clock":"native_receipt","remoteArchiveMapping":"unknown"}),
    }
}
fn owner(d: Dialect) -> ExpectedOwner {
    let bytes=serde_json::to_vec(&owner_value(d)).unwrap();
    match d { Dialect::Soop=>ExpectedOwner::Soop(parse(&bytes).unwrap()), Dialect::Chzzk=>ExpectedOwner::Chzzk(parse(&bytes).unwrap()) }
}
fn timing(n: u64) -> Value {
    let v=90000+n*180000; let a=48000+n*96256;
    json!({"originTicks":48000,"originTimescale":48000,"segmentStart90k":n*180000,"segmentEnd90k":(n+1)*180000,
        "video":{"timescale":90000,"firstDts":v,"lastDtsEnd":v+180000,"minPts":v,"maxPtsEnd":v+180000,"samples":60,
        "firstDts90k":v-90000,"lastDtsEnd90k":v+90000,"minPts90k":v-90000,"maxPtsEnd90k":v+90000},
        "audio":{"timescale":48000,"firstDts":a,"lastDtsEnd":a+96256,"minPts":a,"maxPtsEnd":a+96256,"samples":94,
        "firstDts90k":(a-48000)*15/8,"lastDtsEnd90k":(a+96256-48000)*15/8,"minPts90k":(a-48000)*15/8,"maxPtsEnd90k":(a+96256-48000)*15/8}})
}
fn envelope(sequence: u64,kind:&str,data:Value)->Value {json!({"version":1,"sequence":sequence,"kind":kind,"binding":binding(),"data":data})}
fn segment(d:Dialect,n:u64,sequence:u64)->Value {
    let before=100+n*100;
    match d {
        Dialect::Soop=>envelope(sequence,"segment_ack",json!({"streamId":STREAM,"ordinal":n,"bytes":1880,"sha256":"ab".repeat(32),"timing":timing(n),"nativeBeforeNs":before,"nativeAfterNs":before+10,"archiveMapping":null})),
        Dialect::Chzzk=>json!({"version":1,"kind":"segment_ack","streamId":STREAM,"ordinal":n,"sha256":"ab".repeat(32),"source":timing(n),"nativeBeforeNs":before,"nativeAfterNs":before+10}),
    }
}
fn end(d:Dialect,count:u64,sequence:u64)->Value {
    let mut v=json!({"streamId":STREAM,"segments":count,"origin":{"ticks":48000,"timescale":48000,"videoBase":90000,"audioBase":48000,"videoRemainder":0,"audioRemainder":0},"preroll":{"videoSamples":30,"audioSamples":47,"bytes":9000},"nativeBeforeNs":1000,"nativeAfterNs":1010});
    match d { Dialect::Soop=>{v["archiveMapping"]=Value::Null;envelope(sequence,"end_ack",v)}, Dialect::Chzzk=>{v["version"]=json!(1);v["kind"]=json!("end_ack");v} }
}
fn observation(d:Dialect,sequence:u64,known:bool)->Value {
    match d {
        Dialect::Soop=>envelope(sequence,"source_observation",json!({"nonce":"66666666-6666-4666-8666-666666666666","nativeBeforeNs":1100,"nativeAfterNs":1200,
            "sample":if known {json!({"sourceTimeSeconds":2.0,"currentTimeSeconds":4.0,"timestampOffsetSeconds":2.0,"performanceNowMs":500.,"playbackRate":1.,"paused":false,"seeking":false,"readyState":4})}else{Value::Null},
            "usable":known,"quality":"presentation-observation-bracket","archiveMapping":null})),
        Dialect::Chzzk=>json!({"version":1,"kind":"source_observation","nonce":"66666666-6666-4666-8666-666666666666","nativeBeforeNs":1100,"nativeAfterNs":1200,
            "sample":if known {json!({"sourceTimeSeconds":2.0,"currentTime":4.0,"timestampOffset":2.0,"performanceNowMs":500.,"trackIndex":0,"sourceId":SOURCE,"clock":"mse_presentation_v1"})}else{Value::Null},
            "quality":if known {"source_bound_player_observation"}else{"unknown"}}),
    }
}
fn serialize(rows:&[Value])->Vec<u8> {
    let mut out=Vec::new();for row in rows {serde_json::to_writer(&mut out,row).unwrap();out.push(b'\n');}out
}
fn inspect(d:Dialect,rows:&[Value],acks:u64,ended:bool)->Result<RelayTimingSummary> {
    scan(&mut Cursor::new(serialize(rows)),&owner(d),Durable{records:rows.len() as u64,incomplete:false,sidecar_incomplete:false},acks,ended,&AtomicBool::new(false)).map(|x|x.0)
}
fn summary_value(d:Dialect,count:u64)->Value {
    let s=json!({"sidecarId":if d==Dialect::Soop {SIDECAR}else{RUN},"joined":false,"rawRows":2,"eventRows":2,"bytes":250,
        "interrupted":false,"assetsIncomplete":false,"failure":if d==Dialect::Soop {Value::Null}else{json!(false)},
        "textState":if d==Dialect::Soop {"complete"}else{"ended"},"clockQuality":if d==Dialect::Soop {"receive-monotonic-only"}else{"native_receipt_and_separate_source_observations"},"timingRecords":count,"timingIncomplete":false});
    if d==Dialect::Soop {s}else{json!({"version":1,"binding":owner_value(d),"summary":s,"assetExpected":0,"assetIncomplete":false,"remoteArchiveMapping":"unknown"})}
}

#[test]
fn dialects_preserve_exact_shared_origin_ack_ranges_and_unknown_remote_anchor() {
    let mut results=Vec::new();
    for d in [Dialect::Soop,Dialect::Chzzk] {
        let rows=[segment(d,0,0),segment(d,1,1),end(d,2,2)];
        let r=inspect(d,&rows,2,true).unwrap();
        assert!(r.transport_complete);assert_eq!(r.prefix_segments,2);assert_eq!(r.transport_end_90k,Some(360000));
        assert_eq!(r.video.unwrap().samples,120);assert_eq!(r.audio.unwrap().samples,188);
        assert_eq!(r.remote_anchor,RemoteAnchor::Unknown);
        assert_eq!(map(&r,Track::Video,SourceTicks{ticks:180000,timescale:90000}).unwrap(),MappedPosition::Transport90k(90000));
        assert_eq!(map(&r,Track::Audio,SourceTicks{ticks:96000,timescale:48000}).unwrap(),MappedPosition::Transport90k(90000));
        results.push(r);
    }
    assert_eq!(results[0].origin,results[1].origin);assert_eq!(results[0].video,results[1].video);assert_eq!(results[0].audio,results[1].audio);
}
#[test]
fn ack_gap_preserves_only_prior_prefix_and_never_maps_across_missing_media() {
    for d in [Dialect::Soop,Dialect::Chzzk] {
        let r=inspect(d,&[segment(d,0,0),segment(d,2,1),end(d,3,2)],3,true).unwrap();
        assert!(!r.transport_complete);assert!(r.timing_incomplete);assert_eq!(r.prefix_segments,1);
        assert_eq!(map(&r,Track::Video,SourceTicks{ticks:450000,timescale:90000}).unwrap(),MappedPosition::OutsideAcknowledgedPrefix);
        let late=inspect(d,&[segment(d,1,0),end(d,2,1)],2,true).unwrap();
        assert_eq!(late.prefix_segments,0);assert_eq!(map(&late,Track::Audio,SourceTicks{ticks:144256,timescale:48000}).unwrap(),MappedPosition::Unknown);
    }
}
#[test]
fn duplicate_reset_changed_stream_clock_and_false_end_are_rejected() {
    for d in [Dialect::Soop,Dialect::Chzzk] {
        assert!(inspect(d,&[segment(d,0,0),segment(d,0,1)],2,false).is_err());
        let mut changed=segment(d,1,1);let data=if d==Dialect::Soop {&mut changed["data"]}else{&mut changed};
        data["streamId"]=json!(SOURCE);
        assert_eq!(inspect(d,&[segment(d,0,0),changed],2,false).err(),Some(TimingReadError::Identity));
        let mut reset=segment(d,1,1);let t=if d==Dialect::Soop {&mut reset["data"]["timing"]}else{&mut reset["source"]};
        t["segmentStart90k"]=json!(0);
        assert!(inspect(d,&[segment(d,0,0),reset],2,false).is_err());
        assert!(inspect(d,&[segment(d,0,0),end(d,1,1)],1,false).is_err());
        assert!(inspect(d,&[segment(d,0,0),end(d,2,1)],1,true).is_err());
        let missing=inspect(d,&[segment(d,0,0)],1,true).unwrap();assert!(!missing.transport_complete);
    }
}
#[test]
fn source_numeric_corruption_remainder_and_unsupported_pts_dts_are_rejected() {
    for (key,value) in [("timescale",json!(0)),("firstDts90k",json!(1)),("minPts",json!(89999)),("samples",json!(0)),("maxPtsEnd",json!(u64::MAX))] {
        let mut row=segment(Dialect::Soop,0,0);row["data"]["timing"]["video"][key]=value;
        assert!(inspect(Dialect::Soop,&[row],1,false).is_err(),"{key}");
    }
    let mut e=end(Dialect::Soop,1,1);e["data"]["origin"]["videoRemainder"]=json!(1);
    assert!(inspect(Dialect::Soop,&[segment(Dialect::Soop,0,0),e],1,true).is_err());
}
#[test]
fn rational_rounding_uses_original_ticks_without_millisecond_accumulation() {
    let zero=SourceTicks{ticks:0,timescale:48000};
    assert_eq!(to90k(1,180000,zero).unwrap(),1); // exact half rounds up
    assert_eq!(to90k(1,180001,zero).unwrap(),0);
    assert_eq!(to90k(47999,48000,zero).unwrap(),89998);
    let origin=SourceTicks{ticks:47999,timescale:48000};
    assert_eq!(to90k(90000,90000,origin).unwrap(),2);
    let hours=7200u64*48000;
    assert_eq!(to90k(hours+47999,48000,origin).unwrap(),7200*90000);
    assert!(to90k(0,48000,origin).is_err());
    assert!(to90k(u64::MAX,1,zero).is_err());
    assert!(to90k(1,0,zero).is_err());
}
#[test]
fn observation_playback_unknowns_do_not_destroy_encoded_mapping_or_infer_remote_zero() {
    let d=Dialect::Soop;
    let mut paused=observation(d,2,true);paused["data"]["sample"]["paused"]=json!(true);paused["data"]["usable"]=json!(false);
    let r=inspect(d,&[segment(d,0,0),end(d,1,1),paused],1,true).unwrap();
    assert!(r.transport_complete);assert_eq!(r.usable_observations,0);assert_eq!(r.unknown_observations,1);
    let d=Dialect::Chzzk;
    for known in [true,false] {
        // Current writer can settle the last probe after its ENDLIST receipt.
        let r=inspect(d,&[segment(d,0,0),end(d,1,1),observation(d,2,known)],1,true).unwrap();
        assert!(r.transport_complete);assert_eq!(r.usable_observations,0);assert_eq!(r.unknown_observations,1);
        assert_eq!(r.remote_anchor,RemoteAnchor::Unknown);
    }
}
fn playing_chzzk_observation() -> Value {
    let mut row=observation(Dialect::Chzzk,2,true);
    for (key,value) in [("paused",json!(false)),("seeking",json!(false)),("readyState",json!(4)),
        ("ended",json!(false)),("playbackRate",json!(1.))] { row["sample"][key]=value; }
    row
}
#[test]
fn chzzk_observed_playback_state_qualifies_without_claiming_chat_or_remote_mapping() {
    let d=Dialect::Chzzk;
    let r=inspect(d,&[segment(d,0,0),end(d,1,1),playing_chzzk_observation()],1,true).unwrap();
    assert!(r.transport_complete); assert_eq!(r.usable_observations,1); assert_eq!(r.unknown_observations,0);
    assert_eq!(r.remote_anchor,RemoteAnchor::Unknown);
    // Old numeric-only observations retain their old unknown status.
    let old=inspect(d,&[observation(d,0,true)],0,false).unwrap();
    assert_eq!(old.usable_observations,0); assert_eq!(old.unknown_observations,1);
    for (key,value) in [("paused",json!(true)),("seeking",json!(true)),("readyState",json!(1)),("ended",json!(true))] {
        let mut row=playing_chzzk_observation(); row["sample"][key]=value;
        let r=inspect(d,&[segment(d,0,0),end(d,1,1),row],1,true).unwrap();
        assert!(r.transport_complete); assert_eq!(r.usable_observations,0); assert_eq!(r.unknown_observations,1);
        assert_eq!(r.remote_anchor,RemoteAnchor::Unknown);
    }
}
#[test]
fn chzzk_playback_state_partial_invalid_rate_or_overlong_bracket_is_rejected() {
    let d=Dialect::Chzzk;
    for key in ["paused","seeking","readyState","ended","playbackRate"] {
        let mut row=playing_chzzk_observation(); row["sample"].as_object_mut().unwrap().remove(key);
        assert_eq!(inspect(d,&[row],0,false).err(),Some(TimingReadError::Clock),"{key}");
    }
    for (key,value) in [("readyState",json!(5)),("playbackRate",json!(0.)),("playbackRate",json!(17.))] {
        let mut row=playing_chzzk_observation(); row["sample"][key]=value;
        assert_eq!(inspect(d,&[row],0,false).err(),Some(TimingReadError::Clock),"{key}");
    }
    let mut row=playing_chzzk_observation(); row["nativeAfterNs"]=json!(3_000_001_101u64);
    assert_eq!(inspect(d,&[row],0,false).err(),Some(TimingReadError::Clock));
}
#[test]
fn observation_identity_equation_bracket_order_and_duplicate_are_checked() {
    let d=Dialect::Chzzk;
    for (key,value) in [("sourceId",json!(SESSION)),("currentTime",json!(7.)),("trackIndex",json!(2)),("performanceNowMs",json!(-1))] {
        let mut o=observation(d,0,true);o["sample"][key]=value;
        assert!(inspect(d,&[o],0,false).is_err(),"{key}");
    }
    let mut o=observation(d,0,true);o["nativeAfterNs"]=json!(1);
    assert!(inspect(d,&[o],0,false).is_err());
    assert!(inspect(d,&[observation(d,0,true),observation(d,1,true)],0,false).is_err());
    let mut s=observation(Dialect::Soop,0,true);s["data"]["usable"]=json!(false);
    assert!(inspect(Dialect::Soop,&[s],0,false).is_err());
}
#[test]
fn typed_rows_reject_duplicate_unknown_fields_and_any_remote_mapping() {
    for d in [Dialect::Soop,Dialect::Chzzk] {
        let good=serialize(&[segment(d,0,0)]);
        let duplicate=String::from_utf8(good.clone()).unwrap().replacen("\"version\":1","\"version\":1,\"version\":1",1);
        assert!(scan(&mut Cursor::new(duplicate),&owner(d),Durable{records:1,incomplete:false,sidecar_incomplete:false},1,false,&AtomicBool::new(false)).is_err());
        let mut row=segment(d,0,0);row["privateField"]=json!("disallowed");
        assert!(inspect(d,&[row],1,false).is_err());
    }
    let mut r=segment(Dialect::Soop,0,0);r["data"]["archiveMapping"]=json!({"remoteZero":0});
    assert!(inspect(Dialect::Soop,&[r],1,false).is_err());
    let bytes=String::from_utf8(serialize(&[segment(Dialect::Soop,0,0)])).unwrap().replacen("\"firstDts\":90000","\"firstDts\":90000,\"firstDts\":90000",1);
    assert!(scan(&mut Cursor::new(bytes),&owner(Dialect::Soop),Durable{records:1,incomplete:false,sidecar_incomplete:false},1,false,&AtomicBool::new(false)).is_err());
}
#[test]
fn streaming_reader_bounds_tail_counts_and_cancellation_without_unbounded_allocation() {
    let d=Dialect::Soop;let good=serialize(&[segment(d,0,0)]);
    let durable=Durable{records:1,incomplete:false,sidecar_incomplete:false};
    assert_eq!(scan(&mut Cursor::new(&good),&owner(d),durable,1,false,&AtomicBool::new(true)).err(),Some(TimingReadError::Cancelled));
    assert!(scan(&mut Cursor::new(&good[..good.len()-1]),&owner(d),durable,1,false,&AtomicBool::new(false)).is_err());
    assert_eq!(scan(&mut Cursor::new(vec![b' ';LINE_BYTES*2]),&owner(d),durable,1,false,&AtomicBool::new(false)).err(),Some(TimingReadError::Limit));
    assert!(scan(&mut Cursor::new(good),&owner(d),Durable{records:2,incomplete:false,sidecar_incomplete:false},1,false,&AtomicBool::new(false)).is_err());
}
#[test]
fn exact_owner_and_summary_binding_reject_foreign_runs_and_false_durable_join() {
    for d in [Dialect::Soop,Dialect::Chzzk] {
        let o=serde_json::to_vec(&owner_value(d)).unwrap();let s=serde_json::to_vec(&summary_value(d,0)).unwrap();
        assert!(validate_controls(&owner(d),&o,&s).is_ok());
        let mut foreign=owner_value(d);
        if d==Dialect::Soop {foreign["binding"]["receiverId"]=json!(SOURCE);}else{foreign["sessionId"]=json!(SOURCE);}
        assert_eq!(validate_controls(&owner(d),&serde_json::to_vec(&foreign).unwrap(),&s).err(),Some(TimingReadError::Identity));
        let mut joined=summary_value(d,0);
        if d==Dialect::Soop {joined["joined"]=json!(true);}else{joined["summary"]["joined"]=json!(true);}
        assert!(validate_controls(&owner(d),&o,&serde_json::to_vec(&joined).unwrap()).is_err());
    }
}

fn proof(bytes:&[u8])->FileProof {FileProof{bytes:bytes.len() as u64,sha256:Sha256::digest(bytes).into()}}
#[cfg(windows)]
fn local_fixture(d:Dialect)->(tempfile::TempDir,RelaySidecarReadAuthorization) {
    let dir=tempfile::tempdir().unwrap();let id=if d==Dialect::Soop {SIDECAR}else{RUN};let root=dir.path().join(id);fs::create_dir(&root).unwrap();
    let data=[serde_json::to_vec(&owner_value(d)).unwrap(),serde_json::to_vec(&summary_value(d,3)).unwrap(),serialize(&[segment(d,0,0),segment(d,1,1),end(d,2,2)])];
    for (name,bytes) in ["relay-owner.json","relay-summary.json","relay-timing.jsonl"].iter().zip(data.iter()) {fs::write(root.join(name),bytes).unwrap();}
    let auth=RelaySidecarReadAuthorization{root:fs::canonicalize(root).unwrap(),owner:owner(d),files:[proof(&data[0]),proof(&data[1]),proof(&data[2])],acknowledged_segments:2,end_list_acked:true};
    (dir,auth)
}
#[test]
#[cfg(windows)]
fn lease_denies_file_write_delete_and_directory_move_until_drop_without_media_access() {
    for d in [Dialect::Soop,Dialect::Chzzk] {
        let (_dir,auth)=local_fixture(d);let sentinel=auth.root.join("original-video.untouched");fs::write(&sentinel,b"not opened by reader").unwrap();
        let lease=RelayTimingLease::open(&auth,&AtomicBool::new(false)).unwrap();assert!(lease.summary().transport_complete);
        assert_eq!(lease.map_source_pts(Track::Video,SourceTicks{ticks:180000,timescale:90000}).unwrap(),MappedPosition::Transport90k(90000));
        let path=auth.root.join("relay-timing.jsonl");
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());assert!(fs::remove_file(&path).is_err());
        assert!(fs::rename(&auth.root,auth.root.with_file_name("moved")).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(),b"not opened by reader");drop(lease);
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
        let now=fs::read(&path).unwrap();assert_eq!(<[u8;32]>::from(Sha256::digest(now)),auth.files[2].sha256);
    }
}
#[test]
#[cfg(windows)]
fn point_index_is_built_once_per_pinned_lease_and_reused_for_reverse_queries() {
    let (_dir,auth)=local_fixture(Dialect::Chzzk);
    let lease=RelayTimingLease::open(&auth,&AtomicBool::new(false)).unwrap();
    assert!(lease.disk_index.get().is_none());
    let first=lease.map_video_observations(&[180000,360000],&AtomicBool::new(false)).unwrap();
    assert_eq!(first[0].as_ref().unwrap().transport_ticks90k,90000);
    assert_eq!(first[1].as_ref().unwrap().transport_ticks90k,270000);
    let cached=lease.disk_index.get().unwrap();
    // try_clone shares the seek position. Moving it to a non-row boundary
    // proves a cached lookup neither rereads nor rescans the timing ledger.
    let mut handle=lease._files[2].try_clone().unwrap();handle.seek(SeekFrom::Start(3)).unwrap();
    let reverse=lease.map_video_observations(&[360000,180000],&AtomicBool::new(false)).unwrap();
    assert_eq!(reverse[0].as_ref().unwrap().transport_ticks90k,270000);
    assert_eq!(reverse[1].as_ref().unwrap().transport_ticks90k,90000);
    assert!(std::ptr::eq(cached,lease.disk_index.get().unwrap()));
    assert_eq!(handle.stream_position().unwrap(),3);
    assert_eq!(lease.map_video_observations(&[180000],&AtomicBool::new(true)).err(),Some(TimingReadError::Cancelled));
    assert_eq!(handle.stream_position().unwrap(),3);
}
#[test]
#[cfg(windows)]
fn lease_rejects_hash_replacement_hardlinks_and_wrong_native_root_identity() {
    let (_dir,mut auth)=local_fixture(Dialect::Soop);auth.files[2].sha256[0]^=1;
    assert_eq!(RelayTimingLease::open(&auth,&AtomicBool::new(false)).err(),Some(TimingReadError::Hash));auth.files[2].sha256[0]^=1;
    let linked=auth.root.join("other-link");fs::hard_link(auth.root.join("relay-timing.jsonl"),&linked).unwrap();
    assert_eq!(RelayTimingLease::open(&auth,&AtomicBool::new(false)).err(),Some(TimingReadError::UnsafePath));fs::remove_file(linked).unwrap();
    let previous=auth.root.clone();auth.root=auth.root.with_file_name("foreign");fs::rename(previous,&auth.root).unwrap();
    assert_eq!(RelayTimingLease::open(&auth,&AtomicBool::new(false)).err(),Some(TimingReadError::UnsafePath));
}

/// Existing numeric ledger only. This controlled read authorization is confined
/// to this child test; it is not an admission path for historical user receipts.
#[test]
#[ignore="root opt-in; held numeric sidecar read, no network or media"]
#[cfg(windows)]
fn held_soop_numeric_ledger_preserves_source_hashes_and_unknown_archive_anchor() {
    assert_eq!(std::env::var("ATSUMI_RELAY_TIMING_READ_QA").as_deref(),Ok("1"));
    let root=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(".runtime/soop-hls-native-qa/fc76ed1d5c574185a8298967f64fa861/media/SOOP/Relays/16117edc-5e64-4169-8706-c1b6d061bd52");
    let pins=[(381,"b8e1ddca66f1cdd8fae6f8249b7d2e58fcc133efc044eea086a97b2e7b2d9f34"),(274,"83458a06e66114b92ef78b72a546719f5ed8bf5330d91c4d251022d252222569"),(5023,"ab00147746eb0775bf442fa42de00847f287edf5d1259d686ab1434a352b6d9c")];
    let mut hashes=Vec::new();
    for (length,hash) in pins {
        let mut sha256=[0u8;32];
        for (index,byte) in sha256.iter_mut().enumerate() { *byte=u8::from_str_radix(&hash[index*2..index*2+2],16).unwrap(); }
        // The first content read is the bounded, pinned production lease below.
        hashes.push(FileProof{bytes:length,sha256});
    }
    // Expected identity is independently fixed, not copied out of owner JSON.
    let fixed=json!({"version":1,"kind":"soop-relay-chat","sidecarId":"16117edc-5e64-4169-8706-c1b6d061bd52","binding":{"broadcastId":"298655255","channelId":"iamquaddurup","documentEpoch":1,"receiverId":"379261a7-9793-4a52-8a17-1697bed036f9","runId":"eb2a473f-d7b1-486e-9eb2-511bd399b543","sourceId":"95efefc0-21f5-43d2-8405-4013b75e5bf1"},"timelineQuality":"receive-monotonic-only","mediaJob":false});
    let auth=RelaySidecarReadAuthorization{root:fs::canonicalize(root).unwrap(),owner:ExpectedOwner::Soop(parse(&serde_json::to_vec(&fixed).unwrap()).unwrap()),files:hashes.try_into().ok().unwrap(),acknowledged_segments:1,end_list_acked:true};
    let lease=RelayTimingLease::open(&auth,&AtomicBool::new(false)).unwrap();let report=lease.summary();
    assert!(report.sidecar_incomplete); // Original assetsIncomplete is preserved, not repaired by timing validation.
    assert_eq!(report.records,7);assert_eq!(report.observations,5);assert_eq!(report.acknowledged_segments,1);assert!(report.transport_complete);
    assert_eq!(report.remote_anchor,RemoteAnchor::Unknown);assert_eq!(report.origin,Some(SourceTicks{ticks:3001439,timescale:48000}));
    assert_eq!(report.preroll,Some(Preroll{video_samples:250,audio_samples:195,bytes:615629}));
    // Exercise the production lazy disk derivative against the independently
    // pinned actual ledger, not only newly generated unit fixtures.
    let mapped=lease.map_video_observations(&[5_800_000],&AtomicBool::new(false)).unwrap();
    let point=mapped[0].as_ref().unwrap();
    assert_eq!(point.ordinal,0);
    assert_eq!(point.segment_sha256,"feb216456f8f60f146ffa22db8fed32b498d7500e0ad847ce0365e29d7cc5a73");
    assert_eq!(point.transport_ticks90k,172_302);
    assert!(lease.disk_index.get().is_some());
    for (name,pin) in ["relay-owner.json","relay-summary.json","relay-timing.jsonl"].into_iter().zip(&auth.files) {
        assert_eq!(<[u8;32]>::from(Sha256::digest(fs::read(auth.root.join(name)).unwrap())),pin.sha256);
    }
    println!("held numeric SOOP ledger: 7 records; actual ACK disk lookup verified; remote anchor unknown; original files unchanged; no media opened");
}

#[test]
#[cfg(windows)]
fn pinned_reference_windows_bind_exact_ack_hash_and_pts_for_both_dialects() {
    use super::super::live_fingerprint::{FrameFeature, FrameReference, ReferenceWindow};
    for dialect in [Dialect::Soop, Dialect::Chzzk] {
        let (_dir, auth) = local_fixture(dialect);
        let lease = RelayTimingLease::open(&auth, &AtomicBool::new(false)).unwrap();
        let window = ReferenceWindow { ordinal: 1, segment_sha256: "ab".repeat(32),
            source_origin_ticks: 48000, source_origin_timescale: 48000, source_video_timescale: 90000,
            frames: (0..8).map(|i| FrameReference { pts90k: 180000 + i * 9000,
                feature: FrameFeature { gradient: [0;4], mean: 100, spread: 20 } }).collect() };
        lease.verify_reference_windows(&[window.clone()], &AtomicBool::new(false)).unwrap();
        // A second read starts from the pinned handle's beginning, not its old cursor.
        lease.verify_reference_windows(&[window.clone()], &AtomicBool::new(false)).unwrap();
        for fault in ["hash", "ordinal", "origin", "clock", "outside", "duplicate"] {
            let mut changed = window.clone();
            match fault {
                "hash" => changed.segment_sha256 = "cd".repeat(32),
                "ordinal" => changed.ordinal = 0,
                "origin" => changed.source_origin_ticks += 1,
                "clock" => changed.source_video_timescale = 48000,
                "outside" => changed.frames[7].pts90k = 360000,
                _ => changed.frames[1].pts90k = changed.frames[0].pts90k,
            }
            assert!(lease.verify_reference_windows(&[changed], &AtomicBool::new(false)).is_err(), "{fault}");
        }
        assert!(lease.verify_reference_windows(&[window.clone(), window.clone()], &AtomicBool::new(false)).is_err());
        assert_eq!(lease.verify_reference_windows(&[window], &AtomicBool::new(true)).err(), Some(TimingReadError::Cancelled));
    }
}
