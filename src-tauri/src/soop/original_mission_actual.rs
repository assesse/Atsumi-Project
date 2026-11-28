//! Ignored child of original_appearance_actual.rs. Actual public receive only;
//! no synthetic/gift/login/desktop actions. Render comparison remains separate.
use super::*;
use crate::platform_media::PlatformMediaTools;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Write};

const WATCH: &str = r#"(()=>{
 const area=document.getElementById('chat_area');if(!area)return {ready:false,rows:[]};
 const key='__atsumiMissionObservationV1';let s=window[key];
 if(!s){s={seen:new WeakSet(),sequence:0};window[key]=s;for(const n of area.querySelectorAll('.chatting-list-item'))s.seen.add(n);return {ready:true,baseline:true,rows:[]}}
 const fresh=[...area.querySelectorAll('.chatting-list-item')].filter(n=>!s.seen.has(n));for(const n of fresh)s.seen.add(n);
 const clean=(t,max)=>typeof t==='string'?[...t].slice(0,max).join(''):'';
 const shape=n=>{if(!n)return null;const c=getComputedStyle(n),r=n.getBoundingClientRect();return {color:c.color,background:c.backgroundColor,fontSize:c.fontSize,lineHeight:c.lineHeight,fontWeight:c.fontWeight,x:r.x,y:r.y,width:r.width,height:r.height,classes:[...n.classList].slice(0,10)}};
 const classify=r=>{const money=r.querySelector('.money')?.textContent??'',title=r.querySelector('.bubble-container .title')?.textContent??'';
   if(/대결미션|도전미션/.test(money+title))return 'mission';if(r.querySelector('.donation-container'))return 'gift';if(r.querySelector('.emoticon-box'))return 'ogq';return r.querySelector('.message-container .msg')?'ordinary':'other'};
 const rows=fresh.map(r=>({r,category:classify(r)})).filter(x=>x.category!=='other');
 rows.sort((a,b)=>Number(b.category==='mission')-Number(a.category==='mission'));
 return {ready:true,truncated:rows.length>6,rows:rows.slice(0,6).map(({r,category})=>{
   const name=r.querySelector('.info-box .name,.username .author'),msg=r.querySelector('.money,.bubble-container .msg,.message-text .msg'),title=r.querySelector('.bubble-container .title');
   const images=[...r.querySelectorAll('.ceremony-img img,.emoticon-box img,.username .thumb img')].slice(0,4).map(i=>({width:i.naturalWidth,height:i.naturalHeight,loaded:i.complete&&i.naturalWidth>0,rect:shape(i)}));
   const countText=msg?.querySelector('em')?.textContent??'';const count=/^\s*\d+(?:개)?\s*$/.test(countText)?Number(countText.replace(/\D/g,'')):null;
   return {ordinal:++s.sequence,at:Date.now(),category,pixelRatio:devicePixelRatio,rowStyle:shape(r),author:clean(name?.textContent,160),text:clean(msg?.textContent,1024),count,title:clean(title?.textContent,32),authorStyle:shape(name),messageStyle:shape(msg),bubbleStyle:shape(r.querySelector('.donation-bubble,.bubble-container')),images};})};
})()"#;

