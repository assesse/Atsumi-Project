//! Real production scheduler + existing IPC, isolated database/profile/media.
//! No production test hook, ordinary account access, input automation or forced exit.
use atsumi_lib::{
    application::ApplicationService, domain::SettingsPatch, infrastructure::SqliteRepository,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !cfg!(debug_assertions) {
        return Err("QA requires a debug build".into());
    }
    let channel = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://www.youtube.com/@KitkatPlayroom".into());
    let url = reqwest::Url::parse(&channel)?;
    let segments: Vec<_> = url
        .path_segments()
        .ok_or("channel URL path required")?
        .collect();
    let channel_path = matches!(segments.as_slice(), [handle] if handle.starts_with('@') && handle.len() > 1)
        || matches!(segments.as_slice(), ["channel", id] if id.starts_with("UC") && id.len() == 24 && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')));
    if url.scheme() != "https"
        || url.host_str() != Some("www.youtube.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !channel_path
    {
        return Err("public channel URL required".into());
    }
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("workspace missing")?
        .to_owned();
    let identity = format!(
        "local.atsumi.youtubeautoqa.{}",
        uuid::Uuid::new_v4().simple()
    );
    let root = workspace
        .join(".runtime/youtube-auto-record-qa")
        .join(&identity);
    // Same new-identity isolation as platform_app_qa. Windows known-folder APIs
    // need not honor an APPDATA override, so resolve the real folder up front.
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("Windows app data unavailable")?)
        .join(&identity);
    if data.exists() {
        return Err("test identity already exists; nothing overwritten".into());
    }
    let media = root.join("media");
    for path in [&data, &media, &root.join("local")] {
        std::fs::create_dir_all(path)?;
    }
    std::env::set_var("LOCALAPPDATA", root.join("local"));
    let repository = Arc::new(SqliteRepository::open(data.join("atsumi-next.sqlite3"))?);
    let service = ApplicationService::new(repository);
    service.settings_update(
        SettingsPatch {
            download_root: Some(media.to_string_lossy().into_owned()),
            privacy_on_startup: Some(false),
            ..Default::default()
        },
        service.settings_get()?.revision,
    )?;
    drop(service);
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let origin = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let script_path = format!("/{nonce}.js");
    let report_path = format!("/{nonce}/report");
    let script = include_str!("youtube_auto_record_qa.js")
        .replace("__CHANNEL__", &serde_json::to_string(&channel)?)
        .replace("__REPORT__", &report_path);
    let html = format!("<!doctype html><meta charset=utf-8><title>Atsumi isolated automatic recording QA</title><p>격리된 YouTube 자동 녹화 검증 중</p><script src='{script_path}'></script>");
    let result = Arc::new(Mutex::new(None::<Value>));
    let recorded = result.clone();
    let proof_media = media.clone();
    let proof_data = data.clone();
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(300);
        let mut served = [false; 3];
        while Instant::now() < deadline {
            let (mut socket, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(30));
                    continue;
                }
                Err(_) => break,
            };
            let _ = socket.set_read_timeout(Some(Duration::from_secs(3)));
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            let mut header_end = None;
            let mut length = 0;
            while request.len() < 16384 {
                match socket.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => request.extend_from_slice(&buffer[..count]),
                }
                if header_end.is_none() {
                    if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                        header_end = Some(end + 4);
                        length = String::from_utf8_lossy(&request[..end])
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(key, _)| key.eq_ignore_ascii_case("Content-Length"))
                                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if length > 8192 {
                            break;
                        }
                    }
                }
                if header_end.is_some_and(|end| request.len() >= end + length) {
                    break;
                }
            }
            let first = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap_or("")
                .to_owned();
            let (status, mime, body) = if first == "GET / HTTP/1.1" {
                report_server_stage(&mut served[0], "main_served");
                ("200 OK", "text/html", html.as_bytes().to_vec())
            } else if first == format!("GET {script_path} HTTP/1.1") {
                report_server_stage(&mut served[1], "script_served");
                (
                    "200 OK",
                    "application/javascript",
                    script.as_bytes().to_vec(),
                )
            } else if first == format!("POST {report_path} HTTP/1.1") && length <= 8192 {
                report_server_stage(&mut served[2], "report_received");
                if let Some(end) = header_end {
                    if let Some(bytes) = request.get(end..end + length) {
                        if let Ok(mut value) = serde_json::from_slice::<Value>(bytes) {
                            if value["kind"] == "youtube_auto_record_qa" {
                                // app_quit exits Tauri directly, so validate here
                                // while all jobs are terminal, before its ACK.
                                let continuity = match check_source_continuity(&proof_media, &proof_data, deadline) {
                                    Ok(value) => value,
                                    Err(_) => json!({"kind":"youtube_auto_continuity_qa","success":false,"evidenceValid":false,"recordingFlowVerified":false,"fullContinuityVerified":false,"error":"Isolated source proof validation failed"}),
                                };
                                println!("{continuity}");
                                combine_report(&mut value, &continuity);
                                println!("{value}");
                                *recorded.lock().unwrap() = Some(value);
                            }
                        }
                    }
                }
                let response = recorded.lock().unwrap().as_ref()
                    .map(|value| json!({"success":value["success"],"recordingFlowVerified":value["recordingFlowVerified"],"fullContinuityVerified":value["fullContinuityVerified"],"sourceContinuity":value["sourceContinuity"],
                        "nativePlaybackVerified":value["nativePlaybackVerified"],"nativeSeekVerified":value["nativeSeekVerified"],"nativeEofReplayVerified":value["nativeEofReplayVerified"]}))
                    .unwrap_or_else(||json!({"success":false}));
                ("200 OK", "application/json", response.to_string().into_bytes())
            } else {
                ("404 Not Found", "text/plain", Vec::new())
            };
            let _ = write!(socket, "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n", body.len());
            let _ = socket.write_all(&body);
        }
        if recorded.lock().unwrap().is_none() {
            eprintln!(
                "{}",
                json!({"kind":"youtube_auto_record_qa","success":false,"stage":"deadline","error":"No bounded QA report; close the isolated app normally"})
            );
        }
    });
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = identity;
    context.config_mut().build.dev_url = Some(origin.parse()?);
    context.config_mut().app.security.csp = None;
    context.config_mut().app.security.dev_csp = None;
    for window in &mut context.config_mut().app.windows {
        window.title = "Atsumi · YouTube 자동 녹화 검증 (격리)".into();
        window.width = 1290.;
        window.height = 720.;
        window.x = Some(-16000.);
        window.y = Some(-16000.);
        window.focus = false;
        window.skip_taskbar = true;
    }
    println!("QA_ISOLATED_ROOT={}", root.display());
    atsumi_lib::run_with_context(context)?;
    let continuity = check_source_continuity(&media, &data, Instant::now() + Duration::from_secs(120))?;
    println!("{continuity}");
    if !result
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|value| value["success"] == true)
    {
        return Err("Actual scheduler flow not verified; inspect bounded report".into());
    }
    if continuity["success"] != true {
        return Err("Saved files passed individually but source continuity failed".into());
    }
    Ok(())
}

