//! Explicit local-only evidence probe. Never initialized in production or normal tests.
//! Original media and sidecars are only opened read-only; derivatives stay in a new audit scratch.
use super::*;
use super::super::browser_store::{parts as capture_parts, BrowserRecording, BrowserRecordingStatus};
use super::super::model::ChatNoticeKind;
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};

const RECORDING: &str = "6e82ac06ed1048c6b8201b29892ff659";
const SELECTED_PART: usize = 11;
const MEDIA_SHA: &str = "e03adaf0e107aa028640329bf7c6cae918e4624f89a1c0591a69573b4c82f2a3";
const METADATA_SHA: &str = "74d4cb34515cd78f8face205ca87f97fef5c07d29e5ec543cf45629ba35872eb";
const PROGRESSIVE_SHA: &str = "211705f2680834dbcdf4b144646d68d2910aef314e1f026d4b5666238a29ebb1";
const CHAT_SHA: &str = "b300ed24b5e40eb6dabc2b1c1c84a308dd183ec530c2e24a83b3cb3bf4aac35e";
type Checked<T> = Result<T, String>;

fn code(error: StreamError) -> String {
    if error.code.len() <= 64 && error.code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') {
        error.code
    } else { "QA_NATIVE_ERROR".into() }
}
fn pinned(path: &Path) -> Checked<File> {
    let meta = fs::symlink_metadata(path).map_err(|_| "QA_SOURCE_METADATA")?;
    if !regular_metadata(&meta) { return Err("QA_SOURCE_REGULAR".into()); }
    let file = File::open(path).map_err(|_| "QA_SOURCE_OPEN")?;
    crate::cloud_recording::source::pin_file(&file).map_err(|_| "QA_SOURCE_PIN".into())
}
fn digest_file(file: &File) -> Checked<String> {
    let mut file = file.try_clone().map_err(|_| "QA_SOURCE_CLONE")?;
    file.seek(SeekFrom::Start(0)).map_err(|_| "QA_SOURCE_SEEK")?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|_| "QA_SOURCE_READ")?;
        if n == 0 { break; }
        hash.update(&buffer[..n]);
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| "QA_SOURCE_SEEK")?;
    Ok(format!("{:x}", hash.finalize()))
}
struct Original {
    path: PathBuf,
    held: File,
    stamp: FileStamp,
    sha256: Option<String>,
}
impl Original {
    fn read(path: PathBuf, hash_content: bool, limit: u64) -> Checked<Self> {
        let held = pinned(&path)?;
        let stamp = FileStamp::read(&held).map_err(code)?;
        if stamp.len > limit { return Err("QA_SOURCE_LIMIT".into()); }
        let sha256 = hash_content.then(|| digest_file(&held)).transpose()?;
        Ok(Self { path, held, stamp, sha256 })
    }
    fn unchanged(&self) -> Checked<bool> {
        let reopened = pinned(&self.path)?;
        if FileStamp::read(&reopened).map_err(code)? != self.stamp || !self.stamp.matches(&self.held) { return Ok(false); }
        Ok(match &self.sha256 { Some(expected) => digest_file(&reopened)? == *expected, None => true })
    }
}
fn bytes_under(root: &Path) -> Checked<u64> {
    let mut total = 0u64;
    for entry in fs::read_dir(root).map_err(|_| "QA_SCRATCH_READ")? {
        let entry = entry.map_err(|_| "QA_SCRATCH_ENTRY")?;
        let meta = fs::symlink_metadata(entry.path()).map_err(|_| "QA_SCRATCH_METADATA")?;
        if meta.file_type().is_symlink() { return Err("QA_SCRATCH_LINK".into()); }
        total = total.checked_add(if meta.is_dir() { bytes_under(&entry.path())? } else { meta.len() }).ok_or("QA_SCRATCH_SIZE")?;
    }
    Ok(total)
}

