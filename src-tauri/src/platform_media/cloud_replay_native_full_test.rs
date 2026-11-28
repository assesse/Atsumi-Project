//! Staged test-only extension. Parent fixed source/receipt guards are reused.
//! Not wired or executed until root coordinates the shared native batch.
use super::*;

const FULL_DOM: &str = r#"(()=>{const s=document.querySelector('aside[data-provider="soop"]')?.shadowRoot;return {dialog:!!document.querySelector('.cloud-recording-replay'),shadow:!!s,rows:s?.querySelectorAll('.chat_memoyo>.chatting-list-item').length??0,alerts:document.querySelectorAll('[role="alert"]').length+(s?.querySelectorAll('[role="alert"]').length??0),preparing:!!s?.querySelector('.chat_memoyo [role="status"]'),clockWait:!!document.querySelector('.cloud-recording-clock-wait')}})()"#;
const FULL_VIDEO: &str = r#"(()=>{const p=document.getElementById('movie_player'),v=p?.querySelector('video'),q=v?.getVideoPlaybackQuality?.(),b=p?.querySelector('.ytp-play-button');const visible=e=>{if(!e)return false;const r=e.getBoundingClientRect(),s=getComputedStyle(e);return r.width>0&&r.height>0&&s.display!=='none'&&s.visibility==='visible'&&Number(s.opacity)>0&&!e.closest('[hidden],[aria-hidden="true"]')};const r=b?.getBoundingClientRect(),pr=p?.getBoundingClientRect();return {ready:v?.readyState??0,time:v?.currentTime??null,duration:Number.isFinite(v?.duration)?v.duration:null,paused:v?.paused??null,ended:v?.ended??false,seeking:v?.seeking??false,rate:v?.playbackRate??null,frames:q?.totalVideoFrames??null,dropped:q?.droppedVideoFrames??null,networkState:v?.networkState??null,buffered:v?Array.from({length:Math.min(v.buffered.length,8)},(_,i)=>[v.buffered.start(i),v.buffered.end(i)]):[],ad:!!p&&(p.classList.contains('ad-showing')||p.classList.contains('ad-interrupting')),capture:!!window.__atsumiYouTubeEncodedCapture,chat:!!document.querySelector('iframe#chatframe'),rates:(p?.getAvailablePlaybackRates?.()??[]).filter(n=>typeof n==='number'&&n>=.1&&n<=16).slice(0,20),replayButtonVisible:visible(b),replayButtonSvg:!!b?.querySelector('svg'),replayButtonCentral:!!r&&!!pr&&Math.abs(r.x+r.width/2-pr.x-pr.width/2)<pr.width*.2&&Math.abs(r.y+r.height/2-pr.y-pr.height/2)<pr.height*.2,largeReplayVisible:visible(p?.querySelector('.ytp-large-play-button'))}})()"#;
const CLICK_OFFICIAL_REPLAY: &str = r#"(()=>{const p=document.getElementById('movie_player'),v=p?.querySelector('video'),b=p?.querySelector('.ytp-play-button');if(!v?.ended||!b||b.disabled||b.getAttribute('aria-disabled')==='true'||p.classList.contains('ad-showing'))return false;const r=b.getBoundingClientRect(),s=getComputedStyle(b);if(!r.width||!r.height||s.display==='none'||s.visibility!=='visible'||Number(s.opacity)<=0||b.closest('[hidden],[aria-hidden="true"]'))return false;b.click();return true})()"#;

fn full_rate(app: &tauri::AppHandle, token: &str, rate: f64) -> Result<(), String> {
    let c = clock(app, token)?;
    app.state::<YouTubeOriginalHost>().vod_control(app, token, VodControlRequest {
        request_id: uuid::Uuid::new_v4().to_string(), document_epoch: c.document_epoch,
        source_id: c.fields.source_id, source_generation: c.fields.source_generation,
        action: "rate".into(), seconds: None, rate: Some(rate),
    }).map_err(|_| "QA_FULL_RATE_CONTROL".into())
}

fn full_identity(state: &QaState, c: &VodClock) -> Result<(), String> {
    if c.fields.video_id != state.job.video_id.as_deref().unwrap_or("")
        || c.fields.duration.is_none_or(|v| (v - 42.102521).abs() > 1.0) {
        return Err("QA_FULL_REMOTE_IDENTITY".into());
    }
    Ok(())
}

fn full_expected(times: &[u64], seconds: f64) -> usize {
    if seconds < 0.0 { return 0; }
    let at = (seconds * 1000.0).floor() as u64;
    times.iter().filter(|value| **value <= at).count()
}