fn report_server_stage(seen: &mut bool, stage: &'static str) {
    if !*seen {
        *seen = true;
        println!("{}", json!({"kind":"youtube_auto_record_qa_stage","stage":stage}));
    }
}

// These are independent dimensions: native file/continuity evidence is not
// erased by a decoder timeout, and successful playback cannot create it.
fn combine_report(value: &mut Value, continuity: &Value) {
    let playback = playback_dimensions(value);
    let requested_success = value["success"] == true && value.get("cleanupError").is_none();
    value["fileLevelSuccess"] = json!(value["archiveFlowVerified"] == true);
    value["sourceContinuity"] = continuity.clone();
    value["recordingFlowVerified"] = json!(value["archiveFlowVerified"] == true && continuity["recordingFlowVerified"] == true);
    value["fullContinuityVerified"] = json!(continuity["fullContinuityVerified"] == true);
    value["nativePlaybackVerified"] = json!(playback[0]);
    value["nativeSeekVerified"] = json!(playback[1]);
    value["nativeEofReplayVerified"] = json!(playback[2]);
    value["success"] = json!(requested_success && value["recordingFlowVerified"] == true
        && continuity["success"] == true && playback.into_iter().all(|passed| passed));
}

fn playback_dimensions(value: &Value) -> [bool; 3] {
    let Some(completed) = value["completed"].as_u64().filter(|n| (1..=8).contains(n)) else { return [false; 3] };
    let Some(rows) = value["playback"].as_array() else { return [false; 3] };
    if rows.len() as u64 != completed || value["playbackTested"].as_u64() != Some(completed)
        || value["playbackSkipped"].as_u64() != Some(0) || value["reopened"].as_u64() != Some(completed) {
        return [false; 3];
    }
    let finite = |v: &Value, low: f64, high: f64| v.as_f64().is_some_and(|n| n.is_finite() && n >= low && n <= high);
    let count = |v: &Value, low: u64, high: u64| v.as_u64().is_some_and(|n| n >= low && n <= high);
    let progress = |v: &Value, minimum: f64| finite(&v["seconds"], minimum, 86400.0) && count(&v["frames"], 1, 10_000_000);
    let mut jobs = std::collections::BTreeSet::new();
    let mut all = [true; 3];
    for row in rows {
        let identity = row["jobId"].as_str().is_some_and(|id| uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id) && jobs.insert(id));
        let events = &row["events"];
        let common = identity && row["closed"] == true && row.get("errorCode").is_some_and(Value::is_null)
            && finite(&row["duration"], 4.0, 86400.0) && count(&row["width"], 1, 16384) && count(&row["height"], 1, 16384)
            && matches!(row["frames"].as_str(), Some("decoded" | "nondropped"))
            && count(&events["metadata"], 1, 100000) && count(&events["timeupdate"], 1, 100000);
        let initial = common && count(&events["play"], 1, 100000) && progress(&row["initial"], 1.0);
        let middle = initial && count(&events["play"], 2, 100000) && count(&events["seeked"], 1, 100000)
            && finite(&row["middleSeekError"], 0.0, 0.250) && progress(&row["middle"], 1.0);
        let replay = middle && row["stage"] == "complete" && row["outcome"] == "complete"
            && count(&events["play"], 4, 100000) && count(&events["seeked"], 3, 100000) && count(&events["ended"], 1, 100000)
            && finite(&row["eofError"], 0.0, 0.100) && finite(&row["replaySeekError"], 0.0, 0.250)
            && progress(&row["tail"], f64::MIN_POSITIVE) && progress(&row["replay"], 1.0);
        for (target, passed) in all.iter_mut().zip([initial, middle, replay]) { *target &= passed; }
    }
    all
}