fn capture_original(app: &AppHandle, path: &Path) -> Result<(), String> {
    use webview2_com::{
        CapturePreviewCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
    };
    use windows::Win32::{
        System::Com::{STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET},
        UI::Shell::SHCreateMemStream,
    };
    let views: Vec<_> = app
        .webviews()
        .into_values()
        .filter(|v| v.label().starts_with("soop-original-"))
        .collect();
    if views.len() != 1 {
        return Err("capture_source_count".into());
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    views[0]
        .with_webview(move |platform| unsafe {
            let Some(stream) = SHCreateMemStream(None) else {
                let _ = tx.try_send(Err("capture_stream"));
                return;
            };
            let data = stream.clone();
            let done = tx.clone();
            let callback = CapturePreviewCompletedHandler::create(Box::new(move |status| {
                let result = (|| -> Result<Vec<u8>, &'static str> {
                    status.map_err(|_| "capture_failed")?;
                    let mut stat = STATSTG::default();
                    data.Stat(&mut stat, STATFLAG_NONAME)
                        .map_err(|_| "capture_stat")?;
                    if stat.cbSize == 0 || stat.cbSize > 16 * 1024 * 1024 {
                        return Err("capture_bound");
                    }
                    data.Seek(0, STREAM_SEEK_SET, None)
                        .map_err(|_| "capture_seek")?;
                    let mut bytes = vec![0; stat.cbSize as usize];
                    let mut n = 0;
                    data.Read(bytes.as_mut_ptr().cast(), bytes.len() as u32, Some(&mut n))
                        .ok()
                        .map_err(|_| "capture_read")?;
                    if n as usize != bytes.len() || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                        return Err("capture_png");
                    }
                    Ok(bytes)
                })();
                let _ = done.try_send(result);
                Ok(())
            }));
            if platform
                .controller()
                .CoreWebView2()
                .and_then(|c| {
                    c.CapturePreview(
                        COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                        &stream,
                        &callback,
                    )
                })
                .is_err()
            {
                let _ = tx.try_send(Err("capture_dispatch"));
            }
        })
        .map_err(|_| "capture_dispatch")?;
    let bytes = rx
        .recv_timeout(Duration::from_secs(6))
        .map_err(|_| "capture_timeout")?
        .map_err(str::to_owned)?;
    no_links(path)?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| "capture_create")?;
    file.write_all(&bytes).map_err(|_| "capture_write".into())
}
fn canonical(v: &str) -> String {
    v.chars().filter(|c| !c.is_whitespace()).collect()
}
fn signature(author: &str, text: &str, salt: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!(
            "{salt}\0{}\0{}",
            canonical(author),
            canonical(text)
        ))
    )
}
fn observed_group(v: &Value) -> &str {
    v["category"].as_str().unwrap_or("other")
}
// WebView2 eval may serialize Date.now()/counts as JSON 123.00. Accept only
// exact JS-safe nonnegative integers; never round fractional observations.
fn observed_integer(value: &Value) -> Option<u64> {
    const MAX: u64 = 9_007_199_254_740_991;
    if let Some(n) = value.as_u64() {
        return (n <= MAX).then_some(n);
    }
    let n = value.as_f64()?;
    (n.is_finite() && n >= 0.0 && n <= MAX as f64 && n.fract() == 0.0)
        .then_some(n as u64)
}
fn match_signature(v: &Value, dom: bool, salt: &str) -> String {
    let text = if dom && observed_group(v) == "gift" && observed_integer(&v["count"]).is_some() {
        format!("gift-count:{}", observed_integer(&v["count"]).unwrap())
    } else if !dom && v["soopGift"]["count"].as_u64().is_some() {
        format!("gift-count:{}", v["soopGift"]["count"])
    } else {
        v["text"].as_str().unwrap_or("").to_owned()
    };
    signature(v["author"].as_str().unwrap_or(""), &text, salt)
}
fn owned_bytes(root: &Path) -> Result<u64, String> {
    let mut paths = vec![(root.to_path_buf(), 0)];
    let (mut count, mut total) = (0, 0u64);
    while let Some((path, depth)) = paths.pop() {
        if depth > 6 {
            return Err("tree_depth".into());
        }
        no_links(&path)?;
        for entry in fs::read_dir(path).map_err(|_| "tree_read")? {
            let entry = entry.map_err(|_| "tree_entry")?;
            count += 1;
            if count > 4096 {
                return Err("tree_count".into());
            }
            no_links(&entry.path())?;
            let metadata = entry.metadata().map_err(|_| "tree_metadata")?;
            if metadata.is_dir() {
                paths.push((entry.path(), depth + 1));
            } else if metadata.is_file() {
                total = total.checked_add(metadata.len()).ok_or("tree_overflow")?;
            } else {
                return Err("tree_type".into());
            }
        }
    }
    Ok(total)
}