/// Observe without injecting a seek or modifying the media clock during a pass.
/// Persist only numerical media facts/counts, never IDs, authors or message text.
fn full_observe(
    app: &tauri::AppHandle, state: &QaState, main: &Webview, remote: &Webview,
    token: &str, phase: &str, expected_rate: f64, expected_times: &[u64],
    seconds: u64, require_whole: bool, samples: &mut Vec<Value>,
) -> Result<Value, String> {
    let start = Instant::now();
    let deadline = (start + Duration::from_secs(seconds)).min(state.deadline);
    let initial = clock(app, token)?;
    full_identity(state, &initial)?;
    let first_time = initial.fields.current_time.ok_or("QA_FULL_TIME")?;
    if require_whole && first_time > 0.5 { return Err("QA_FULL_START_NOT_ZERO".into()); }
    let mut last_sequence = initial.fields.sequence;
    let mut last_time = first_time;
    let mut progressed_at = start;
    let mut last_good_clock = start;
    let mut chat_bad_since: Option<Instant> = None;
    let mut max_clock_gap_ms = 0u128;
    let mut max_nonprogress_ms = 0u128;
    let mut buffering_samples = 0usize;
    let mut bins = std::collections::BTreeSet::new();
    let first_video = evaluate(remote, FULL_VIDEO)?;
    let first_frames = first_video["frames"].as_u64().ok_or("QA_FULL_FRAME_COUNTER")?;
    let mut count = 0usize;
    loop {
        let now = Instant::now();
        if now >= deadline { return Err("QA_FULL_PHASE_TIMEOUT".into()); }
        if samples.len() >= 700 { return Err("QA_FULL_SAMPLE_BOUND".into()); }
        let raw = app.state::<YouTubeOriginalHost>().vod_clock(token).map_err(|_| "QA_FULL_CLOCK_READ")?;
        let Some(c) = raw.filter(|v| v.fields.valid && v.fields.ad_state == "content" && v.age_ms <= 1500) else {
            let gap = now.duration_since(last_good_clock).as_millis();
            max_clock_gap_ms = max_clock_gap_ms.max(gap);
            samples.push(json!({"phase":phase,"wallMs":start.elapsed().as_millis(),"validContentClock":false,"gapMs":gap}));
            if gap > 2000 { return Err("QA_FULL_CLOCK_GAP".into()); }
            thread::sleep(Duration::from_millis(250)); continue;
        };
        full_identity(state, &c)?;
        max_clock_gap_ms = max_clock_gap_ms.max(now.duration_since(last_good_clock).as_millis());
        last_good_clock = now;
        if c.document_epoch != initial.document_epoch || c.fields.source_id != initial.fields.source_id
            || c.fields.source_generation != initial.fields.source_generation
            || c.fields.seek_generation != initial.fields.seek_generation
            || c.fields.sequence < last_sequence { return Err("QA_FULL_SOURCE_OR_SEQUENCE_CHANGED".into()); }
        last_sequence = c.fields.sequence;
        let time = c.fields.current_time.ok_or("QA_FULL_TIME")?;
        if time + 0.03 < last_time || c.fields.seeking { return Err("QA_FULL_UNREQUESTED_SEEK".into()); }
        if (c.fields.playback_rate - expected_rate).abs() > 0.000001 { return Err("QA_FULL_RATE_CHANGED".into()); }
        if time > last_time + 0.01 { progressed_at = now; }
        let stalled_ms = now.duration_since(progressed_at).as_millis();
        if !c.fields.ended { max_nonprogress_ms = max_nonprogress_ms.max(stalled_ms); }
        last_time = time;
        bins.insert(time.floor().max(0.0) as u64);
        let video = evaluate(remote, FULL_VIDEO)?;
        let dom = evaluate(main, FULL_DOM)?;
        if video["ad"] != false || video["capture"] != false || video["chat"] != false {
            return Err("QA_FULL_UNEXPECTED_RECEIVER_OR_AD".into());
        }
        if dom["dialog"] != true || dom["shadow"] != true || dom["alerts"] != 0 {
            return Err("QA_FULL_REPLAY_UI_UNAVAILABLE".into());
        }
        let rows = dom["rows"].as_u64().ok_or("QA_FULL_ROWS")? as usize;
        // Native clock can precede DOM by a250ms publish tick. Compare exact
        // saved millisecond timestamps; the bounds below cover only asynchronous
        // observation latency, not an accepted integer-second chat quantization.
        let after = clock(app, token)?;
        let latest_time = after.fields.current_time.unwrap_or(time).max(video["time"].as_f64().unwrap_or(time));
        let lower = full_expected(expected_times, time - expected_rate * 1.5);
        let upper = full_expected(expected_times, latest_time + expected_rate * 0.5);
        let aligned = rows >= lower && rows <= upper && dom["preparing"] == false;
        if aligned { chat_bad_since = None; } else { chat_bad_since.get_or_insert(now); }
        let chat_gap_ms = chat_bad_since.map(|since| now.duration_since(since).as_millis()).unwrap_or(0);
        if c.fields.buffering || video["ready"].as_u64().unwrap_or(0) < 3 { buffering_samples += 1; }
        samples.push(json!({"phase":phase,"wallMs":start.elapsed().as_millis(),"validContentClock":true,
            "time":time,"videoTime":video["time"],"sequence":c.fields.sequence,"ageMs":c.age_ms,
            "paused":c.fields.paused,"ended":c.fields.ended,"seeking":c.fields.seeking,"rate":c.fields.playback_rate,
            "buffering":c.fields.buffering,"readyState":video["ready"],"frames":video["frames"],"droppedFrames":video["dropped"],
            "buffered":video["buffered"],"rows":rows,"expectedRowsLower":lower,"expectedRowsUpper":upper,
            "chatConvergenceMs":chat_gap_ms,"clockWaitVisible":dom["clockWait"]}));
        count += 1;
        if c.fields.ended && video["ended"] == true {
            if (time - c.fields.duration.unwrap_or(0.0)).abs() > 0.3 { return Err("QA_FULL_EARLY_EOF".into()); }
            let settle = (Instant::now() + Duration::from_millis(1800)).min(state.deadline);
            wait_until(settle, || Ok(evaluate(main, FULL_DOM)?["rows"].as_u64() == Some(expected_times.len() as u64)))?;
            if require_whole && (0..42).any(|second| !bins.contains(&second)) { return Err("QA_FULL_COVERAGE_GAP".into()); }
            let frames = video["frames"].as_u64().ok_or("QA_FULL_FRAME_COUNTER")?;
            if frames <= first_frames { return Err("QA_FULL_NO_DECODE_PROGRESS".into()); }
            let wall = start.elapsed().as_secs_f64();
            return Ok(json!({"phase":phase,"naturalEof":true,"startTime":first_time,"endTime":time,"wallSeconds":wall,
                "effectiveMediaRate":(time-first_time)/wall,"requestedRate":expected_rate,"samples":count,
                "observedSeconds":bins,"decodedFramesDelta":frames-first_frames,"maxClockGapMs":max_clock_gap_ms,
                "maxNonprogressMs":max_nonprogress_ms,"bufferingSamples":buffering_samples,"finalRows":expected_times.len()}));
        }
        if c.fields.paused { return Err("QA_FULL_UNEXPECTED_PAUSE".into()); }
        if stalled_ms > 2000 { return Err("QA_FULL_SUSTAINED_NONPROGRESS".into()); }
        if chat_gap_ms > 1500 { return Err("QA_FULL_CHAT_CONVERGENCE".into()); }
        thread::sleep(Duration::from_millis(250));
    }
}