// Read the new isolated run only, after all jobs finalized. File-level success
// must not hide a gap while the same source changes its decoder configuration.
#[cfg(debug_assertions)]
fn check_source_continuity(media: &std::path::Path, data: &std::path::Path, outer_deadline: Instant) -> Result<Value, Box<dyn std::error::Error>> {
    use atsumi_lib::platform_media::{inspect_automatic_raw_qa, MediaJob};
    use std::sync::atomic::AtomicBool;
    let deadline = outer_deadline.min(Instant::now() + Duration::from_secs(120));
    let cancel = AtomicBool::new(false);
    let mut groups: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
    let mut anchors: BTreeMap<String, Vec<(f64, u64)>> = BTreeMap::new();
    let mut raw_groups: BTreeMap<String, Vec<(QaRational, QaRational)>> = BTreeMap::new();
    let mut raw_anchors: BTreeMap<String, Vec<(QaRational,u64)>> = BTreeMap::new();
    let mut scoped: BTreeMap<String, ScopedComparisons> = BTreeMap::new();
    let mut rows = Vec::new();
    let mut files = 0;
    let mut seen = std::collections::BTreeSet::new();
    let mut within_count = 0u64; let mut within_gap = 0.0f64;
    let mut evidence_valid = true; let mut recordings_complete = true;
    for entry in std::fs::read_dir(media.join("YouTube"))?.take(130) {
        if Instant::now() >= deadline { return Err("QA deadline".into()); }
        let entry = entry?;
        if entry.file_name() == ".official" { continue; }
        if files >= 128 || !entry.file_type()?.is_dir() { return Err("Unexpected isolated archive layout".into()); }
        let job_id = entry.file_name().to_str().ok_or("Invalid job filename")?.to_owned();
        if uuid::Uuid::parse_str(&job_id).is_err() || !seen.insert(job_id.clone()) { return Err("Invalid job identity".into()); }
        let job_path = data.join("platform-media/jobs").join(format!("{job_id}.json"));
        let job: MediaJob = serde_json::from_slice(&read_fixed(&job_path,1024*1024)?)?;
        if job.job_id != job_id || job.download_root != media || job.reference.provider != "youtube" || job.reference.kind != "live" {
            return Err("Invalid isolated job binding".into());
        }
        files += 1;
        let raw = match std::fs::symlink_metadata(entry.path().join("proof.json")) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err("Raw proof presence unavailable".into()),
        };
        if job.state != "completed" {
            // Failed/partial work cannot become success through source existence.
            if !matches!(job.state.as_str(), "failed" | "interrupted") { return Err("Nonterminal isolated job".into()); }
            evidence_valid = false; recordings_complete = false;
            rows.push(json!({"jobId":job_id,"format":if raw {"raw"} else {"avc"},"stage":"job_state","state":job.state,"verified":false}));
            continue;
        }
        if raw {
            match inspect_automatic_raw_qa(media, &job, &cancel, deadline) {
                Err(stage) => { evidence_valid=false; recordings_complete=false;
                    rows.push(json!({"jobId":job_id,"format":"raw","verified":false,"stage":stage})); }
                Ok(evidence) => {
                    let start = QaRational::new(evidence.first_tick,evidence.ticks_per_second as i128)?;
                    let end = QaRational::new(evidence.end_tick,evidence.ticks_per_second as i128)?;
                    within_count=within_count.checked_add(evidence.within_job_boundaries).ok_or("Boundary overflow")?;
                    let scope=scoped.entry(evidence.group_key.clone()).or_insert(ScopedComparisons {format:"raw",within:0,gap:0.0});
                    scope.within=scope.within.checked_add(evidence.within_job_boundaries).ok_or("Boundary overflow")?;
                    // A selected range is native-contiguous, but omitted source
                    // intervals cannot certify a boundary to a different job.
                    if evidence.excluded_bytes[0] == 0 && !evidence.source_interrupted && !evidence.partial_tail[0] {
                        raw_groups.entry(evidence.group_key.clone()).or_default().push((start,end));
                        if let (Some(num),Some(den),Some(origin)) = (evidence.common_numerator,evidence.common_denominator,evidence.timeline_origin_ms) {
                            raw_anchors.entry(evidence.group_key.clone()).or_default().push((QaRational::new(num,den)?,origin));
                        }
                    }
                    evidence_valid &= evidence.final_file_verified && evidence.chat_metadata_verified && evidence.timeline_verified;
                    recordings_complete &= evidence.recording_complete;
                    rows.push(json!({"format":"raw","verified":evidence.final_file_verified,"evidence":evidence}));
                }
            }
            continue;
        }
        let read = |name: &str| -> Result<String, Box<dyn std::error::Error>> {
            Ok(String::from_utf8(read_fixed(&entry.path().join(name),1024*1024)?)?)
        };
        let proof: Value = serde_json::from_str(&read("encoded-owner.json")?)?;
        if proof["version"] != 1 || proof["reference"]["provider"] != "youtube"
            || proof["reference"]["kind"] != "live" || proof["owner"]["kind"] != "scheduled"
            || proof["broadcastId"] != proof["reference"]["externalId"]
            || proof["broadcastId"] != proof["owner"]["broadcast_key"]
            || proof["jobId"].as_str() != entry.file_name().to_str()
            || !proof["generation"].as_u64().is_some_and(|n|n>0)
            || !proof["owner"]["generation"].as_u64().is_some_and(|n|n>0)
            || !proof["owner"]["schedule_id"].as_str().is_some_and(|s|s.starts_with("UC")&&s.len()==24)
            || ["receiverId","sourceId","jobId"].iter().any(|key| !proof[*key].as_str().is_some_and(|s|uuid::Uuid::parse_str(s).is_ok()))
            || !proof["owner"]["run_id"].as_str().is_some_and(|s|uuid::Uuid::parse_str(s).is_ok()) {
            return Err("Isolated source owner proof is invalid".into());
        }
        let key = json!([proof["reference"],proof["broadcastId"],proof["receiverId"],proof["sourceId"],proof["generation"],proof["owner"]]).to_string();
        let mut count = 0;
        let mut first_source = f64::INFINITY;
        let mut last_source = 0.0f64;
        let mut local = Vec::new();
        for line in read("encoded-segments.jsonl")?.lines() {
            count += 1;
            if count > 4096 || line.len() > 4096 { return Err("Isolated segment proof limit".into()); }
            let part: Value = serde_json::from_str(line)?;
            let start = part["sourceStart"].as_f64().ok_or("Missing source start")?;
            let end = part["sourceEnd"].as_f64().ok_or("Missing source end")?;
            if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start || end - start > 120.0 {
                return Err("Invalid source interval".into());
            }
            local.push((start,end));
            first_source = first_source.min(start);
            last_source = last_source.max(end);
        }
        if count == 0 { return Err("Empty isolated segment proof".into()); }
        let (local_count,local_gap)=maximum_source_gap(&mut BTreeMap::from([(key.clone(),local)]));
        within_count=within_count.checked_add(local_count as u64).ok_or("Boundary overflow")?;
        within_gap=within_gap.max(local_gap);
        let scope=scoped.entry(key.clone()).or_insert(ScopedComparisons {format:"avc",within:0,gap:0.0});
        scope.within=scope.within.checked_add(local_count as u64).ok_or("Boundary overflow")?;
        scope.gap=scope.gap.max(local_gap);
        groups.entry(key.clone()).or_default().push((first_source,last_source));
        let timeline: Value = serde_json::from_str(&read("chat-timeline.json")?)?;
        let committed = timeline["firstSegmentCommittedAtMs"].as_u64().ok_or("Missing chat commit time")?;
        let duration = timeline["firstSegmentDurationMs"].as_u64().ok_or("Missing chat first duration")?;
        if timeline["version"] != 1 || timeline["provider"] != "youtube"
            || timeline["timelineQuality"] != "receive-time-approximate"
            || !(1..=120000).contains(&duration) || !(1_000_000_000_000..=9_000_000_000_000).contains(&committed) {
            return Err("Invalid approximate chat timeline".into());
        }
        anchors.entry(key).or_default().push((first_source,committed-duration));
        rows.push(json!({"jobId":job_id,"format":"avc","verified":true,"segments":count,"firstSource":first_source,"lastSource":last_source}));
    }
    if files == 0 { return Err("No isolated final source proof".into()); }
    // No hidden extra job may be silently skipped by the media enumeration.
    for entry in std::fs::read_dir(data.join("platform-media/jobs"))?.take(130) {
        let entry=entry?; let name=entry.file_name(); let name=name.to_str().ok_or("Invalid job name")?;
        if let Some(id)=name.strip_suffix(".json") {
            if uuid::Uuid::parse_str(id).is_err() || !seen.contains(id) { return Err("Unmatched isolated job journal".into()); }
        } else { return Err("Unexpected isolated job evidence".into()); }
    }
    let (cross_count,cross_gap)=maximum_source_gap(&mut groups);
    let (raw_count,raw_gap)=raw_source_gap(&mut raw_groups)?;
    let chat_count=anchors.values().map(|v|v.len().saturating_sub(1) as u64).sum::<u64>();
    let (raw_chat_count,raw_drift)=raw_chat_drift(&raw_anchors)?;
    let within=dimension(within_count,within_gap,0.250);
    let cross=dimension(cross_count as u64+raw_count,cross_gap.max(raw_gap),0.250);
    let chat=dimension(chat_count+raw_chat_count,maximum_chat_drift(&anchors).max(raw_drift),1000.0);
    // Aggregate counts are summaries only: another source's comparisons must
    // never hide a raw singleton (or excluded range) without boundary evidence.
    let mut scope_rows=Vec::new();
    for (at,(key,scope)) in scoped.iter().enumerate() {
        let (count,gap,chat_count,drift)=if scope.format=="raw" {
            let mut one=BTreeMap::from([(key.clone(),raw_groups.get(key).cloned().unwrap_or_default())]);
            let (count,gap)=raw_source_gap(&mut one)?;
            let (chat_count,drift)=raw_chat_drift(&BTreeMap::from([(key.clone(),raw_anchors.get(key).cloned().unwrap_or_default())]))?;
            (count,gap,chat_count,drift)
        } else {
            let mut one=BTreeMap::from([(key.clone(),groups.get(key).cloned().unwrap_or_default())]);
            let (count,gap)=maximum_source_gap(&mut one);
            let values=anchors.get(key).cloned().unwrap_or_default();
            let chat_count=values.len().saturating_sub(1) as u64;
            let drift=maximum_chat_drift(&BTreeMap::from([(key.clone(),values)]));
            (count as u64,gap,chat_count,drift)
        };
        scope_rows.push(group_dimensions(at+1,scope.format,dimension(scope.within,scope.gap,0.250),
            dimension(count,gap,0.250),dimension(chat_count,drift,1000.0)));
    }
    let full=evidence_valid && all_groups_verified(&scope_rows);
    let flow=evidence_valid && recordings_complete;
    Ok(json!({"kind":"youtube_auto_continuity_qa","success":flow&&full,"evidenceValid":evidence_valid,
        "recordingFlowVerified":flow,"fullContinuityVerified":full,"files":files,
        "withinJob":within,"crossJob":cross,"approximateChatDrift":chat,"groups":scope_rows,"jobs":rows}))
}