fn saved_evidence(
    media: &PlatformMediaService,
    root: &Path,
    job: &str,
    observations: &[Value],
    report: &mut Value,
) -> Result<(), String> {
    let salt = root
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or("run_identity")?;
    let completed = media
        .jobs(Some("soop"))
        .into_iter()
        .find(|v| v.job_id == job)
        .ok_or("job_missing")?;
    if completed.state != "completed" {
        return Err("job_not_completed".into());
    }
    let library = completed.library.as_ref().ok_or("library_missing")?;
    let directory = root.join("media/SOOP").join(job);
    let timeline: Value =
        serde_json::from_slice(&read(&directory.join("chat-timeline.json"), 65536)?)
            .map_err(|_| "timeline_json")?;
    let origin = timeline["firstSegmentCommittedAtMs"]
        .as_u64()
        .zip(timeline["firstSegmentDurationMs"].as_u64())
        .and_then(|(end, d)| end.checked_sub(d))
        .ok_or("timeline_origin")?;
    let mut service_counts = BTreeMap::new();
    let mut source_bytes = 0;
    for i in 0..4 {
        let path = directory.join(if i == 0 {
            "chat.source.jsonl".into()
        } else {
            format!("chat.source.{i:06}.jsonl")
        });
        if !path.exists() {
            break;
        }
        let bytes = read(&path, 8 * 1024 * 1024)?;
        source_bytes += bytes.len();
        if source_bytes > 16 * 1024 * 1024 {
            return Err("source_budget".into());
        }
        for line in bytes.split(|b| *b == b'\n').filter(|v| !v.is_empty()) {
            if line.len() > 512 * 1024 {
                return Err("line_bound".into());
            }
            let v: Value = serde_json::from_slice(line).map_err(|_| "source_json")?;
            if let Some(service) = v["extra"]["service"].as_u64() {
                *service_counts.entry(service).or_insert(0u64) += 1;
            }
        }
    }
    report["sourceServices"] = json!(service_counts);
    report["mission121Observed"] = json!(service_counts.get(&121).copied().unwrap_or(0) > 0);
    let mut chosen: Vec<_> = observations
        .iter()
        .filter(|v| observed_group(v) == "mission")
        .take(12)
        .collect();
    for category in ["ordinary", "ogq", "gift"] {
        chosen.extend(
            observations
                .iter()
                .filter(|v| observed_group(v) == category)
                .take(2),
        );
    }
    let mut samples = Vec::new();
    let mut ids = BTreeSet::new();
    let mut correlated = BTreeMap::<String, u64>::new();
    for observed in chosen {
        let at = observed_integer(&observed["at"]).ok_or("observation_clock")?;
        if at < origin {
            continue;
        }
        let pos = at
            .saturating_sub(origin)
            .min((library.duration * 1000.).max(0.) as u64);
        let p = page(media, job, pos.saturating_add(1000))?;
        let expected = match_signature(observed, true, salt);
        let matches: Vec<_> = p["items"]
            .as_array()
            .ok_or("replay_items")?
            .iter()
            .filter(|r| {
                r["timeMs"]
                    .as_u64()
                    .is_some_and(|t| t.abs_diff(pos) <= 3000)
                    && match_signature(r, false, salt) == expected
            })
            .collect();
        let unique_dom = observations
            .iter()
            .filter(|o| {
                observed_integer(&o["at"]).is_some_and(|t| t.abs_diff(at) <= 3000)
                    && match_signature(o, true, salt) == expected
            })
            .count()
            == 1;
        let unique = matches.len() == 1 && unique_dom;
        if unique {
            *correlated
                .entry(observed_group(observed).into())
                .or_default() += 1;
            asset_ids(matches[0], &mut ids);
        }
        // OGQ visible text can be empty. Export nearby actual candidates without
        // treating temporal proximity alone as an event identity match.
        for r in p["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| {
                r["timeMs"]
                    .as_u64()
                    .is_some_and(|t| t.abs_diff(pos) <= 3000)
                    && r["soopEmoticon"].is_object()
            })
            .take(4)
        {
            asset_ids(r, &mut ids);
        }
        if ids.len() > 64 {
            return Err("asset_count".into());
        }
        samples.push(
            json!({"positionMs":pos,"official":observed,"uniqueSignatureWithin3s":unique,"page":p}),
        );
    }
    let mut assets = serde_json::Map::new();
    let mut total = 0;
    let mut ready = 0;
    for id in ids {
        let a = media
            .replay_chat_asset(job, &id)
            .map_err(|_| "local_asset_read")?;
        total += a.data_url.as_ref().map_or(0, String::len);
        if total > 8 * 1024 * 1024 {
            return Err("asset_bytes".into());
        }
        ready += usize::from(a.state == "ready");
        assets.insert(id, json!({"state":a.state,"dataUrl":a.data_url}));
    }
    // Independent official computed palette vs saved production replay state,
    // even when no public service121 happens during the bounded observation.
    let last = page(media, job, (library.duration * 1000.).max(0.) as u64)?;
    let captured = report["lastOfficialAppearance"].clone();
    report["paletteCompared"] = json!(
        captured["palette"].is_object() && last["soopDisplay"]["palette"] == captured["palette"]
    );
    report["surfaceCompared"] = json!(
        captured["surface"].is_object() && last["soopDisplay"]["surface"] == captured["surface"]
    );
    report["settingsCompared"] = json!(last["soopDisplay"]["values"] == values(&captured));
    report["signatureCorrelations"] = json!(correlated);
    report["localAssetsReadable"] = json!(ready);
    report["officialImageByteEqualityVerified"] = json!(false);
    report["replayRenderedComparisonVerified"] = json!(false);
    report["matchQualification"]=json!("Unique public display signature (gift author/count) and receive-time window only; no shared server event ID is available. OGQ proximity candidates are not event-identity proof.");
    write(
        &root.join("actual-replay-export.json"),
        &json!({"actual":true,"jobId":job,"durationSeconds":library.duration,"samples":samples,"lastPage":last,"assets":assets,"renderComparisonPending":true}),
    )?;
    Ok(())
}