fn full_exercise_inner(app: &tauri::AppHandle, state: &QaState, output: &Path, samples: &mut Vec<Value>) -> Result<Value, String> {
    let main = app.get_webview("main").ok_or("QA_MAIN")?;
    let ready = (Instant::now() + Duration::from_secs(70)).min(state.deadline);
    wait_until(ready, || Ok(evaluate(&main, DOM)?["ready"] == true && !state.video_tokens.lock().unwrap().is_empty() && !state.leases.lock().unwrap().is_empty()))?;
    let token = state.video_tokens.lock().unwrap().last().cloned().ok_or("QA_VIDEO_TOKEN")?;
    wait_until(ready, || Ok(clock(app, &token).is_ok()))?;
    full_identity(state, &clock(app, &token)?)?;
    let remote = app.webviews().into_values().find(|v| v.label().starts_with("youtube-original-vod-")).ok_or("QA_REMOTE_VIEW")?;
    let lease = state.leases.lock().unwrap().last().cloned().ok_or("QA_LEASE")?;
    // This provider has independent immutable page readers, unlike CHZZK's
    // renderer-owned chat_at generation. Read once before observing playback.
    let mut times = None;
    wait_until(ready, || {
        let page = state.media.cloud_chat_page(&lease, 43_000, None, 120).map_err(|_| "QA_FULL_EXPECTED_PAGE")?;
        if page.state != "ready" { return Ok(false); }
        if page.items.len() != 14 || page.items.iter().any(|row| row.time_ms.is_none()) { return Err("QA_FULL_EXPECTED_ROWS".into()); }
        times = Some(page.items.iter().filter_map(|row| row.time_ms).collect::<Vec<_>>()); Ok(true)
    })?;
    let times = times.ok_or("QA_FULL_EXPECTED_TIMES")?;
    control(app, &token, "pause", None)?;
    full_rate(app, &token, 1.0)?;
    control(app, &token, "seek", Some(0.0))?;
    wait_until(ready, || Ok(clock(app, &token).is_ok_and(|c| c.fields.paused && !c.fields.seeking && c.fields.current_time.is_some_and(|t| t < 0.1)) && evaluate(&main, FULL_DOM)?["rows"] == 0))?;
    control(app, &token, "resume", None)?;
    wait_until(ready, || Ok(clock(app, &token).is_ok_and(|c| !c.fields.paused && !c.fields.seeking && !c.fields.buffering)))?;
    let first = full_observe(app, state, &main, &remote, &token, "one-times-full", 1.0, &times, 60, true, samples)?;
    thread::sleep(Duration::from_millis(1100));
    let eof = clock(app, &token)?;
    if !eof.fields.ended || state.video_tokens.lock().unwrap().last() != Some(&token) { return Err("QA_FULL_EOF_SESSION_CHANGED".into()); }
    let eof_controls = evaluate(&remote, FULL_VIDEO)?;
    capture(&main, &output.join("actual42-full-eof-chat.png"))?;
    capture(&remote, &output.join("actual42-full-eof-video.png"))?;
    if evaluate(&remote, CLICK_OFFICIAL_REPLAY)? != true { return Err("QA_FULL_OFFICIAL_REPLAY_UNAVAILABLE".into()); }
    let resume_deadline = (Instant::now() + Duration::from_secs(8)).min(state.deadline);
    wait_until(resume_deadline, || Ok(clock(app, &token).is_ok_and(|c| !c.fields.ended && !c.fields.paused && c.fields.current_time.is_some_and(|t| t < 3.0)) && evaluate(&main, FULL_DOM)?["rows"] == 0))?;
    let rates = evaluate(&remote, FULL_VIDEO)?;
    if !rates["rates"].as_array().is_some_and(|list| list.iter().any(|v| v.as_f64().is_some_and(|n| (n-1.5).abs()<0.000001))) { return Err("QA_FULL_RATE_15_UNAVAILABLE".into()); }
    full_rate(app, &token, 1.5)?;
    wait_until(resume_deadline, || Ok(clock(app, &token).is_ok_and(|c| !c.fields.paused && !c.fields.seeking && (c.fields.playback_rate-1.5).abs()<0.000001)
        && evaluate(&remote, FULL_VIDEO)?["rate"].as_f64().is_some_and(|v| (v-1.5).abs()<0.000001)))?;
    let second = full_observe(app, state, &main, &remote, &token, "replayed-one-point-five", 1.5, &times, 40, false, samples)?;
    let effective = second["effectiveMediaRate"].as_f64().ok_or("QA_FULL_RATE_METRIC")?;
    if !(1.3..=1.7).contains(&effective) { return Err("QA_FULL_RATE_PROGRESS_MISMATCH".into()); }
    // Exercise the historic dead-end specifically after natural EOF, same token.
    full_rate(app, &token, 1.0)?;
    let tail = clock(app, &token)?.fields.duration.ok_or("QA_FULL_DURATION")? - 5.0;
    control(app, &token, "seek", Some(tail))?;
    let seek_deadline = (Instant::now() + Duration::from_secs(8)).min(state.deadline);
    wait_until(seek_deadline, || Ok(clock(app, &token).is_ok_and(|c| !c.fields.seeking && !c.fields.ended && c.fields.current_time.is_some_and(|t| (t-tail).abs()<0.35))))?;
    control(app, &token, "resume", None)?;
    wait_until(seek_deadline, || Ok(clock(app, &token).is_ok_and(|c| !c.fields.paused && !c.fields.seeking && !c.fields.buffering)))?;
    let tail_result = full_observe(app, state, &main, &remote, &token, "post-eof-tail", 1.0, &times, 15, false, samples)?;
    if state.video_tokens.lock().unwrap().len() != 1 || state.leases.lock().unwrap().len() != 1 { return Err("QA_FULL_SESSION_REPLACED".into()); }
    evaluate(&main, "(()=>{window.__atsumiActual42Qa.close();return true})()")?;
    wait_until((Instant::now()+Duration::from_secs(8)).min(state.deadline), || Ok(state.leases.lock().unwrap().is_empty()))?;
    Ok(json!({"passed":true,"firstPass":first,"replayedRatePass":second,"postEofTail":tail_result,
        "eofOfficialControls":eof_controls,"officialReplayWorked":true,"sameNativeSession":true,
        "centralReplayVisuallyVerified":false,"donationAvVerified":false,"audioAudibilityVerified":false,
        "localVideoBytesRead":0,"newUpload":false,"desktopInput":false}))
}