#[cfg(not(debug_assertions))]
fn check_source_continuity(_: &std::path::Path, _: &std::path::Path, _: Instant) -> Result<Value,Box<dyn std::error::Error>> {
    Err("QA requires a debug build".into())
}
fn read_fixed(path:&std::path::Path,cap:u64)->Result<Vec<u8>,Box<dyn std::error::Error>> {
    let meta=std::fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len()==0 || meta.len()>cap { return Err("Invalid fixed proof".into()); }
    #[cfg(windows)] { use std::os::windows::fs::MetadataExt; if meta.file_attributes()&0x400!=0 { return Err("Reparse proof".into()); } }
    let mut options=std::fs::OpenOptions::new(); options.read(true);
    #[cfg(windows)] { use std::os::windows::fs::OpenOptionsExt; options.share_mode(1).custom_flags(0x00200000); }
    let mut file=options.open(path)?; let before=file.metadata()?;
    if before.len()!=meta.len() { return Err("Changed proof".into()); }
    let mut bytes=Vec::new(); Read::by_ref(&mut file).take(cap+1).read_to_end(&mut bytes)?;
    if bytes.len() as u64!=meta.len() || file.metadata()?.len()!=meta.len() { return Err("Changed proof".into()); }
    Ok(bytes)
}
fn dimension(count:u64,maximum:f64,limit:f64)->Value {
    if count==0 { json!({"state":"not_verified","comparisons":0,"maximum":null}) }
    else { json!({"state":if maximum<=limit {"verified"} else {"failed"},"comparisons":count,"maximum":maximum}) }
}
struct ScopedComparisons { format:&'static str,within:u64,gap:f64 }
fn group_dimensions(ordinal:usize,format:&str,within:Value,cross:Value,chat:Value)->Value {
    let verified=within["state"]=="verified" && cross["state"]=="verified" && chat["state"]=="verified";
    json!({"group":ordinal,"format":format,"fullContinuityVerified":verified,
        "withinJob":within,"crossJob":cross,"approximateChatDrift":chat})
}
fn all_groups_verified(groups:&[Value])->bool {
    !groups.is_empty() && groups.iter().all(|group|group["fullContinuityVerified"]==true)
}
#[derive(Clone,Copy)]
struct QaRational { n:i128,d:i128 }
impl QaRational {
    fn new(n:i128,d:i128)->Result<Self,Box<dyn std::error::Error>> {
        if n<0 || !(1..=1_000_000_000).contains(&d) || n>i128::MAX/1_000_000_000 { return Err("Bounded source rational required".into()); }
        Ok(Self {n,d})
    }
    fn compare(self,other:Self)->std::cmp::Ordering { (self.n*other.d).cmp(&(other.n*self.d)) }
    fn difference(self,other:Self)->f64 { ((self.n*other.d)-(other.n*self.d)) as f64/(self.d*other.d) as f64 }
}
fn raw_source_gap(groups:&mut BTreeMap<String,Vec<(QaRational,QaRational)>>)->Result<(u64,f64),Box<dyn std::error::Error>> {
    let mut count=0;let mut gap=0.0f64;
    for values in groups.values_mut() {
        values.sort_by(|a,b|a.0.compare(b.0)); let mut previous=None::<QaRational>;
        for &(start,end) in values.iter() {
            if start.compare(end)!=std::cmp::Ordering::Less { return Err("Invalid source interval".into()); }
            if let Some(old)=previous { count+=1; gap=gap.max(start.difference(old)); }
            previous=Some(previous.map_or(end,|old|if old.compare(end).is_gt(){old}else{end}));
        }
    }
    Ok((count,gap))
}
fn raw_chat_drift(groups:&BTreeMap<String,Vec<(QaRational,u64)>>)->Result<(u64,f64),Box<dyn std::error::Error>> {
    let mut count=0;let mut drift=0.0f64;
    for values in groups.values() {
        if let Some(&(first,wall))=values.first() {
            for &(next,next_wall) in values.iter().skip(1) { count+=1;
                drift=drift.max(((i128::from(next_wall)-i128::from(wall)) as f64-next.difference(first)*1000.0).abs()); }
        }
    }
    Ok((count,drift))
}

fn maximum_chat_drift(groups: &BTreeMap<String, Vec<(f64,u64)>>) -> f64 {
    let mut maximum=0.0f64;
    for group in groups.values() {
        if let Some(&(source,origin))=group.first() {
            for &(next_source,next_origin) in group.iter().skip(1) {
                let actual=(i128::from(next_origin)-i128::from(origin)) as f64;
                maximum=maximum.max((actual-(next_source-source)*1000.0).abs());
            }
        }
    }
    maximum
}

fn maximum_source_gap(groups: &mut BTreeMap<String, Vec<(f64,f64)>>) -> (usize,f64) {
    let mut comparisons=0; let mut gap=0.0f64;
    for intervals in groups.values_mut() {
        intervals.sort_by(|a,b|a.0.total_cmp(&b.0));
        let mut previous_end=None::<f64>;
        for &(start,end) in intervals.iter() {
            if let Some(previous)=previous_end { comparisons+=1;gap=gap.max(start-previous); }
            previous_end=Some(previous_end.map_or(end,|previous|previous.max(end)));
        }
    }
    (comparisons,gap)
}

#[cfg(test)]
mod playback_tests {
    use super::*;