fn observe_owned(
    app: &AppHandle,
    media: &PlatformMediaService,
    root: &Path,
    channel: &str,
    report: &mut Value,
) -> Result<(), String> {
    if fs2::available_space(root).map_err(|_| "disk_space")? < 2 * 1024 * 1024 * 1024 {
        return Err("disk_reserve".into());
    }
    let host = app.state::<SoopOriginalHost>();
    host.set_tray_muted(app, true);
    let opened = host
        .open(
            app,
            media.clone(),
            channel.into(),
            uuid::Uuid::new_v4().to_string(),
            root.join("media"),
        )
        .map_err(|_| "public_source_unavailable")?;
    let token = opened.token;
    let broadcast = opened.broadcast_id;
    if opened.channel_id != channel || broadcast.is_empty() {
        return Err("source_identity".into());
    }
    report["sourceBroadcast"] = json!(broadcast);
    host.viewport(app,&token,serde_json::from_value(json!({"sequence":1,"player":{"x":0,"y":0,"width":960,"height":720},"chat":{"x":970,"y":0,"width":320,"height":720},"visible":true,"muted":true,"occlusions":[]})).map_err(|_|"viewport_shape")?).map_err(|_|"viewport")?;
    let ready = Instant::now() + Duration::from_secs(25);
    loop {
        let s = host.snapshot(&token).map_err(|_| "source_snapshot")?;
        if s.channel_id != channel || s.broadcast_id != broadcast {
            return Err("source_changed".into());
        }
        if s.capture_ready
            && s.chat_visible
            && observe(app)?["video"]["ready"].as_u64().unwrap_or(0) >= 3
        {
            break;
        }
        if Instant::now() >= ready {
            return Err("source_not_ready_publicly".into());
        }
        thread::sleep(Duration::from_millis(250));
    }
    host.set_tray_muted(app, false);
    host.resume(app, &token).map_err(|_| "source_resume")?;
    mute_own_video(app)?;
    let initial_time = observe(app)?["video"]["time"]
        .as_f64()
        .ok_or("source_clock")?;
    loop {
        let s = host.snapshot(&token).map_err(|_| "source_snapshot")?;
        if s.channel_id != channel || s.broadcast_id != broadcast {
            return Err("source_changed".into());
        }
        let observed = observe(app)?;
        require_muted(&observed)?;
        if s.state == "playing"
            && s.capture_ready
            && observed["video"]["time"]
                .as_f64()
                .is_some_and(|t| t > initial_time + 0.15)
        {
            break;
        }
        if Instant::now() >= ready {
            return Err("source_not_progressing_publicly".into());
        }
        thread::sleep(Duration::from_millis(250));
    }
    report["publicPlaybackProgressVerified"] = json!(true);
    let baseline = eval(app, WATCH)?;
    if baseline["ready"] != true {
        return Err("chat_dom_missing".into());
    }
    host.record(app, &token, true).map_err(|_| "record_start")?;
    let begin = Instant::now();
    let mut job = None;
    let mut observations = Vec::new();
    let mut shots = 0;
    let mut clipped = false;
    while begin.elapsed() < Duration::from_secs(90) {
        let s = host.snapshot(&token).map_err(|_| "source_snapshot")?;
        if s.channel_id != channel || s.broadcast_id != broadcast {
            return Err("source_changed".into());
        }
        if job.is_none() {
            job = s.recording_job.clone();
        }
        if job.is_none() && begin.elapsed() > Duration::from_secs(8) {
            return Err("record_ack".into());
        }
        if fs2::available_space(root).map_err(|_| "disk_space")? < 2 * 1024 * 1024 * 1024 {
            return Err("disk_reserve".into());
        }
        if owned_bytes(&root.join("media"))? > 256 * 1024 * 1024 {
            return Err("soft_media_budget".into());
        }
        let facts = eval(app, WATCH)?;
        clipped |= facts["truncated"] == true;
        let rows = facts["rows"].as_array().ok_or("observation_shape")?;
        let mission = rows.iter().any(|v| observed_group(v) == "mission");
        let mut shot = None;
        if shots < 3 && (mission || (shots == 0 && !rows.is_empty())) {
            let name = format!("official-{shots}.png");
            let result = capture_original(app, &root.join(&name));
            report[format!("capture{shots}Succeeded")] = json!(result.is_ok());
            if result.is_ok() {
                shot = Some(name);
            }
            shots += 1;
        }
        for row in rows {
            let mut row = row.clone();
            row["officialScreenshot"] = json!(shot);
            if observations.len() < 256 {
                observations.push(row);
            } else if observed_group(&row) == "mission" {
                if let Some(i) = observations
                    .iter()
                    .position(|v| observed_group(v) != "mission")
                {
                    observations.remove(i);
                    observations.push(row);
                } else {
                    clipped = true;
                }
            } else {
                clipped = true;
            }
        }
        thread::sleep(
            Duration::from_millis(500).min(Duration::from_secs(90).saturating_sub(begin.elapsed())),
        );
    }
    report["observationSeconds"] = json!(begin.elapsed().as_secs_f64());
    report["domObservationTruncated"] = json!(clipped);
    report["lastOfficialAppearance"] = observe(app)?;
    require_muted(&report["lastOfficialAppearance"])?;
    host.record(app, &token, false).map_err(|_| "record_stop")?;
    let job = job.ok_or("record_missing")?;
    report["jobId"] = json!(job);
    let drain = Instant::now() + Duration::from_secs(120);
    loop {
        let j = media
            .jobs(Some("soop"))
            .into_iter()
            .find(|j| j.job_id == job)
            .ok_or("job_missing")?;
        if matches!(
            j.state.as_str(),
            "completed" | "failed" | "interrupted" | "cancelled"
        ) {
            break;
        }
        if Instant::now() > drain {
            return Err("normal_drain_pending".into());
        }
        thread::sleep(Duration::from_millis(250));
    }
    write(
        &root.join("official-dom-observations.json"),
        &json!({"actual":true,"rows":observations}),
    )?;
    saved_evidence(media, root, &job, &observations, report)?;
    report["status"] = json!(if report["mission121Observed"] == true {
        "mission_observed_render_comparison_pending"
    } else {
        "mission_not_observed_other_appearance_recorded"
    });
    host.close(app, &token).map_err(|_| "source_close")?;
    Ok(())
}