fn exercise_full_playthrough(app: &tauri::AppHandle, state: &QaState, output: &Path) -> Result<Value, String> {
    let mut samples = Vec::new();
    let result = full_exercise_inner(app, state, output, &mut samples);
    let trace = serde_json::to_vec_pretty(&json!({"scope":"actual existing SOOP42 remote watch-only full playback", "samples":samples})).map_err(|_| "QA_FULL_TRACE_JSON")?;
    if trace.len() > 512*1024 { return Err("QA_FULL_TRACE_BOUND".into()); }
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(output.join("playthrough-trace.json")).map_err(|_| "QA_FULL_TRACE_CREATE")?;
    file.write_all(&trace).map_err(|_| "QA_FULL_TRACE_WRITE")?;
    result
}

#[test]
#[ignore = "Actual existing SOOP42 full playback/EOF/rate; separate explicit scoped environment required"]
fn actual_soop42_native_full_playthrough_eof_rate() {
    assert_eq!(std::env::var("ATSUMI_SOOP_NATIVE_FULL_REPLAY_QA").as_deref(), Ok("actual-unlisted-full-readonly"), "QA_OPT_IN");
    let host = ActualReceiptHost::open_current();
    let authorization = host.authorization(); let job = host.job_view();
    let catalog = host.profile().join("platform-media/jobs").join(format!("{}.json", actual::RECORDING));
    let raw = host.read(&catalog, 16 * 1024); assert_eq!(actual::digest(&raw), CATALOG_SHA, "QA_CATALOG");
    let source_job: MediaJob = serde_json::from_slice(&raw).unwrap();
    let directory = storage::job_dir(&source_job.download_root, "soop", actual::RECORDING).unwrap();
    let before = snapshot(&host, &directory);
    let media = no_workers(host.scratch(), source_job);
    media.validate_cloud_chat_source(&authorization).unwrap_or_else(|_| panic!("QA_SOURCE"));
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let output = workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume").join(format!("actual42-full-native-{run_id}"));
    fs::create_dir(&output).unwrap();
    let identity = format!("local.atsumi.cloudreplayqa.{run_id}");
    let profile = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(&identity);
    assert!(!profile.exists(), "QA_PROFILE_EXISTS");
    let url: tauri::Url = std::env::var("ATSUMI_SOOP_NATIVE_FULL_REPLAY_QA_URL").unwrap_or_else(|_| format!("http://127.0.0.1:5181{ENTRY}")).parse().unwrap();
    assert!(url.scheme()=="http" && url.host_str()==Some("127.0.0.1") && url.port()==Some(5181) && url.path()==ENTRY
        && url.username().is_empty() && url.password().is_none() && url.query().is_none() && url.fragment().is_none(), "QA_ENTRY");
    let state = Arc::new(QaState {media:media.clone(),authorization,job,leases:Mutex::new(Vec::new()),video_tokens:Mutex::new(Vec::new()),deadline:Instant::now()+Duration::from_secs(195),closed:AtomicBool::new(false)});
    let outcome = Arc::new(Mutex::new(None::<Result<Value,String>>));
    let outcome_worker = outcome.clone(); let worker_state = state.clone(); let output_worker = output.clone(); let profile_check = profile.clone();
    let old_args = std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS");
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "--mute-audio");
    let mut context = tauri::generate_context!();
    context.config_mut().identifier=identity;
    context.config_mut().build.dev_url=Some(url.clone());
    context.config_mut().app.windows.clear(); context.config_mut().app.tray_icon=None;
    let app = tauri::Builder::default().any_thread().manage(state.clone()).manage(YouTubeOriginalHost::default())
        .invoke_handler(tauri::generate_handler![qa_actual42_bootstrap,qa_actual42_describe,qa_actual42_open,qa_actual42_page,qa_actual42_asset,qa_actual42_close,youtube_original_open,
            crate::youtube::original_commands::youtube_original_snapshot,crate::youtube::original_commands::youtube_original_vod_clock,crate::youtube::original_commands::youtube_original_vod_control,
            crate::youtube::original_commands::youtube_original_viewport,crate::youtube::original_commands::youtube_original_close,crate::youtube::original_commands::youtube_original_controls,crate::youtube::original_commands::youtube_original_fullscreen])
        .setup(move |app| {
            if app.path().app_data_dir()? != profile_check { return Err("QA_PROFILE_PATH".into()); }
            tauri::WebviewWindowBuilder::new(app,"main",tauri::WebviewUrl::External(url)).data_directory(profile_check.join("main-webview")).title("Atsumi native replay verification").inner_size(1280.,760.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
            let handle=app.handle().clone();
            thread::spawn(move || {
                let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| exercise_full_playthrough(&handle,&worker_state,&output_worker)))
                    .unwrap_or_else(|_| Err("QA_WORKER_PANIC".into()));
                if value.is_err() {
                    let main = handle.get_webview("main");
                    let remote = handle.webviews().into_values().find(|v| v.label().starts_with("youtube-original-vod-"));
                    let facts = json!({"main":main.as_ref().and_then(|view|evaluate(view,DOM).ok()),"video":remote.as_ref().and_then(|view|evaluate(view,VIDEO).ok())});
                    let _=fs::write(output_worker.join("failure-facts.json"),serde_json::to_vec_pretty(&facts).unwrap());
                    if let Some(view)=main {let _=capture(&view,&output_worker.join("failure-main.png"));}
                    if let Some(view)=remote {let _=capture(&view,&output_worker.join("failure-video.png"));}
                }
                worker_state.closed.store(true,Ordering::Release);
                for token in worker_state.leases.lock().unwrap().drain(..) { worker_state.media.close_cloud_chat(&token); }
                let receiver=handle.state::<YouTubeOriginalHost>();
                for token in worker_state.video_tokens.lock().unwrap().drain(..) { let _=receiver.close(&handle,&token); }
                receiver.shutdown(&handle);
                *outcome_worker.lock().unwrap()=Some(value);
                handle.exit(0);
            }); Ok(())
        }).build(context).unwrap_or_else(|_| panic!("QA_NATIVE_BUILD"));
    app.run_return(|_,_|{});
    if let Some(value)=old_args {std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",value)} else {std::env::remove_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")}
    assert_eq!(snapshot(&host,&directory),before,"QA_SOURCE_SIDECARS_CHANGED");
    assert_eq!(actual::digest(&host.read(&catalog,16*1024)),CATALOG_SHA,"QA_CATALOG_CHANGED");
    assert!(media.inner.playback.lock().unwrap().is_empty() && media.inner.cloud_chat.lock().unwrap().is_empty() && media.inner.upload_leases.lock().unwrap().is_empty(),"QA_LEASE_REMAINS");
    { let saved=media.inner.state.lock().unwrap(); assert!(saved.queue.is_empty()&&saved.cancel.is_empty()&&!saved.shutting_down,"QA_WORK_STARTED"); }
    host.assert_unchanged(); drop(state); drop(media); let read_bytes=host.finish();
    // Only the fresh UUID profile may be cleaned; a busy WebView profile is retained.
    let cleaned = cleanup_profile(&profile,&format!("local.atsumi.cloudreplayqa.{run_id}"));
    let result=outcome.lock().unwrap().take().unwrap_or_else(||Err("QA_NO_RESULT".into()));
    let report=match &result {Ok(value)=>json!({"result":value,"sourceHashesUnchanged":true,"freshProfileCleaned":cleaned,"sourceAndScratchReadBytes":read_bytes}),Err(code)=>json!({"passed":false,"errorCode":code,"sourceHashesUnchanged":true,"freshProfileCleaned":cleaned,"sourceAndScratchReadBytes":read_bytes})};
    fs::write(output.join("result.json"),serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("QA_NATIVE_FULL_RESULT={}",output.join("result.json").display());
    assert!(result.is_ok(),"Actual native playback unverified; inspect sanitized result");
}



