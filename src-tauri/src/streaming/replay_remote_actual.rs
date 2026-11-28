//! Opt-in actual sidecar admission. No video bytes, source recovery, upload or UI.
//! Gate: requirements-audit/20261117-cloud-goal-resume/chzzk-actual-native-pre-edit.json.
use super::*;
use crate::cloud_recording::{replay::{CloudReplayAuthorization, CloudReplayInput}, source::{SourceCoverage, SourceDescription, SourcePart}, CloudSourceRef};
use super::super::{browser_store::{BrowserRecording, BrowserRecordingStatus}, model::ChatNoticeKind};
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use tauri::http::{header, Method, Request, StatusCode};

const RECORDING: &str = "6e82ac06ed1048c6b8201b29892ff659";
const MEDIA_SHA: &str = "e03adaf0e107aa028640329bf7c6cae918e4624f89a1c0591a69573b4c82f2a3";
const CATALOG_LINE_SHA: &str = "755ffbd9ca7f640d0afa9079543cd27513caf3c29705fcb2bdcaf76d1c8b5631";
const OFFSET: f64 = 660.1371041666571;
const DURATION: f64 = 60.00664583333128;
const EVENT: f64 = 18.403003999999783;
const BYTES: u64 = 61_411_740;
type Checked<T> = Result<T, String>;

fn error_code(error: StreamError) -> String {
    if error.code.len() <= 64 && error.code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') { error.code }
    else { "QA_NATIVE_ERROR".into() }
}
fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
struct Original { role: &'static str, path: PathBuf, file: File, stamp: FileStamp, sha256: String, bytes: Vec<u8> }
impl Original {
    fn read(role: &'static str, path: PathBuf, expected: &str) -> Checked<Self> {
        if matches!(path.extension().and_then(|s|s.to_str()), Some("mp4"|"webm"|"ts")) { return Err("QA_MEDIA_READ_PROHIBITED".into()); }
        let opened = File::open(&path).map_err(|_| format!("QA_SOURCE_OPEN_{role}"))?;
        let mut file = crate::cloud_recording::source::pin_file(&opened).map_err(|_| format!("QA_SOURCE_PIN_{role}"))?;
        let stamp = FileStamp::read(&file).map_err(error_code)?;
        if stamp.len > 2 * 1024 * 1024 { return Err(format!("QA_SOURCE_LIMIT_{role}")); }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|_| format!("QA_SOURCE_READ_{role}"))?;
        file.seek(SeekFrom::Start(0)).map_err(|_| "QA_SOURCE_REWIND")?;
        let sha256 = hash(&bytes);
        if sha256 != expected || !stamp.matches(&file) { return Err(format!("QA_FIXED_SOURCE_CHANGED_{role}")); }
        Ok(Self { role, path, file, stamp, sha256, bytes })
    }
    fn unchanged(&self) -> Checked<bool> {
        if !self.stamp.matches(&self.file) { return Ok(false); }
        let reopened = Original::read(self.role, self.path.clone(), &self.sha256)?;
        Ok(reopened.stamp == self.stamp)
    }
}
fn saved<'a>(originals: &'a [Original], role: &str) -> Checked<&'a Original> {
    originals.iter().find(|f| f.role == role).ok_or_else(|| "QA_INPUT_MISSING".into())
}
fn parse(originals: &[Original], role: &str) -> Checked<Value> {
    serde_json::from_slice(&saved(originals,role)?.bytes).map_err(|_| format!("QA_JSON_{role}"))
}
fn scratch_bytes(root: &Path) -> Checked<u64> {
    let mut sum=0u64;
    for entry in fs::read_dir(root).map_err(|_| "QA_SCRATCH_LIST")? {
        let entry=entry.map_err(|_| "QA_SCRATCH_ENTRY")?;
        let meta=fs::symlink_metadata(entry.path()).map_err(|_| "QA_SCRATCH_STAT")?;
        if meta.file_type().is_symlink() { return Err("QA_SCRATCH_LINK".into()); }
        sum=sum.checked_add(if meta.is_dir(){scratch_bytes(&entry.path())?}else{meta.len()}).ok_or("QA_SCRATCH_LIMIT")?;
    }
    Ok(sum)
}
struct Stop(ReplayService);
impl Drop for Stop { fn drop(&mut self) { self.0.shutdown_and_wait(); } }