#[test]
#[ignore = "One public SOOP90s fresh-profile observation; no account/gift/cloud/desktop actions"]
fn actual_public_mission_or_other_appearance_90s() {
    assert_eq!(
        std::env::var("ATSUMI_SOOP_MISSION_QA").as_deref(),
        Ok("public-90s-fresh-profile"),
        "QA_OPT_IN"
    );
    let channel = std::env::var("ATSUMI_SOOP_MISSION_CHANNEL")
        .unwrap_or_else(|_| "ioioiobb".into())
        .to_ascii_lowercase();
    assert!(policy::valid_channel(&channel), "QA_CHANNEL");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string();
    let root = workspace.join(".runtime/soop-mission-native-qa").join(&run);
    no_links(&root).unwrap();
    fs::create_dir_all(root.parent().unwrap()).unwrap();
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("media")).unwrap();
    let identity = format!("local.atsumi.soopmissionqa.{run}");
    let profile = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(&identity);
    no_links(&profile).unwrap();
    assert!(!profile.exists(), "QA_PROFILE_EXISTS");
    let ff = workspace
        .join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let media = PlatformMediaService::new(
        root.join("data"),
        PlatformMediaTools {
            yt_dlp: workspace.join(".runtime/platform-tools/bin/yt-dlp.exe"),
            deno: workspace.join(".runtime/platform-tools/bin/deno.exe"),
            ffmpeg: ff.join("ffmpeg.exe"),
            ffprobe: ff.join("ffprobe.exe"),
        },
        None,
    )
    .unwrap();
    assert!(media.jobs(None).is_empty(), "QA_NOT_EMPTY");
    let result = Arc::new(Mutex::new(json!({"status":"not_run"})));
    let output = result.clone();
    let worker_root = root.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = identity;
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app=tauri::Builder::default().any_thread().manage(SoopOriginalHost::default()).setup(move|app|{
        if app.path().app_data_dir()?!=profile{return Err("QA_PROFILE_PATH".into());}no_links(&profile)?;
        tauri::window::WindowBuilder::new(app,"main").title("Atsumi public mission observation").inner_size(1290.,720.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
        let handle=app.handle().clone();thread::spawn(move||{
            let mut report=json!({"status":"failed","source":channel,"localOnly":true,"noCloud":true,"noDesktopInput":true,"mission121Observed":false,"allArtifactsRetained":true});
            let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||observe_owned(&handle,&media,&worker_root,&channel,&mut report))).unwrap_or_else(|_|Err("qa_worker_panic".into()));
            report["error"]=json!(outcome.err());let host=handle.state::<SoopOriginalHost>();host.shutdown(&handle);media.shutdown();
            let grace=Instant::now()+Duration::from_secs(30);while !media.active_ids().is_empty()&&Instant::now()<grace{thread::sleep(Duration::from_millis(100));}
            if !media.active_ids().is_empty(){report["normalDrainPending"]=json!(true);let _=write(&worker_root.join("pending-result.json"),&report);while !media.active_ids().is_empty(){thread::sleep(Duration::from_secs(1));}}
            report["normalDrainPending"]=json!(false);report["activeOwnedWork"]=json!(media.active_ids().len());*lock(&output)=report;handle.exit(0);
        });Ok(())
    }).build(context).expect("QA_BUILD");
    app.run_return(|_, _| {});
    let mut report = lock(&result).clone();
    report["normalExitReturned"] = json!(true);
    write(&root.join("result.json"), &report).unwrap();
    println!("QA_MISSION_RESULT={}", root.join("result.json").display());
    assert!(
        report["error"].is_null(),
        "Actual bounded capture incomplete; inspect safe report"
    );
}