const CENTER_REPLAY: &str = r#"(()=>{const p=document.getElementById('movie_player'),b=p?.querySelector('button[data-atsumi-vod-restart]'),v=p?.querySelector('video');if(!p||!b||!v)return {present:false};const r=b.getBoundingClientRect(),q=p.getBoundingClientRect(),own=getComputedStyle(b);let ancestorsVisible=true,opacity=1,hiddenVisibilityAncestors=0;for(let n=b;n instanceof Element;n=n.parentElement){const s=getComputedStyle(n);opacity*=Number(s.opacity);if(s.visibility!=='visible')hiddenVisibilityAncestors++;if(n.hasAttribute('hidden')||n.hasAttribute('inert')||n.getAttribute('aria-hidden')==='true'||s.display==='none'||s.contentVisibility==='hidden'||!Number.isFinite(opacity)||opacity<=.001)ancestorsVisible=false;}const zIndex=s=>{const z=Number(s.zIndex);return Number.isFinite(z)&&z>=0&&z<=2147483647?z:null};const endcardAncestorZIndices=[];for(let n=p.querySelector('.ytp-fullscreen-grid-stills-container'),depth=0;n&&depth<12;n=n.parentElement,depth++){const z=zIndex(getComputedStyle(n));if(z!==null)endcardAncestorZIndices.push(z);if(n===p)break;}const cx=r.x+r.width/2,cy=r.y+r.height/2,hit=document.elementFromPoint(cx,cy);return {present:true,ended:v.ended,visible:ancestorsVisible&&own.visibility==='visible'&&r.width>0&&r.height>0&&r.right>0&&r.bottom>0&&r.left<innerWidth&&r.top<innerHeight,ancestorsVisible,ancestorOpacity:opacity,hiddenVisibilityAncestors,hit:!!hit&&b.contains(hit),enabled:!b.disabled&&b.getAttribute('aria-disabled')!=='true',label:b.getAttribute('aria-label'),text:b.querySelector('span')?.textContent,svg:!!b.querySelector('svg[viewBox="0 0 24 24"][aria-hidden="true"] path[d]'),centerDx:Math.abs(cx-q.x-q.width/2),centerDy:Math.abs(cy-q.y-q.height/2),width:r.width,height:r.height,buttonZIndex:zIndex(own),endcardAncestorZIndices}})()"#;
const CLICK_CENTER_REPLAY: &str = r#"(()=>{const p=document.getElementById('movie_player'),v=p?.querySelector('video'),b=p?.querySelector('button[data-atsumi-vod-restart]');if(!v?.ended||!b||b.disabled||b.hidden||b.getAttribute('aria-label')!=='처음부터 다시보기'||getComputedStyle(b).visibility!=='visible')return false;const r=b.getBoundingClientRect(),hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);if(!hit||!b.contains(hit))return false;b.click();return true})()"#;
// Failure-only bounded public DOM facts. No internal bridge/configuration state,
// account fields, text, URLs, class/attribute lists or unbounded ancestor dump.
const CENTER_FAILURE_ENV: &str = r#"(()=>{const p=document.getElementById('movie_player'),b=p?.querySelector('button[data-atsumi-vod-restart]'),v=p?.querySelector('video');const number=n=>Number.isFinite(n)&&Math.abs(n)<1e8?n:null;const rect=n=>{if(!n)return null;const r=n.getBoundingClientRect();return {x:number(r.x),y:number(r.y),width:number(r.width),height:number(r.height)}};const style=n=>{if(!n)return null;const s=getComputedStyle(n);return {displayNone:s.display==='none',visibilityVisible:s.visibility==='visible',visibilityHidden:s.visibility==='hidden',visibilityCollapse:s.visibility==='collapse',opacity:number(Number(s.opacity)),hidden:n.hasAttribute('hidden'),inert:n.hasAttribute('inert')||n.inert===true,ariaHidden:n.getAttribute('aria-hidden')==='true',contentVisibilityHidden:s.contentVisibility==='hidden',pointerEventsNone:s.pointerEvents==='none',rect:rect(n)}};const ancestors=[];let n=p?.parentElement;for(let depth=0;n&&depth<20;depth++,n=n.parentElement)ancestors.push({depth,isBody:n===document.body,isDocumentElement:n===document.documentElement,...style(n)});return {documentHidden:document.hidden,documentVisibilityState:document.visibilityState==='visible'?1:document.visibilityState==='hidden'?2:document.visibilityState==='prerender'?3:0,documentFocused:document.hasFocus(),viewportWidth:number(innerWidth),viewportHeight:number(innerHeight),playerPresent:!!p,player:style(p),buttonPresent:!!b,button:style(b),video:style(v),restartStyleInjected:!!document.getElementById('atsumi-youtube-vod-layout')?.textContent?.includes('[data-atsumi-vod-restart]'),playerSeekMethod:typeof p?.seekTo==='function',playerPlayMethod:typeof p?.playVideo==='function',ancestors,ancestorsTruncated:!!n}})()"#;

