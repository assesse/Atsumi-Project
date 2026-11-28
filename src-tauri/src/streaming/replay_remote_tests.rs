//! Synthetic immutable receipts/sidecars. No source media file is ever created.
use super::*;
use crate::cloud_recording::{replay::{CloudReplayAuthorization, CloudReplayInput}, source::{SourceDescription, SourceCoverage}, CloudSourceRef};
use serde_json::{json, Value};
use tauri::http::{Request, Method, StatusCode, header};

struct Fixture {
    service: ReplayService,
    auth: CloudReplayAuthorization,
    output: PathBuf,
    store: Arc<Mutex<BrowserCaptureStore>>,
    _dir: tempfile::TempDir,
}
fn json_line(v: &Value) -> Vec<u8> { format!("{v}\n").into_bytes() }
fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

fn fixture(part: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let channel = "a".repeat(32);
    let output = base.join("CHZZK").join("BrowserCapture").join(&id);
    fs::create_dir_all(&output).unwrap();
    let catalog_root = base.join("streaming").join("browser");
    fs::create_dir_all(&catalog_root).unwrap();
    let mut journal = vec![];
    let count = if part { 2 } else { 1 };
    let segments: Vec<_> = (0..count).map(|i| json!({"index":i,"file":format!("segment-{i:012}.mp4"),"bytes":16,"durationSeconds":60.0,"sourceStartSeconds":100.0+60.0*i as f64,"sourceEndSeconds":160.0+60.0*i as f64})).collect();
    let mut part_rows = vec![];
    let mut last_proof = String::new();
    let mut last_token = String::new();
    for (i, segment) in segments.iter().enumerate() {
        journal.extend(json_line(segment));
        let token = uuid::Uuid::new_v4().simple().to_string();
        let timeline = json_line(&json!({"segmentIndex":i,"sourceFile":format!("segment-{i:012}.mp4"),"mergedStartSeconds":0.0,"mergedDurationSeconds":60.0,"sourceStartSeconds":100.0+60.0*i as f64,"sourceEndSeconds":160.0+60.0*i as f64,"chatRewritten":false}));
        let proof = serde_json::to_vec(&json!({"version":1,"recordingId":id,"mediaSha256":"b".repeat(64),"timelineSha256":hash(&timeline),"journalSha256":hash(&journal),"segments":[{"index":i,"bytes":16,"sha256":"c".repeat(64)}]})).unwrap();
        fs::write(output.join(format!("merged-{token}.timeline.jsonl")), timeline).unwrap();
        fs::write(output.join(format!("merged-{token}.cleanup.json")), &proof).unwrap();
        part_rows.push(json!({"first":i,"end":i+1,"file":format!("merged-{token}.mp4"),"timelineFile":format!("merged-{token}.timeline.jsonl"),"bytes":16,"duration":60.0,"offset":i as f64 * 60.0,"sourceStart":100.0+60.0*i as f64,"sourceEnd":160.0+60.0*i as f64,"proofFile":format!("merged-{token}.cleanup.json"),"proofSha256":hash(&proof)}));
        last_proof = hash(&proof); last_token = token;
    }
    fs::write(output.join("segments.jsonl"), &journal).unwrap();
    if part { fs::write(output.join("progressive.json"), serde_json::to_vec(&json!({"version":1,"recording_id":id,"parts":part_rows})).unwrap()).unwrap(); }
    let record = json!({"id":id,"channelId":channel,"title":"synthetic sidecars","startedAt":1,"updatedAt":2,"status":"stopped","mimeType":"video/mp4","outputDir":output.to_string_lossy(),"segmentCount":count,"bytesWritten":count*16,"durationSeconds":count as f64*60.0,"lastError":null,"segments":segments,"captureChat":true,"chatStatus":"complete","chatCount":5,"merge":if part { json!({"status":"queued","segmentCount":count,"updatedAt":2}) } else { json!({"status":"complete","segmentCount":count,"updatedAt":2,"file":format!("merged-{last_token}.mp4"),"timelineFile":format!("merged-{last_token}.timeline.jsonl"),"bytes":16,"durationSeconds":60.0,"sourceCleanup":{"status":"preserved","deletedSegments":0,"proofFile":format!("merged-{last_token}.cleanup.json"),"proofSha256":last_proof}}) },"progressive":if part {json!({"partCount":2,"segmentCount":2,"durationSeconds":120.0,"lastError":null})} else {Value::Null}});
    fs::write(output.join("recording.json"), serde_json::to_vec(&record).unwrap()).unwrap();
    fs::write(catalog_root.join("catalog.jsonl"), json_line(&json!({"id":id,"channelId":channel,"title":"synthetic sidecars","startedAt":1,"mimeType":"video/mp4","outputDir":output.to_string_lossy()}))).unwrap();
    let start = if part { 60.0 } else { 0.0 };
    let mut chat = vec![];
    // In-range receive time must never revive a known out-of-range observation.
    for (sequence, source_time, receive) in [(1, 99.0, start+3.0), (2, 100.0+start, start+4.0), (3, 118.4+start, start+21.6), (4, 159.999+start, start+59.999), (5, 160.0+start, start+5.0)] {
        chat.extend(json_line(&json!({"sequence":sequence,"sender":"synthetic","text":"fixture","receivedAt":sequence,"serverTime":null,"offsetSeconds":receive,"replayClock":{"version":1,"receivedAtMs":sequence,"observedMonotonicMs":1000.0,"sourceGeneration":1,"clock":"mse_presentation_v1","mediaTimeSeconds":source_time,"sourceTimeSeconds":source_time,"sourceId":"40000000-0000-4000-8000-000000000001","playbackRate":1.0},"rich":{"notice":{"kind":"donation","amount":1000}}})));
    }
    fs::write(output.join("chat.jsonl"), chat).unwrap();
    use base64::Engine;
    let asset_url="https://ssl.pstatic.net/static/synthetic-replay.png";
    let asset_id=super::super::replay_assets::asset_id(asset_url).unwrap();
    let png=include_bytes!("../../../public/original-player/assets/default_profile_dark.png");
    fs::create_dir(output.join("replay-assets")).unwrap();
    fs::write(output.join("replay-assets").join(format!("{asset_id}.json")),serde_json::to_vec(&json!({"version":1,"mime":"image/png","body":base64::engine::general_purpose::STANDARD.encode(png),"sha256":hash(png)})).unwrap()).unwrap();
    fs::write(output.join("channel-profile.json"),serde_json::to_vec(&json!({"version":1,"channelId":channel,"name":"synthetic profile","imageAssetId":asset_id})).unwrap()).unwrap();
    let raw=fs::read_to_string(output.join("chat.jsonl")).unwrap();
    let enriched=raw.lines().map(|s|{let mut v:Value=serde_json::from_str(s).unwrap();v["rich"]["emojis"]=json!([{"id":"fixture","imageUrl":asset_url}]);format!("{v}\n")}).collect::<String>();
    fs::write(output.join("chat.jsonl"),enriched).unwrap();
    let clock=json!({"version":1,"receivedAtMs":1,"observedMonotonicMs":1000.0,"sourceGeneration":1,"clock":"mse_presentation_v1","mediaTimeSeconds":start+110.0,"sourceTimeSeconds":start+110.0,"sourceId":"40000000-0000-4000-8000-000000000001","playbackRate":1.0});
    fs::write(output.join("viewer-metrics.jsonl"),json_line(&json!({"version":1,"source":"chzzk_live_status_api_v1","receivedAt":1,"offsetSeconds":start+15.0,"viewerCount":42,"replayClock":clock}))).unwrap();
    let source = CloudSourceRef { provider:"chzzk".into(), recording_id:id.clone(), part_index:part.then_some(1), plan_id:None, unit_id:None };
    let auth = CloudReplayAuthorization {job_id:uuid::Uuid::new_v4().to_string(),source:source.clone(),owner_channel_id:format!("UC{}","a".repeat(22)),cohort_id:"d".repeat(64),source_sha256:"b".repeat(64),bytes:16,duration_seconds:60.0,media_offset_seconds:start,original_inputs:vec![CloudReplayInput{index:if part {1}else{0},part_index:part.then_some(1),sha256:"b".repeat(64),bytes:16,duration_seconds:60.0,media_offset_seconds:start,modified_nanos:None}],plan_fingerprint:None,input_parts:part,description:SourceDescription{source,title:"fixture".into(),plan_id:None,coverage:SourceCoverage{state:"incomplete".into(),expected_segments:Some(count),covered_segments:Some(1),missing_from_segment:None,reason:None,capture_warning:None},preparation_required:false,duration_seconds:start+60.0,bytes:16,parts:vec![]}};
    let store = Arc::new(Mutex::new(BrowserCaptureStore::new_deferred(&base).unwrap()));
    let service = ReplayService::new(&base,store.clone());
    Fixture{service,auth,output,store,_dir:dir}
}
fn open_ready(f: &Fixture) -> ReplaySession {
    let opened = f.service.open_cloud_chat(&f.auth).unwrap();
    for _ in 0..12000 {
        let desc = f.service.session(&opened.token).unwrap().descriptor().unwrap();
        if desc.index_state == "ready" { return desc; }
        assert_ne!(desc.index_state,"failed","{:?}",desc.warnings);
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("sidecar index timeout")
}
fn reap(service: &ReplayService) {
    for worker in std::mem::take(&mut *service.inner.workers.lock().unwrap()) { worker.handle.join().unwrap(); }
}

#[test]
fn remote_sidecar_without_any_video_preserves_observed_donation_and_seek_boundaries() {
    for part in [false,true] {
        let f = fixture(part);
        let before = fs::read(f.output.join("chat.jsonl")).unwrap();
        let meta_before = fs::read(f.output.join("recording.json")).unwrap();
        let opened = open_ready(&f);
        let start = f.auth.media_offset_seconds;
        assert_eq!(opened.mime_type,"application/x-atsumi-remote-chat");
        assert!(opened.parts.is_empty());
        assert!(!opened.warnings.is_empty());
        assert_eq!(opened.channel_name.as_deref(),Some("synthetic profile"));
        assert!(opened.channel_profile_image.as_ref().is_some_and(|s|s.starts_with("data:image/png;base64,")));
        let check = |t,g| f.service.chat_at(&opened.token,t,g,None).unwrap();
        assert_eq!(check(start+18.399,1).items.len(),1);
        let at = check(start+18.401,2);
        assert_eq!(at.items.len(),2);
        assert!((at.items[1].media_time_seconds-(start+18.4)).abs()<0.00001);
        assert_eq!(at.items[1].message.rich.as_ref().unwrap().notice.as_ref().unwrap().amount,Some(1000));
        assert_eq!(check(start+18.401,2).items.len(),2);
        assert_eq!(check(start+1.0,3).items.len(),1);
        assert_eq!(check(start+18.401,4).items.len(),2);
        assert_eq!(check(start+60.0,5).items.len(),3);
        assert!(check(start+60.0,5).items.iter().all(|v|v.sync_quality=="observed_media"));
        let timeline=f.service.timeline(&opened.token,Some(10.0)).unwrap();
        assert!(timeline.buckets.iter().any(|b|b.viewer_count==Some(42)));
        let asset_id=at.items[1].asset_ids.values().next().unwrap();
        let asset=f.service.media_response(&Request::builder().uri(format!("http://atsumi-replay.localhost/{}/asset/{asset_id}",opened.token)).body(vec![]).unwrap(),"main");
        assert_eq!(asset.status(),StatusCode::OK);
        assert!(!asset.body().is_empty());
        assert!(f.service.chat_at(&opened.token,start+1.0,4,None).is_err());
        assert_eq!(fs::read(f.output.join("chat.jsonl")).unwrap(),before);
        assert_eq!(fs::read(f.output.join("recording.json")).unwrap(),meta_before);
        assert!(fs::read_dir(&f.output).unwrap().all(|e|e.unwrap().path().extension().is_none_or(|v|v!="mp4")));
        f.service.shutdown_and_wait();
    }
}

#[test]
fn remote_token_denies_all_video_and_part_requests_and_close_preserves_sibling_index() {
    let f = fixture(false);
    let first = open_ready(&f);
    let second = open_ready(&f);
    let index = f.service.session(&first.token).unwrap().index_path.clone();
    assert_eq!(index,f.service.session(&second.token).unwrap().index_path);
    for method in [Method::GET,Method::HEAD] { for path in ["","/part/0","/part/999"] { for range in [false,true] {
        let mut req=Request::builder().method(method.clone()).uri(format!("http://atsumi-replay.localhost/{}{path}",first.token));
        if range { req=req.header(header::RANGE,"bytes=0-15"); }
        assert_eq!(f.service.media_response(&req.body(vec![]).unwrap(),"main").status(),StatusCode::FORBIDDEN);
    }}}
    f.service.close(&first.token).unwrap(); reap(&f.service);
    assert!(index.exists());
    assert_eq!(f.service.chat_at(&second.token,30.0,1,None).unwrap().items.len(),2);
    assert!(f.service.chat_at(&first.token,30.0,1,None).is_err());
    assert!(f.service.prepare_recording_delete(&f.auth.source.recording_id).is_err());
    f.service.shutdown_and_wait();
    assert!(f.service.chat_at(&second.token,30.0,1,None).is_err());
    assert!(f.service.open_cloud_chat(&f.auth).is_err());
}

#[test]
fn remote_admission_rejects_wrong_receipt_timeline_generation_and_missing_chat() {
    for alteration in 0..6 {
        let mut f=fixture(true);
        match alteration {
            0=>f.auth.original_inputs[0].sha256="e".repeat(64),
            1=>f.auth.original_inputs[0].media_offset_seconds=0.0,
            2=>f.auth.plan_fingerprint=Some("f".repeat(64)),
            3=>{fs::remove_file(f.output.join("chat.jsonl")).unwrap();},
            4=>{let p:Value=serde_json::from_slice(&fs::read(f.output.join("progressive.json")).unwrap()).unwrap();fs::write(f.output.join(p["parts"][1]["timelineFile"].as_str().unwrap()),b"changed").unwrap();},
            _=>f.auth.source.provider="soop".into(),
        }
        assert!(f.service.open_cloud_chat(&f.auth).is_err(),"alteration {alteration}");
        assert!(f.service.inner.sessions.lock().unwrap().is_empty());
    }
}

#[test]
fn remote_legacy_verified_part_ignores_uncommitted_tail_without_promoting_recording() {
    use std::io::Write;
    let f=fixture(true);
    let path=f.output.join("recording.json");
    let mut record:Value=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["segmentCount"]=json!(3);
    record["durationSeconds"]=json!(180.0);
    record["merge"]["segmentCount"]=json!(3);
    fs::write(&path,serde_json::to_vec(&record).unwrap()).unwrap();
    fs::OpenOptions::new().append(true).open(f.output.join("segments.jsonl")).unwrap().write_all(b"{uncommitted tail").unwrap();
    let before=fs::read(&path).unwrap();
    let opened=open_ready(&f);
    assert_eq!(opened.duration_seconds,120.0);
    assert!(!opened.warnings.is_empty());
    assert_eq!(f.service.chat_at(&opened.token,90.0,0,None).unwrap().items.len(),2);
    assert_eq!(fs::read(&path).unwrap(),before);
    assert_eq!(record["merge"]["status"],"queued");
    f.service.shutdown_and_wait();
}

#[test]
#[cfg(windows)]
fn remote_holds_readonly_sidecars_and_directory_until_last_worker_releases_lease() {
    let f=fixture(true);
    let opened=open_ready(&f);
    assert!(fs::OpenOptions::new().write(true).open(f.output.join("recording.json")).is_err());
    assert!(fs::OpenOptions::new().append(true).open(f.output.join("chat.jsonl")).is_err());
    assert!(fs::rename(&f.output,f.output.with_extension("moved")).is_err());
    assert!(f.store.lock().unwrap().prepare_delete(&f.auth.source.recording_id).is_err());
    f.service.close(&opened.token).unwrap();reap(&f.service);
    assert!(fs::OpenOptions::new().write(true).open(f.output.join("chat.jsonl")).is_ok());
}