#[test]
fn observed_integer_accepts_integral_webview_numbers_without_rounding() {
    for text in ["1795297232583", "1795297232583.00", "0", "0.0", "9007199254740991"] {
        let v: Value = serde_json::from_str(text).unwrap();
        assert!(observed_integer(&v).is_some(), "{text}");
    }
    for text in ["-1", "1.5", "9007199254740992", "9007199254740992.0", "18446744073709551615", "\"1795297232583\"", "null", "true"] {
        let v: Value = serde_json::from_str(text).unwrap();
        assert!(observed_integer(&v).is_none(), "{text}");
    }
    let dom: Value = serde_json::from_str(r#"{"category":"gift","author":"fixture","count":10.0}"#).unwrap();
    assert_eq!(match_signature(&dom, true, "salt"), match_signature(&json!({"author":"fixture","soopGift":{"count":10}}), false, "salt"));
}

struct PinnedEvidence {
    path: PathBuf,
    file: fs::File,
    bytes: u64,
    sha256: String,
}
fn original_file_list(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut files = Vec::new();
    while let Some((dir, depth)) = pending.pop() {
        if depth > 8 || pending.len() + files.len() > 4096 { return Err("source_tree_bound".into()); }
        no_links(&dir)?;
        for entry in fs::read_dir(dir).map_err(|_| "source_tree_read")? {
            let path = entry.map_err(|_| "source_tree_entry")?.path();
            no_links(&path)?;
            let metadata = fs::symlink_metadata(&path).map_err(|_| "source_tree_metadata")?;
            if metadata.is_dir() { pending.push((path, depth + 1)); }
            else if metadata.is_file() { files.push(path); }
            else { return Err("source_tree_type".into()); }
        }
    }
    files.sort();
    Ok(files)
}
fn pinned_hash(file: &mut fs::File, expected: u64) -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(0)).map_err(|_| "source_seek")?;
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|_| "source_read")?;
        if n == 0 { break; }
        bytes = bytes.checked_add(n as u64).ok_or("source_size")?;
        if bytes > expected { return Err("source_grew".into()); }
        digest.update(&buffer[..n]);
    }
    if bytes != expected { return Err("source_changed".into()); }
    Ok(format!("{:x}", digest.finalize()))
}
fn pin_originals(root: &Path) -> Result<Vec<PinnedEvidence>, String> {
    use std::os::windows::fs::OpenOptionsExt;
    let mut result = Vec::new();
    let mut total = 0u64;
    for path in original_file_list(root)? {
        crate::platform_media::storage::regular(&path)?;
        let mut file = fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000)
            .open(&path).map_err(|_| "source_pin")?;
        let bytes = file.metadata().map_err(|_| "source_metadata")?.len();
        total = total.checked_add(bytes).ok_or("source_size")?;
        if total > 512 * 1024 * 1024 { return Err("source_total_bound".into()); }
        let sha256 = pinned_hash(&mut file, bytes)?;
        result.push(PinnedEvidence { path, file, bytes, sha256 });
    }
    Ok(result)
}
struct ReadonlyExportService(PlatformMediaService);
impl Drop for ReadonlyExportService {
    fn drop(&mut self) { self.0.shutdown(); }
}