    fn report() -> Value {
        json!({"success":true,"archiveFlowVerified":true,"completed":1,"reopened":1,
            "playbackTested":1,"playbackSkipped":0,"playback":[{
                "jobId":"12345678-1234-1234-1234-123456789abc","stage":"complete","outcome":"complete",
                "duration":20.0,"width":854,"height":480,"frames":"decoded","errorCode":null,"closed":true,
                "events":{"metadata":1,"play":4,"timeupdate":20,"seeked":3,"ended":1},
                "initial":{"seconds":1.05,"frames":31},"middle":{"seconds":1.05,"frames":31},
                "tail":{"seconds":1.5,"frames":45},"replay":{"seconds":1.05,"frames":31},
                "middleSeekError":0.0,"replaySeekError":0.0,"eofError":0.0
            }]})
    }
    fn continuity() -> Value {
        json!({"success":true,"recordingFlowVerified":true,"fullContinuityVerified":true})
    }

    #[test]
    fn actual_playback_metrics_and_independent_native_dimensions_are_required() {
        let mut value = report();
        assert_eq!(playback_dimensions(&value), [true; 3]);
        combine_report(&mut value, &continuity());
        assert_eq!(value["success"], true);
        let mut value = report();
        combine_report(&mut value, &json!({"success":false,"recordingFlowVerified":false,"fullContinuityVerified":false}));
        assert_eq!(value["nativeEofReplayVerified"], true);
        assert_eq!(value["recordingFlowVerified"], false);
        assert_eq!(value["success"], false);
    }