// Separate bounded evaluation preserves the existing 8192-byte callback limit.
// Only public YouTube UI names are admitted; no IDs, content or other classes.
const CENTER_OCCLUSION_ENV: &str = r#"(()=>{const p=document.getElementById('movie_player'),b=p?.querySelector('button[data-atsumi-vod-restart]');const number=n=>Number.isFinite(n)&&Math.abs(n)<1e8?n:null;const matches=(n,s)=>{try{return n.matches(s)}catch{return false}};const style=n=>{if(!n)return null;const s=getComputedStyle(n),r=n.getBoundingClientRect();return {zIndex:s.zIndex==='auto'?'auto':number(Number(s.zIndex)),position:({static:1,relative:2,absolute:3,fixed:4,sticky:5})[s.position]||0,pointerEventsNone:s.pointerEvents==='none',displayNone:s.display==='none',visibilityVisible:s.visibility==='visible',opacity:number(Number(s.opacity)),rect:{x:number(r.x),y:number(r.y),width:number(r.width),height:number(r.height)},popoverOpen:matches(n,':popover-open'),modal:matches(n,':modal')}};const r=b?.getBoundingClientRect(),cx=r?r.x+r.width/2:null,cy=r?r.y+r.height/2:null;const stack=cx!==null&&cy!==null?document.elementsFromPoint(cx,cy):[];return {centerX:number(cx),centerY:number(cy),button:style(b),hits:stack.slice(0,8).map(n=>({tag:/^[A-Za-z][A-Za-z0-9-]{0,31}$/.test(n.tagName)?n.tagName:'OTHER',classes:Array.from(n.classList).filter(c=>/^ytp-[A-Za-z0-9_-]{1,60}$/.test(c)).slice(0,8),isButton:!!b&&(n===b||b.contains(n)),knownEndscreen:!!n.closest('.html5-endscreen,.ytp-endscreen-content,.ytp-ce-element'),knownMenu:!!n.closest('.ytp-popup,.ytp-settings-menu,.ytp-contextmenu'),...style(n)})),hitsTruncated:stack.length>8,known:[['html5-endscreen','.html5-endscreen'],['ytp-endscreen-content','.ytp-endscreen-content'],['ytp-popup','.ytp-popup']].map(([kind,selector])=>({kind,present:!!document.querySelector(selector),style:style(document.querySelector(selector))}))}})()"#;