#[test]
#[ignore = "Fixed CHZZK part11 sidecars only; explicit ATSUMI_CHZZK_SIDECAR_QA recording ID required"]
fn actual_chzzk_selected_part_cloud_admission_and_donation_readonly() -> Checked<()> {
    if std::env::var("ATSUMI_CHZZK_SIDECAR_QA").ok().as_deref() != Some(RECORDING) { return Err("QA_OPT_IN_REQUIRED".into()); }
    let workspace=Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?;
    let audit=workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume");
    let profile=PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join("local.atsumi.next");
    let local=profile.join("ssd-recordings/CHZZK/BrowserCapture").join(RECORDING);
    let archive=PathBuf::from("D:/Astumi/CHZZK/BrowserCapture").join(RECORDING);
    let mut originals=Vec::new();
    for (role,path,expected) in [
        ("METADATA",local.join("recording.json"),"74d4cb34515cd78f8face205ca87f97fef5c07d29e5ec543cf45629ba35872eb"),
        ("PROGRESSIVE",local.join("progressive.json"),"211705f2680834dbcdf4b144646d68d2910aef314e1f026d4b5666238a29ebb1"),
        ("CHAT",local.join("chat.jsonl"),"b300ed24b5e40eb6dabc2b1c1c84a308dd183ec530c2e24a83b3cb3bf4aac35e"),
        ("JOURNAL",local.join("segments.jsonl"),"a25771ead7d0295657b6e55837d676088484ed15b50b66c2270b9856fcd0d7a7"),
        ("LOCAL_RANGES",local.join("ssd-ranges.json"),"25a5f94a01941afe8caee977b9ef22b0238d454134a55810ad58d3f1a6808acb"),
        ("ARCHIVE_RANGES",archive.join("ssd-ranges.json"),"25a5f94a01941afe8caee977b9ef22b0238d454134a55810ad58d3f1a6808acb"),
        ("ARCHIVE_OWNER",archive.join("ssd-archive-owner.json"),"d5fd9eea481ac9ca492cb4df05cfd6507adf47ffcca93388c9da3e2727703400"),
        ("TIMELINE",archive.join("merged-01c92a302acd423c8ef036493f2bafb6.timeline.jsonl"),"99d4e82e647bc9639cfb24bcb29f6b0636d72fd68a891fbd44e2b7c4c8b6600d"),
        ("PROOF",archive.join("merged-01c92a302acd423c8ef036493f2bafb6.cleanup.json"),"59a421fea55cd537f9f67e49576f3fc4c123c21122dc64b8bf65c0978822ffe2"),
        ("VIEWERS",local.join("viewer-metrics.jsonl"),"0163637fc21e058ab52a545a5e6c496494ee37e0e48fe627eeff563c358b6565"),
        ("PROFILE",local.join("channel-profile.json"),"bde3bc53251d16fef9201743ba9d7196a7e1d596bdd7b901bd118d0b9a92cdf5"),
        ("CATALOG",profile.join("streaming/browser/catalog.jsonl"),"39411d1280415d0491843114cf19f0c81f4c8d977092370435933a0e0902f5c6"),
    ] { originals.push(Original::read(role,path,expected)?); }
    let stored_profile=parse(&originals,"PROFILE")?;
    let asset=stored_profile["imageAssetId"].as_str().filter(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))).ok_or("QA_PROFILE_ASSET")?;
    originals.push(Original::read("PROFILE_ASSET",local.join("replay-assets").join(format!("{asset}.json")),"dcab1a44feef4a03b48411412b04e34f8958a9ac6909761c0a8607e29841ea42")?);
    let record:BrowserRecording=serde_json::from_slice(&saved(&originals,"METADATA")?.bytes).map_err(|_|"QA_METADATA_JSON")?;
    if record.id!=RECORDING || record.status!=BrowserRecordingStatus::Stopped || record.preserve_sources || record.segment_count!=310
        || record.merge.as_ref().is_none_or(|m|m.status!=super::super::browser_store::BrowserMergeStatus::Queued) { return Err("QA_PARENT_CHANGED".into()); }
    let catalog=&saved(&originals,"CATALOG")?.bytes;
    let mut selected=None;
    for line in catalog.split_inclusive(|b|*b==b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) { continue; }
        let v:Value=serde_json::from_slice(line).map_err(|_|"QA_CATALOG_JSON")?;
        if v["id"]==RECORDING { if selected.replace(line.to_vec()).is_some(){return Err("QA_DUPLICATE_ENTRY".into());} }
    }
    let selected=selected.ok_or("QA_SELECTED_ENTRY")?;
    if hash(&selected)!=CATALOG_LINE_SHA { return Err("QA_CATALOG_LINE_CHANGED".into()); }
    let scratch=tempfile::Builder::new().prefix("chzzk-sidecar-actual-").tempdir_in(&audit).map_err(|_|"QA_SCRATCH_CREATE")?;
    fs::create_dir_all(scratch.path().join("streaming/browser")).map_err(|_|"QA_SCRATCH_CATALOG")?;
    fs::write(scratch.path().join("streaming/browser/catalog.jsonl"),&selected).map_err(|_|"QA_SCRATCH_CATALOG_WRITE")?;
    let outcome:Checked<Value>=(|| {
        let store=Arc::new(Mutex::new(BrowserCaptureStore::new_deferred(scratch.path()).map_err(error_code)?));
        let service=ReplayService::new(scratch.path(),store);
        let stop=Stop(service.clone());
        let source=CloudSourceRef{provider:"chzzk".into(),recording_id:RECORDING.into(),part_index:Some(11),plan_id:None,unit_id:None};
        let description=SourceDescription{source:source.clone(),title:"fixed read-only candidate".into(),plan_id:None,
            coverage:SourceCoverage{state:"incomplete".into(),expected_segments:Some(310),covered_segments:Some(5),missing_from_segment:None,reason:Some("Only the selected verified range is admitted.".into()),capture_warning:None},
            preparation_required:false,duration_seconds:OFFSET+DURATION,bytes:BYTES,
            parts:vec![SourcePart{source:source.clone(),unit_index:0,part_index:Some(11),duration_seconds:DURATION,media_offset_seconds:OFFSET,bytes:BYTES,kind:"direct".into()}]};
        let auth=CloudReplayAuthorization{job_id:uuid::Uuid::new_v4().to_string(),source,owner_channel_id:format!("UC{}","a".repeat(22)),cohort_id:"d".repeat(64),
            source_sha256:MEDIA_SHA.into(),bytes:BYTES,duration_seconds:DURATION,media_offset_seconds:OFFSET,
            original_inputs:vec![CloudReplayInput{index:11,part_index:Some(11),sha256:MEDIA_SHA.into(),bytes:BYTES,duration_seconds:DURATION,media_offset_seconds:OFFSET,modified_nanos:None}],
            plan_fingerprint:None,input_parts:true,description};
        let opened=service.open_cloud_chat(&auth).map_err(error_code)?;
        let started=Instant::now();
        let ready=loop {
            let state=service.session(&opened.token).map_err(error_code)?.descriptor().map_err(error_code)?;
            if state.index_state=="ready" {break state;}
            if state.index_state=="failed" {return Err("QA_INDEX_FAILED".into());}
            if started.elapsed()>Duration::from_secs(15) {return Err("QA_INDEX_TIMEOUT".into());}
            std::thread::sleep(Duration::from_millis(10));
        };
        if ready.mime_type!="application/x-atsumi-remote-chat" || !ready.parts.is_empty() || ready.warnings.is_empty()
            || (ready.duration_seconds-(OFFSET+DURATION)).abs()>0.000001 {return Err("QA_DESCRIPTOR_RANGE".into());}
        if ready.channel_name.as_ref().is_none_or(|v|v.is_empty()) || ready.channel_profile_image.as_ref().is_none_or(|v|!v.starts_with("data:image/")) {return Err("QA_STORED_PROFILE".into());}
        for method in [Method::GET,Method::HEAD] {for suffix in ["","/part/0"] {for ranged in [false,true] {
            let mut request=Request::builder().method(method.clone()).uri(format!("http://atsumi-replay.localhost/{}{suffix}",opened.token));
            if ranged {request=request.header(header::RANGE,"bytes=0-15");}
            if service.media_response(&request.body(vec![]).map_err(|_|"QA_REQUEST")?,"main").status()!=StatusCode::FORBIDDEN {return Err("QA_MEDIA_PROTOCOL_ALLOWED".into());}
        }}}
        let at=OFFSET+EVENT;
        let before=service.chat_at(&opened.token,at-0.05,1,Some(200)).map_err(error_code)?;
        let after=service.chat_at(&opened.token,at+0.05,2,Some(200)).map_err(error_code)?;
        let donation=|m:&&ReplayChatMessage|m.message.rich.as_ref().and_then(|r|r.notice.as_ref()).is_some_and(|n|n.kind==ChatNoticeKind::Donation);
        if before.items.iter().filter(donation).count()!=0 {return Err("QA_DONATION_EARLY".into());}
        let matches:Vec<_>=after.items.iter().filter(donation).collect();
        if matches.len()!=1 {return Err("QA_DONATION_COUNT".into());}
        let event=matches[0];
        if event.sync_quality!="observed_media" || (event.media_time_seconds-at).abs()>0.00001
            || event.message.rich.as_ref().and_then(|r|r.notice.as_ref()).and_then(|n|n.amount).is_none() || event.message.text.is_empty() {return Err("QA_DONATION_MAPPING".into());}
        let sequence=event.message.sequence;
        let mapped=event.media_time_seconds-OFFSET;
        let backward=service.chat_at(&opened.token,at-0.05,3,Some(200)).map_err(error_code)?;
        let forward=service.chat_at(&opened.token,at+0.05,4,Some(200)).map_err(error_code)?;
        let paused=service.chat_at(&opened.token,at+0.05,4,Some(200)).map_err(error_code)?;
        if backward.items.iter().any(|m|m.message.sequence==sequence) || forward.items.iter().filter(|m|m.message.sequence==sequence).count()!=1
            || forward.items.iter().map(|m|m.message.sequence).collect::<Vec<_>>()!=paused.items.iter().map(|m|m.message.sequence).collect::<Vec<_>>() {return Err("QA_SEEK_CHANGED".into());}
        if service.chat_at(&opened.token,at,3,None).is_ok() {return Err("QA_STALE_GENERATION_ACCEPTED".into());}
        let all=service.chat_at(&opened.token,OFFSET+DURATION,5,Some(200)).map_err(error_code)?;
        if all.items.iter().any(|m|m.media_time_seconds<OFFSET || m.media_time_seconds>=OFFSET+DURATION) {return Err("QA_OUTSIDE_AUTHORIZED_RANGE".into());}
        let timeline=service.timeline(&opened.token,Some(10.0)).map_err(error_code)?;
        let scratch_size=scratch_bytes(scratch.path())?;
        if scratch_size>32*1024*1024 {return Err("QA_SCRATCH_LIMIT".into());}
        service.close(&opened.token).map_err(error_code)?;
        if service.chat_at(&opened.token,at,6,None).is_ok(){return Err("QA_CLOSED_TOKEN_ACCEPTED".into());}
        drop(stop);
        Ok(json!({"nativeCloudSidecarAdmission":true,"isolatedDeferredStore":true,"selectedCatalogueCopiedExactly":true,
            "partIndex":11,"firstSegment":55,"endSegmentExclusive":60,"mediaOffsetSeconds":OFFSET,"durationSeconds":DURATION,
            "descriptorDurationSeconds":ready.duration_seconds,"incompleteWarning":true,"mediaProtocolDenied":true,
            "mappedDonationPartSeconds":mapped,"quality":"observed_media","donationCount":1,"amountPresent":true,"textPresent":true,
            "backwardAbsent":true,"forwardExactlyOnce":true,"stationaryStable":true,"staleGenerationRejected":true,"closedTokenRejected":true,
            "returnedRowsWithinSelectedRange":true,"storedProfileLoaded":true,"viewerTimelineBuckets":timeline.buckets.len(),
            "scratchBytesBeforeTeardown":scratch_size,"indexReadyWaitMs":started.elapsed().as_millis(),
            "parentState":"stopped","parentMergeState":"queued","parentComplete":false,"preserveSources":false}))
    })();
    let unchanged=originals.iter().map(Original::unchanged).collect::<Checked<Vec<_>>>()?.into_iter().all(|v|v);
    let result=json!({"stage":"actual-chzzk-cloud-sidecar-readonly","recordingId":RECORDING,"result":outcome.as_ref().ok(),"error":outcome.as_ref().err(),
        "originalSmallFilesPinned":originals.len(),"allReadOriginalsUnchanged":unchanged,"mediaBytesRead":0,"mediaDecoded":false,
        "encryptedUploadReceiptAdmissionExercised":false,"runningMainStoreRegistryExercised":false,"actualUiOrRemoteVideoSync":false,"uploadAttempted":false,
        "scope":"Actual selected source proof and production remote-chat capability with isolated exact catalogue snapshot; no local video, original recovery, account, network or UI."});
    fs::write(audit.join("chzzk-cloud-sidecar-actual-result.json"),serde_json::to_vec_pretty(&result).map_err(|_|"QA_RESULT_JSON")?).map_err(|_|"QA_RESULT_WRITE")?;
    if !unchanged{return Err("QA_ORIGINAL_CHANGED".into());}
    outcome.map(|_|())
}