    #[test]
    fn playback_failure_preserves_prior_recording_and_continuity_evidence() {
        let mut value = report();
        value["success"] = json!(false);
        value["playback"][0]["stage"] = json!("replay");
        value["playback"][0]["outcome"] = json!("replay_timeout");
        value["playback"][0]["replay"] = Value::Null;
        combine_report(&mut value, &continuity());
        assert_eq!(value["fileLevelSuccess"], true);
        assert_eq!(value["recordingFlowVerified"], true);
        assert_eq!(value["fullContinuityVerified"], true);
        assert_eq!(value["nativePlaybackVerified"], true);
        assert_eq!(value["nativeSeekVerified"], true);
        assert_eq!(value["nativeEofReplayVerified"], false);
        assert_eq!(value["success"], false);
    }

    #[test]
    fn zero_skipped_duplicate_or_missing_file_rows_cannot_verify_playback() {
        for (key, replacement) in [("completed",json!(0)),("completed",json!(9)),("playbackTested",json!(0)),
            ("playbackSkipped",json!(1)),("reopened",json!(0)),("playback",json!([]))] {
            let mut value = report(); value[key] = replacement;
            assert_eq!(playback_dimensions(&value), [false;3], "{key}");
        }
        let mut value = report();
        let duplicate = value["playback"][0].clone();
        value["playback"].as_array_mut().unwrap().push(duplicate);
        for key in ["completed","playbackTested","reopened"] { value[key]=json!(2); }
        assert_eq!(playback_dimensions(&value), [false;3]);
        value["playback"][1]["jobId"]=json!("23456789-1234-1234-1234-123456789abc");
        assert_eq!(playback_dimensions(&value), [true;3]);
    }