fn exercise_center_replay(app: &tauri::AppHandle, state: &QaState, output: &Path) -> Result<Value, String> {
    let main = app.get_webview("main").ok_or("QA_MAIN")?;
    let ready = (Instant::now() + Duration::from_secs(45)).min(state.deadline);
    wait_until(ready, || Ok(evaluate(&main, DOM)?["ready"] == true && !state.video_tokens.lock().unwrap().is_empty() && !state.leases.lock().unwrap().is_empty()))?;
    let token = state.video_tokens.lock().unwrap().last().cloned().ok_or("QA_VIDEO_TOKEN")?;
    let lease = state.leases.lock().unwrap().last().cloned().ok_or("QA_LEASE")?;
    wait_until(ready, || Ok(clock(app, &token).is_ok()))?;
    let c = clock(app, &token)?; full_identity(state, &c)?;
    let duration = c.fields.duration.ok_or("QA_CENTER_DURATION")?;
    let remote = app.webviews().into_values().find(|v| v.label().starts_with("youtube-original-vod-")).ok_or("QA_REMOTE_VIEW")?;
    control(app, &token, "pause", None)?;
    control(app, &token, "seek", Some(duration - 2.0))?;
    wait_until(ready, || Ok(clock(app, &token).is_ok_and(|value| !value.fields.seeking && value.fields.paused && value.fields.current_time.is_some_and(|t| (t-duration+2.0).abs()<0.35))))?;
    control(app, &token, "resume", None)?;
    let end_deadline = (Instant::now() + Duration::from_secs(10)).min(state.deadline);
    wait_until(end_deadline, || Ok(clock(app, &token).is_ok_and(|value| value.fields.ended) && evaluate(&remote, FULL_VIDEO)?["ended"] == true && evaluate(&main, FULL_DOM)?["rows"] == 14))?;
    wait_until(end_deadline, || {
        let v = evaluate(&remote, CENTER_REPLAY)?;
        Ok(v["present"] == true && v["visible"] == true && v["hit"] == true && v["enabled"] == true
            && v["label"] == "처음부터 다시보기" && v["text"] == "다시보기" && v["svg"] == true
            && v["centerDx"].as_f64().is_some_and(|d| d <= 2.0) && v["centerDy"].as_f64().is_some_and(|d| d <= 2.0))
    })?;
    let center = evaluate(&remote, CENTER_REPLAY)?;
    let before = clock(app, &token)?;
    let frames = evaluate(&remote, FULL_VIDEO)?["frames"].as_u64().ok_or("QA_CENTER_FRAMES")?;
    capture(&remote, &output.join("actual42-center-eof-video.png"))?;
    if evaluate(&remote, CLICK_CENTER_REPLAY)? != true { return Err("QA_CENTER_CLICK".into()); }
    let restart = (Instant::now() + Duration::from_secs(8)).min(state.deadline);
    let mut first_position = None;
    wait_until(restart, || {
        let Ok(observed) = clock(app, &token) else { return Ok(false); };
        full_identity(state, &observed)?;
        if observed.document_epoch != before.document_epoch || observed.fields.source_id != before.fields.source_id
            || observed.fields.source_generation != before.fields.source_generation { return Err("QA_CENTER_SOURCE_REPLACED".into()); }
        if observed.fields.ended || observed.fields.paused || observed.fields.sequence <= before.fields.sequence { return Ok(false); }
        let t = observed.fields.current_time.ok_or("QA_CENTER_TIME")?;
        if t >= 3.0 { return Ok(false); }
        let start = *first_position.get_or_insert(t);
        let video = evaluate(&remote, FULL_VIDEO)?;
        Ok(t > start + 0.15 && video["frames"].as_u64().is_some_and(|value| value > frames)
            && evaluate(&main, FULL_DOM)?["rows"] == 0 && evaluate(&remote, CENTER_REPLAY)?["visible"] == false)
    })?;
    if state.video_tokens.lock().unwrap().as_slice() != [token.clone()] || state.leases.lock().unwrap().as_slice() != [lease] { return Err("QA_CENTER_SESSION_REPLACED".into()); }
    let observed = clock(app, &token)?;
    evaluate(&main, "(()=>{window.__atsumiActual42Qa.close();return true})()")?;
    wait_until((Instant::now()+Duration::from_secs(5)).min(state.deadline), || Ok(state.leases.lock().unwrap().is_empty()))?;
    Ok(json!({"passed":true,"naturalEofObserved":true,"centerControl":center,"centerClickWorked":true,
        "restartedMediaTime":observed.fields.current_time,"savedChatReset":true,"sameNativeSession":true,
        "newUpload":false,"localVideoBytesRead":0,"desktopInput":false,"audioAudibilityVerified":false,
        "donationAvVerified":false,"screenshotRequiresRootVisualInspection":true}))
}