#[test]
#[ignore = "Exact completed SOOP90 run only: local read/export, no WebView/source/network or mutation of existing files"]
fn actual_resume_saved_mission_export_9e6d6e0c() {
    assert_eq!(std::env::var("ATSUMI_SOOP_MISSION_SAVED_QA").as_deref(), Ok("resume-9e6d6e0c-readonly"), "QA_OPT_IN");
    const RUN: &str = "9e6d6e0cbe7c4c02a1b948bbb5dc1f94";
    const JOB: &str = "e312f54a-d9ca-4f0d-bb78-dc0a6e8ba012";
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(".runtime/soop-mission-native-qa").join(RUN);
    let export = root.join("actual-replay-export.json");
    let result_path = root.join("resume-result.json");
    no_links(&root).unwrap();
    assert!(!export.exists() && !result_path.exists(), "QA_OUTPUT_EXISTS");
    let mut pinned = pin_originals(&root).expect("QA_PIN_ORIGINALS");
    let fingerprint: Vec<_> = pinned.iter().map(|p| json!([
        p.path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"), p.bytes, p.sha256
    ])).collect();
    assert_eq!(format!("{:x}", Sha256::digest(serde_json::to_vec(&fingerprint).unwrap())),
        "0ff168024d6773a6c05b8c9b5777e5c37df7d00522dd1adf2041cca67d372a0a", "QA_EXACT_ORIGINAL_SNAPSHOT");
    let prior_raw = read(&root.join("result.json"), 1024 * 1024).unwrap();
    let mut report: Value = serde_json::from_slice(&prior_raw).unwrap();
    assert_eq!(report["error"], "observation_clock", "QA_ORIGINAL_FAILURE");
    assert_eq!(report["normalExitReturned"], true, "QA_ORIGINAL_NOT_EXITED");
    assert_eq!(report["activeOwnedWork"], 0, "QA_ORIGINAL_ACTIVE");
    assert_eq!(report["jobId"], JOB, "QA_ORIGINAL_JOB");
    let journal = root.join("data/platform-media/jobs");
    let entries: Vec<_> = fs::read_dir(&journal).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(entries, vec![journal.join(format!("{JOB}.json"))], "QA_ONLY_COMPLETED_JOB");
    let job: crate::platform_media::MediaJob = serde_json::from_slice(&read(&entries[0], 256 * 1024).unwrap()).unwrap();
    assert_eq!(job.job_id, JOB); assert_eq!(job.state, "completed"); assert!(!job.active());
    assert_eq!(job.reference.provider, "soop");
    assert_eq!(job.library.as_ref().unwrap().library_id, JOB);
    assert_eq!(fs::canonicalize(&job.download_root).unwrap(), fs::canonicalize(root.join("media")).unwrap());
    let observations: Value = serde_json::from_slice(&read(&root.join("official-dom-observations.json"), 2 * 1024 * 1024).unwrap()).unwrap();
    assert_eq!(observations["actual"], true);
    let rows = observations["rows"].as_array().unwrap();
    assert!(rows.len() <= 256 && rows.iter().all(|r| observed_integer(&r["at"]).is_some()), "QA_OBSERVATION_NUMBERS");
    // Completed-only service construction is read-only. Pins prevent any
    // accidental rewrite; no command can invoke these nonexistent tool paths.
    let unavailable = root.join("readonly-export-no-tool");
    assert!(!unavailable.exists());
    let tools = PlatformMediaTools { yt_dlp: unavailable.clone(), deno: unavailable.clone(), ffmpeg: unavailable.clone(), ffprobe: unavailable };
    let service = ReadonlyExportService(PlatformMediaService::new(root.join("data"), tools, None).unwrap());
    assert!(service.0.active_ids().is_empty());
    assert_eq!(service.0.jobs(None).len(), 1);
    report["originalResultSha256"] = json!(format!("{:x}", Sha256::digest(&prior_raw)));
    report["originalFailure"] = json!("observation_clock");
    report["resumedSavedEvidenceOnly"] = json!(true);
    report["nativeSourceProgressPreviouslyObserved"] = report["publicPlaybackProgressVerified"].clone();
    report["publicPlaybackProgressVerified"] = json!(false);
    report["contentPlaybackVerified"] = json!(false);
    report["observationLimitation"] = json!("The original official screenshot contains a preroll overlay and high-quality participation modal. The source uses a separate main-content MSE clock; this export does not itself verify decoded content or visible player presentation. Observed chat DOM appearance may be compared, but the dimmed full screenshot is not pixel-equivalence proof.");
    let outcome = saved_evidence(&service.0, &root, JOB, rows, &mut report);
    service.0.shutdown();
    report["activeOwnedWork"] = json!(service.0.active_ids().len());
    drop(service);
    let mut unchanged = true;
    let mut facts = Vec::new();
    for original in &mut pinned {
        let after = pinned_hash(&mut original.file, original.bytes);
        unchanged &= after.as_ref().is_ok_and(|sha| sha == &original.sha256);
        facts.push(json!({"path":original.path.strip_prefix(&root).unwrap().to_string_lossy(),"bytes":original.bytes,"sha256":original.sha256}));
    }
    let mut expected: Vec<_> = pinned.iter().map(|p| p.path.clone()).collect();
    if export.exists() { expected.push(export); }
    expected.sort();
    let same_files = original_file_list(&root).is_ok_and(|files| files == expected);
    report["originalFilesUnchanged"] = json!(unchanged && same_files);
    report["originalFiles"] = json!(facts);
    report["error"] = json!(outcome.err());
    report["status"] = json!(if report["error"].is_null() { "saved_export_ready_render_comparison_pending" } else { "saved_export_failed" });
    report["newSourceOpenCalls"] = json!(0);
    report["newNetworkCalls"] = json!(0);
    report["newDesktopActions"] = json!(0);
    write(&result_path, &report).unwrap();
    println!("QA_MISSION_SAVED_RESULT={}", result_path.display());
    assert!(unchanged && same_files, "QA_ORIGINAL_CHANGED");
    assert!(report["error"].is_null(), "QA_EXPORT_INCOMPLETE");
}