    #[test]
    fn bad_metadata_or_cleanup_cannot_be_overridden_by_reported_flags() {
        for (key, replacement) in [("duration",json!(0)),("duration",Value::Null),("duration",json!(3.99)),
            ("width",json!(0)),("height",json!(16385)),("closed",json!(false)),("errorCode",json!(3)),
            ("frames",Value::Null),("jobId",json!("not-a-job"))] {
            let mut value=report(); value["playback"][0][key]=replacement;
            value["nativePlaybackVerified"]=json!(true);
            combine_report(&mut value,&continuity());
            assert_eq!(value["nativePlaybackVerified"],false,"{key}");
            assert_eq!(value["recordingFlowVerified"],true);
            assert_eq!(value["success"],false);
        }
        let mut value=report(); value["cleanupError"]=json!("playback close failed");
        combine_report(&mut value,&continuity());
        assert_eq!(value["success"],false);
        assert_eq!(value["recordingFlowVerified"],true);
    }

    #[test]
    fn play_seek_and_ended_events_without_fresh_time_and_frames_do_not_pass() {
        for pointer in ["/events/metadata","/events/play","/events/timeupdate","/initial/frames","/initial/seconds"] {
            let mut value=report(); *value["playback"][0].pointer_mut(pointer).unwrap()=json!(0);
            assert_eq!(playback_dimensions(&value),[false;3],"{pointer}");
        }
        for pointer in ["/events/seeked","/middle/frames","/middle/seconds"] {
            let mut value=report(); *value["playback"][0].pointer_mut(pointer).unwrap()=json!(0);
            assert_eq!(playback_dimensions(&value),[true,false,false],"{pointer}");
        }
        for pointer in ["/events/ended","/tail/frames","/tail/seconds","/replay/frames","/replay/seconds"] {
            let mut value=report(); *value["playback"][0].pointer_mut(pointer).unwrap()=json!(0);
            assert_eq!(playback_dimensions(&value),[true,true,false],"{pointer}");
        }
        for (key,error,expected) in [("middleSeekError",0.251,[true,false,false]),
            ("replaySeekError",0.251,[true,true,false]),("eofError",0.101,[true,true,false])] {
            let mut value=report(); value["playback"][0][key]=json!(error);
            assert_eq!(playback_dimensions(&value),expected);
        }
    }
}

#[cfg(test)]
mod continuity_tests {
    use super::*;
    #[test] fn detects_actual_ten_second_format_gap() {
        let mut groups=BTreeMap::from([("same-owner-source".into(),vec![(46810.00033333333,46815.00469387755),(46825.00033333333,46840.01258503401),(46840.00033333333,46855.00033333333)])]);
        let (count,gap)=maximum_source_gap(&mut groups);
        assert_eq!(count,2);assert!((gap-9.99563945578).abs()<0.00001);
    }
    #[test] fn allows_contiguous_overlap_and_never_compares_unrelated_sources() {
        let mut groups=BTreeMap::from([("a".into(),vec![(15.0,30.0),(0.0,15.01)]),("b".into(),vec![(300.0,310.0)])]);
        assert_eq!(maximum_source_gap(&mut groups),(1,0.0));
    }
    #[test] fn approximate_chat_drift_reports_actual_eight_ms_and_detects_delayed_commit() {
        let mut groups=BTreeMap::from([("same".into(),vec![(46810.00033333333,1792008215568),(46815.00033333333,1792008220576)]),("other".into(),vec![(1.0,1700000000000)])]);
        assert!((maximum_chat_drift(&groups)-8.0).abs()<0.001);
        groups.get_mut("same").unwrap()[1].1+=10_000;
        assert!((maximum_chat_drift(&groups)-10_008.0).abs()<0.001);
    }
    #[test] fn no_comparisons_is_unverified_not_zero_gap_success() {
        let value=dimension(0,0.0,0.250);assert_eq!(value["state"],"not_verified");assert!(value["maximum"].is_null());
        assert_eq!(dimension(1,0.0,0.250)["state"],"verified");assert_eq!(dimension(1,10.0,0.250)["state"],"failed");
    }
    #[test] fn raw_rational_ranges_and_common_origin_do_not_mix_sources() {
        let q=|n,d|QaRational::new(n,d).unwrap();
        let mut groups=BTreeMap::from([("raw-a".into(),vec![(q(10,1),q(15,1)),(q(25,1),q(30,1))]),
            ("raw-b".into(),vec![(q(100,1),q(105,1))])]);
        assert_eq!(raw_source_gap(&mut groups).unwrap(),(1,10.0));
        let anchors=BTreeMap::from([("raw-a".into(),vec![(q(10000,1000),1_700_000_000_000),(q(15000,1000),1_700_000_005_008)]),
            ("raw-b".into(),vec![(q(0,1),1_800_000_000_000)])]);
        assert_eq!(raw_chat_drift(&anchors).unwrap(),(1,8.0));
        assert!(QaRational::new(i128::MAX,1).is_err());assert!(QaRational::new(0,0).is_err());
    }
    #[test] fn avc_comparisons_cannot_hide_a_raw_singletons_missing_evidence() {
        let verified=|count,limit|dimension(count,0.0,limit);
        let avc=group_dimensions(1,"avc",verified(2,0.250),verified(1,0.250),verified(1,1000.0));
        let raw=group_dimensions(2,"raw",verified(3,0.250),verified(0,0.250),verified(0,1000.0));
        assert_eq!(raw["crossJob"]["state"],"not_verified");
        assert!(raw["approximateChatDrift"]["maximum"].is_null());
        assert!(!all_groups_verified(&[avc.clone(),raw]));
        let complete=group_dimensions(2,"raw",verified(3,0.250),verified(1,0.250),verified(1,1000.0));
        assert!(all_groups_verified(&[avc.clone(),complete]));
        let failed=group_dimensions(2,"raw",verified(3,0.250),dimension(1,10.0,0.250),verified(1,1000.0));
        assert!(!all_groups_verified(&[avc,failed]));assert!(!all_groups_verified(&[]));
    }
}