#[test]
#[ignore = "Actual existing SOOP42 central replay after natural EOF; separate explicit scoped environment required"]
fn actual_soop42_native_center_replay() {
    assert_eq!(std::env::var("ATSUMI_SOOP_NATIVE_CENTER_REPLAY_QA").as_deref(), Ok("actual-unlisted-center-readonly"), "QA_OPT_IN");
    let host = ActualReceiptHost::open_current();
    let authorization = host.authorization(); let job = host.job_view();
    let catalog = host.profile().join("platform-media/jobs").join(format!("{}.json", actual::RECORDING));
    let raw = host.read(&catalog, 16 * 1024); assert_eq!(actual::digest(&raw), CATALOG_SHA, "QA_CATALOG");
    let source_job: MediaJob = serde_json::from_slice(&raw).unwrap();
    let directory = storage::job_dir(&source_job.download_root, "soop", actual::RECORDING).unwrap();
    let before = snapshot(&host, &directory);
    let media = no_workers(host.scratch(), source_job);
    media.validate_cloud_chat_source(&authorization).unwrap_or_else(|_| panic!("QA_SOURCE"));
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let output = workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume").join(format!("actual42-center-native-{run_id}"));
    fs::create_dir(&output).unwrap();
    let identity = format!("local.atsumi.cloudreplayqa.{run_id}");
    let profile = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(&identity);
    assert!(!profile.exists(), "QA_PROFILE_EXISTS");
    let url: tauri::Url = std::env::var("ATSUMI_SOOP_NATIVE_CENTER_REPLAY_QA_URL").unwrap_or_else(|_| format!("http://127.0.0.1:5181{ENTRY}")).parse().unwrap();
    assert!(url.scheme()=="http" && url.host_str()==Some("127.0.0.1") && url.port()==Some(5181) && url.path()==ENTRY
        && url.username().is_empty() && url.password().is_none() && url.query().is_none() && url.fragment().is_none(), "QA_ENTRY");
    let state = Arc::new(QaState {media:media.clone(),authorization,job,leases:Mutex::new(Vec::new()),video_tokens:Mutex::new(Vec::new()),deadline:Instant::now()+Duration::from_secs(75),closed:AtomicBool::new(false)});
    let outcome = Arc::new(Mutex::new(None::<Result<Value,String>>));
    let outcome_worker = outcome.clone(); let worker_state = state.clone(); let output_worker = output.clone(); let profile_check = profile.clone();
    let old_args = std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS");
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "--mute-audio");
    let mut context = tauri::generate_context!();
    context.config_mut().identifier=identity;
    context.config_mut().build.dev_url=Some(url.clone());
    context.config_mut().app.windows.clear(); context.config_mut().app.tray_icon=None;
    let app = tauri::Builder::default().any_thread().manage(state.clone()).manage(YouTubeOriginalHost::default())
        .invoke_handler(tauri::generate_handler![qa_actual42_bootstrap,qa_actual42_describe,qa_actual42_open,qa_actual42_page,qa_actual42_asset,qa_actual42_close,youtube_original_open,
            crate::youtube::original_commands::youtube_original_snapshot,crate::youtube::original_commands::youtube_original_vod_clock,crate::youtube::original_commands::youtube_original_vod_control,
            crate::youtube::original_commands::youtube_original_viewport,crate::youtube::original_commands::youtube_original_close,crate::youtube::original_commands::youtube_original_controls,crate::youtube::original_commands::youtube_original_fullscreen])
        .setup(move |app| {
            if app.path().app_data_dir()? != profile_check { return Err("QA_PROFILE_PATH".into()); }
            tauri::WebviewWindowBuilder::new(app,"main",tauri::WebviewUrl::External(url)).data_directory(profile_check.join("main-webview")).title("Atsumi native replay verification").inner_size(1280.,760.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
            let handle=app.handle().clone();
            thread::spawn(move || {
                let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| exercise_center_replay(&handle,&worker_state,&output_worker)))
                    .unwrap_or_else(|_| Err("QA_WORKER_PANIC".into()));
                if value.is_err() {
                    let main = handle.get_webview("main");
                    let remote = handle.webviews().into_values().find(|v| v.label().starts_with("youtube-original-vod-"));
                    let facts = json!({"main":main.as_ref().and_then(|view|evaluate(view,DOM).ok()),
                        "video":remote.as_ref().and_then(|view|evaluate(view,VIDEO).ok()),
                        "center":remote.as_ref().and_then(|view|evaluate(view,CENTER_REPLAY).ok()),
                        "centerEnvironment":remote.as_ref().and_then(|view|evaluate(view,CENTER_FAILURE_ENV).ok()),
                        "centerOcclusion":remote.as_ref().and_then(|view|evaluate(view,CENTER_OCCLUSION_ENV).ok())});
                    let _=fs::write(output_worker.join("failure-facts.json"),serde_json::to_vec_pretty(&facts).unwrap());
                    if let Some(view)=main {let _=capture(&view,&output_worker.join("failure-main.png"));}
                    if let Some(view)=remote {let _=capture(&view,&output_worker.join("failure-video.png"));}
                }
                worker_state.closed.store(true,Ordering::Release);
                for token in worker_state.leases.lock().unwrap().drain(..) { worker_state.media.close_cloud_chat(&token); }
                let receiver=handle.state::<YouTubeOriginalHost>();
                for token in worker_state.video_tokens.lock().unwrap().drain(..) { let _=receiver.close(&handle,&token); }
                receiver.shutdown(&handle);
                *outcome_worker.lock().unwrap()=Some(value);
                handle.exit(0);
            }); Ok(())
        }).build(context).unwrap_or_else(|_| panic!("QA_NATIVE_BUILD"));
    app.run_return(|_,_|{});
    if let Some(value)=old_args {std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",value)} else {std::env::remove_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")}
    assert_eq!(snapshot(&host,&directory),before,"QA_SOURCE_SIDECARS_CHANGED");
    assert_eq!(actual::digest(&host.read(&catalog,16*1024)),CATALOG_SHA,"QA_CATALOG_CHANGED");
    assert!(media.inner.playback.lock().unwrap().is_empty() && media.inner.cloud_chat.lock().unwrap().is_empty() && media.inner.upload_leases.lock().unwrap().is_empty(),"QA_LEASE_REMAINS");
    { let saved=media.inner.state.lock().unwrap(); assert!(saved.queue.is_empty()&&saved.cancel.is_empty()&&!saved.shutting_down,"QA_WORK_STARTED"); }
    host.assert_unchanged(); drop(state); drop(media); let read_bytes=host.finish();
    // Only the fresh UUID profile may be cleaned; a busy WebView profile is retained.
    let cleaned = cleanup_profile(&profile,&format!("local.atsumi.cloudreplayqa.{run_id}"));
    let result=outcome.lock().unwrap().take().unwrap_or_else(||Err("QA_NO_RESULT".into()));
    let report=match &result {Ok(value)=>json!({"result":value,"sourceHashesUnchanged":true,"freshProfileCleaned":cleaned,"sourceAndScratchReadBytes":read_bytes}),Err(code)=>json!({"passed":false,"errorCode":code,"sourceHashesUnchanged":true,"freshProfileCleaned":cleaned,"sourceAndScratchReadBytes":read_bytes})};
    fs::write(output.join("result.json"),serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("QA_NATIVE_CENTER_RESULT={}",output.join("result.json").display());
    assert!(result.is_ok(),"Actual native playback unverified; inspect sanitized result");
}