#[test]
#[ignore = "Explicit fixed CHZZK originals read-only; scratch replay index only; no network/app/store recovery/upload"]
fn actual_chzzk_part_donation_uses_native_observed_media_mapper_readonly() -> Checked<()> {
    if std::env::var("ATSUMI_CHZZK_DONATION_READONLY_QA").ok().as_deref() != Some(RECORDING) {
        return Err("QA_EXPLICIT_OPT_IN_REQUIRED".into());
    }
    if [MEDIA_SHA, METADATA_SHA, PROGRESSIVE_SHA, CHAT_SHA].iter().any(|value|
        value.len() != 64 || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))) {
        return Err("QA_TEST_HASH_CONSTANT_INVALID".into());
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?;
    let audit = workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume");
    let local = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?)
        .join("local.atsumi.next/ssd-recordings/CHZZK/BrowserCapture").join(RECORDING);
    let archive = PathBuf::from("D:/Astumi/CHZZK/BrowserCapture").join(RECORDING);
    let mut originals = Vec::new();
    for (name, expected, role) in [("recording.json", METADATA_SHA, "METADATA"),
        ("progressive.json", PROGRESSIVE_SHA, "PROGRESSIVE"), ("chat.jsonl", CHAT_SHA, "CHAT")] {
        let item = Original::read(local.join(name), true, 2 * 1024 * 1024)?;
        if item.sha256.as_deref() != Some(expected) { return Err(format!("QA_FIXED_SOURCE_CHANGED_{role}")); }
        originals.push(item);
    }
    let record: BrowserRecording = serde_json::from_reader(originals[0].held.try_clone().map_err(|_| "QA_METADATA_CLONE")?)
        .map_err(|_| "QA_METADATA_JSON")?;
    if record.id != RECORDING || record.status != BrowserRecordingStatus::Stopped || record.preserve_sources
        || record.segment_count != 310 || record.progressive.as_ref().is_none_or(|p| p.part_count != 61 || p.segment_count != 305) {
        return Err("QA_PARENT_STATE_CHANGED".into());
    }
    let native_parts = capture_parts::load(&record).map_err(code)?;
    if native_parts.len() != 61 { return Err("QA_PART_COUNT".into()); }
    for name in ["ssd-ranges.json", "viewer-metrics.jsonl", "channel-profile.json"] {
        originals.push(Original::read(local.join(name), true, 512 * 1024)?);
    }
    let profile: Value = serde_json::from_reader(originals.last().ok_or("QA_PROFILE")?.held.try_clone().map_err(|_| "QA_PROFILE_CLONE")?)
        .map_err(|_| "QA_PROFILE_JSON")?;
    let image = profile["imageAssetId"].as_str().ok_or("QA_PROFILE_ASSET")?;
    if image.len() != 64 || !image.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || local.join("channel-profile-current.json").exists() { return Err("QA_PROFILE_CHANGED".into()); }
    originals.push(Original::read(local.join("replay-assets").join(format!("{image}.json")), true, 2 * 1024 * 1024)?);
    for name in ["ssd-ranges.json", "ssd-archive-owner.json"] {
        originals.push(Original::read(archive.join(name), true, 512 * 1024)?);
    }
    for (index, part) in native_parts.iter().enumerate() {
        for (name, small) in [(&part.file, false), (&part.timeline_file, true), (&part.proof_file, true)] {
            let item = Original::read(archive.join(name), small || index == SELECTED_PART, if small { 256 * 1024 } else { 128 * 1024 * 1024 })?;
            if name == &part.file && item.stamp.len != part.bytes { return Err("QA_PART_SIZE".into()); }
            if index == SELECTED_PART && name == &part.file && item.sha256.as_deref() != Some(MEDIA_SHA) { return Err("QA_SELECTED_HASH".into()); }
            originals.push(item);
        }
    }
    let scratch = audit.join(format!("chzzk-native-readonly-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir(&scratch).map_err(|_| "QA_NEW_SCRATCH")?;
    let before_count = originals.len();
    let outcome: Checked<Value> = (|| {
        let cancel = AtomicBool::new(false);
        // This is the production owner/receipt/range/timeline resolver. No capture
        // store rooted in user data, recovery worker or retention writer is called.
        let held = capture_parts::replay_source_cancellable(&record, Some(&cancel)).map_err(code)?;
        if held.parts.len() != 61 || held.storage_lease.is_some() { return Err("QA_PART_ADMISSION_SCOPE".into()); }
        let (file, part) = &held.parts[SELECTED_PART];
        let selected = crate::cloud_recording::source::pin_file(file).map_err(|_| "QA_SELECTED_PIN")?;
        if digest_file(&selected)? != MEDIA_SHA || (part.duration - 60.00664583333128).abs() > 0.000001
            || (part.offset - 660.1371041666571).abs() > 0.000001 {
            return Err("QA_SELECTED_RANGE".into());
        }
        let start = part.offset;
        // Empty store only supplies ReplayService's constructor dependency. Its
        // source getter is never used; all writes are inside this new scratch.
        let empty = BrowserCaptureStore::new(&scratch).map_err(code)?;
        let service = ReplayService::new(&scratch, Arc::new(Mutex::new(empty)));
        let cache = service.cache_root().map_err(code)?;
        let session = Arc::new(service.make_session(held, &cache).map_err(code)?);
        let status = index::build(&session).map_err(code)?;
        if status.state != "ready" { return Err("QA_INDEX_NOT_READY".into()); }
        let observed_rows = status.observed_rows;
        let approximate_rows = status.approximate_rows;
        *session.index.lock().map_err(|_| "QA_INDEX_LOCK")? = status;
        service.inner.sessions.lock().map_err(|_| "QA_SESSION_LOCK")?.insert(session.token.clone(), session.clone());
        let event_at = start + 18.403003999999783;
        let before = service.chat_at(&session.token, event_at - 0.05, 1, Some(200)).map_err(code)?;
        let after = service.chat_at(&session.token, event_at + 0.05, 2, Some(200)).map_err(code)?;
        let is_selected = |message: &&ReplayChatMessage| message.media_time_seconds >= start
            && message.media_time_seconds < start + 60.00664583333128
            && message.message.rich.as_ref().and_then(|r|r.notice.as_ref()).is_some_and(|n| n.kind == ChatNoticeKind::Donation);
        if before.items.iter().filter(is_selected).count() != 0 { return Err("QA_DONATION_EARLY".into()); }
        let notices: Vec<_> = after.items.iter().filter(is_selected).collect();
        if notices.len() != 1 { return Err("QA_DONATION_COUNT".into()); }
        let notice = notices[0];
        if notice.sync_quality != "observed_media" || (notice.media_time_seconds - event_at).abs() > 0.00001
            || notice.message.rich.as_ref().and_then(|r|r.notice.as_ref()).and_then(|n|n.amount).is_none()
            || notice.message.text.is_empty() { return Err("QA_DONATION_MAPPING".into()); }
        let sequence = notice.message.sequence;
        let mapped = notice.media_time_seconds - start;
        let receive = notice.message.offset_seconds - start;
        let backward = service.chat_at(&session.token, event_at - 0.05, 3, Some(200)).map_err(code)?;
        let replayed = service.chat_at(&session.token, event_at + 0.05, 4, Some(200)).map_err(code)?;
        if backward.items.iter().any(|m|m.message.sequence == sequence)
            || replayed.items.iter().filter(|m|m.message.sequence == sequence).count() != 1 {
            return Err("QA_SEEK_GENERATION".into());
        }
        let paused = service.chat_at(&session.token, event_at + 0.05, 4, Some(200)).map_err(code)?;
        if paused.items.iter().map(|m|m.message.sequence).collect::<Vec<_>>() != replayed.items.iter().map(|m|m.message.sequence).collect::<Vec<_>>() {
            return Err("QA_STATIONARY_CLOCK_CHANGED".into());
        }
        let bytes = bytes_under(&scratch)?;
        if bytes > 32 * 1024 * 1024 { return Err("QA_SCRATCH_LIMIT".into()); }
        service.close(&session.token).map_err(code)?;
        service.shutdown_and_wait();
        Ok(json!({"nativeAllPartsOpened":61,"partIndex":SELECTED_PART,"durationSeconds":60.00664583333128,
            "mediaOffsetSeconds":start,"donationRowsInSelectedWindow":1,"observedMediaPartSeconds":mapped,
            "receiveApproximatePartSeconds":receive,"quality":"observed_media","amountPresent":true,"textPresent":true,
            "beforeAbsent":true,"afterPresent":true,"backwardAbsent":true,"forwardExactlyOnce":true,"stationaryClockStable":true,
            "indexObservedRows":observed_rows,"indexApproximateRows":approximate_rows,"scratchBytesBeforeTeardown":bytes,
            "parentState":"stopped","parentMergeState":"queued","parentComplete":false,"preserveSources":false,
            "fullStoreLeaseAdmission":false,"uploadAttempted":false,"actualUiOrRemoteSync":false}))
    })();
    let unchanged = originals.iter().map(Original::unchanged).collect::<Checked<Vec<_>>>()?.into_iter().all(|v|v);
    let report = json!({"stage":"actual-chzzk-native-part-and-donation-mapper-readonly","recordingId":RECORDING,
        "checkedAtUnixMs":SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_err(|_|"QA_CLOCK")?.as_millis(),
        "result":outcome.as_ref().ok(),"error":outcome.as_ref().err(),"originalFilesPinned":before_count,"allReadOriginalsUnchanged":unchanged,
        "scope":"Existing native partial source/timeline mapper and chat API only; no video playback, upload, account operation or whole recording completion."});
    fs::write(audit.join("chzzk-native-readonly-result.json"), serde_json::to_vec_pretty(&report).map_err(|_|"QA_REPORT_JSON")?)
        .map_err(|_|"QA_REPORT_WRITE")?;
    println!("{}", json!({"stage":"chzzk-native-readonly","passed":outcome.is_ok()&&unchanged,"originalFilesPinned":before_count,"allReadOriginalsUnchanged":unchanged}));
    if !unchanged { return Err("QA_ORIGINAL_CHANGED".into()); }
    outcome.map(|_|())
}
