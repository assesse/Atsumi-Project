//! Opt-in actual native production React/YouTube document + exact SOOP118 Private receipt.
//! No production hook, upload, account action or desktop input.
use super::*;
use crate::cloud_recording::{
    replay::{CloudReplayAuthorization, CloudReplayOpen},
    CloudJobSnapshot,
};
use crate::youtube::original::{
    OriginalSnapshot, VodClock, VodControlRequest, YouTubeOriginalHost,
};
use serde_json::Value;
use std::{
    io::Write,
    sync::atomic::AtomicBool,
    thread,
    time::{Duration, Instant},
};
use tauri::{Manager, State, Webview};

const ENTRY: &str =
    "/.runtime/requirements-audit/20261122-cloud-resume/private118-native-replay.html";
struct QaState {
    media: PlatformMediaService,
    authorization: CloudReplayAuthorization,
    job: CloudJobSnapshot,
    leases: Mutex<Vec<String>>,
    video_tokens: Mutex<Vec<String>>,
    deadline: Instant,
    closed: AtomicBool,
}
fn guard(window: &Webview, state: &QaState) -> Result<tauri::AppHandle, String> {
    let app = window.app_handle();
    let url = window.url().map_err(|_| "QA_WINDOW")?;
    if state.closed.load(Ordering::Acquire)
        || Instant::now() >= state.deadline
        || url.path() != ENTRY
        || !crate::frontend_origin::trusted_main(
            window.label(),
            &url,
            true,
            app.config().build.dev_url.as_ref(),
        )
    {
        return Err("QA_SCOPE".into());
    }
    Ok(app.clone())
}
fn job_allowed(state: &QaState, id: &str) -> Result<(), String> {
    if id == state.job.id {
        Ok(())
    } else {
        Err("QA_JOB".into())
    }
}
fn lease_allowed(state: &QaState, token: &str) -> Result<(), String> {
    if state
        .leases
        .lock()
        .unwrap()
        .iter()
        .any(|value| value == token)
    {
        Ok(())
    } else {
        Err("QA_LEASE".into())
    }
}
#[tauri::command]
fn qa_private118_bootstrap(
    window: Webview,
    state: State<'_, Arc<QaState>>,
) -> Result<Value, String> {
    guard(&window, &state)?;
    Ok(json!({"job":state.job,"expectedRows":93,"durationSeconds":118.177334}))
}
#[tauri::command]
fn qa_private118_describe(
    window: Webview,
    state: State<'_, Arc<QaState>>,
    job_id: String,
) -> Result<crate::cloud_recording::source::SourceDescription, String> {
    guard(&window, &state)?;
    job_allowed(&state, &job_id)?;
    Ok(state.authorization.description.clone())
}
#[tauri::command]
fn qa_private118_open(
    window: Webview,
    state: State<'_, Arc<QaState>>,
    job_id: String,
) -> Result<CloudReplayOpen, String> {
    guard(&window, &state)?;
    job_allowed(&state, &job_id)?;
    let mut leases = state.leases.lock().unwrap();
    if leases.len() >= 2 {
        return Err("QA_LEASE_LIMIT".into());
    }
    let token = state
        .media
        .open_cloud_chat(&state.authorization)
        .map_err(|_| "QA_SIDECAR_OPEN")?;
    leases.push(token.clone());
    Ok(CloudReplayOpen {
        provider: "soop".into(),
        token,
        cohort_id: state.authorization.cohort_id.clone(),
        description: state.authorization.description.clone(),
        session: None,
    })
}
#[tauri::command]
fn qa_private118_page(
    window: Webview,
    state: State<'_, Arc<QaState>>,
    token: String,
    position_ms: u64,
    cursor: Option<String>,
    limit: usize,
) -> Result<replay_chat::ReplayChatPage, String> {
    guard(&window, &state)?;
    lease_allowed(&state, &token)?;
    if position_ms > 119_000
        || limit > 120
        || limit == 0
        || cursor.as_ref().is_some_and(|v| v.len() > 1024)
    {
        return Err("QA_PAGE_BOUND".into());
    }
    state
        .media
        .cloud_chat_page(&token, position_ms, cursor.as_deref(), limit)
        .map_err(|_| "QA_PAGE".into())
}
#[tauri::command]
fn qa_private118_asset(
    window: Webview,
    state: State<'_, Arc<QaState>>,
    token: String,
    asset_id: String,
) -> Result<live_chat::ChatAssetRead, String> {
    guard(&window, &state)?;
    lease_allowed(&state, &token)?;
    state
        .media
        .cloud_chat_asset(&token, &asset_id)
        .map_err(|_| "QA_ASSET".into())
}
#[tauri::command]
fn qa_private118_close(
    window: Webview,
    state: State<'_, Arc<QaState>>,
    provider: String,
    token: String,
) -> Result<(), String> {
    guard(&window, &state)?;
    if provider != "soop" {
        return Err("QA_PROVIDER".into());
    }
    state.media.close_cloud_chat(&token);
    state.leases.lock().unwrap().retain(|value| value != &token);
    Ok(())
}
// Only the normal command's AppState work-admission wrapper differs in this test.
// The real receiver, native document bridge, clock, geometry and controls are used.
#[tauri::command]
async fn youtube_original_open(
    window: Webview,
    state: State<'_, Arc<QaState>>,
    channel_id: String,
    request_id: String,
    playback_kind: Option<String>,
) -> Result<OriginalSnapshot, String> {
    let app = guard(&window, &state)?;
    if playback_kind.as_deref() != Some("vod")
        || state.job.video_id.as_deref() != Some(channel_id.as_str())
    {
        return Err("QA_VIDEO".into());
    }
    let owned = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let value = app
            .state::<YouTubeOriginalHost>()
            .open_vod(&app, channel_id, request_id)?;
        owned.video_tokens.lock().unwrap().push(value.token.clone());
        Ok(value)
    })
    .await
    .map_err(|_| "QA_OPEN_WORKER".to_owned())?
}

fn evaluate(view: &Webview, script: &str) -> Result<Value, String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(script, move |raw| {
        let _ = tx.try_send((raw.len() <= 8192).then_some(raw));
    })
    .map_err(|_| "QA_EVAL")?;
    let raw = rx
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| "QA_EVAL_TIMEOUT")?
        .ok_or("QA_EVAL_BOUND")?;
    serde_json::from_str(&raw).map_err(|_| "QA_EVAL_JSON".into())
}
const DOM: &str = r#"(()=>{const q=window.__atsumiPrivate118Qa,h=document.querySelector('aside[data-provider="soop"]'),s=h?.shadowRoot,area=s?.querySelector('#chatArea'),imgs=Array.from(s?.querySelectorAll('.donation-container.subscribe .custom-mobile-img img')??[]),row=s?.querySelector('.chat_memoyo'),notice=document.querySelector('.cloud-recording-capture-warning'),nr=notice?.getBoundingClientRect(),hr=h?.getBoundingClientRect();return {ready:q?.ready===true,dialog:!!document.querySelector('.cloud-recording-replay'),shadow:!!s,rows:row?.querySelectorAll('.platform-replay-chat-row').length??0,gifts:row?.querySelectorAll('.donation-container.subscribe').length??0,customImages:imgs.length,customImagesLoaded:imgs.filter(i=>i.complete&&i.naturalWidth>0&&i.naturalHeight>0&&i.getAttribute('src')?.startsWith('data:image/')).length,loading:!!s?.querySelector('.soop-replay-status[role="status"]'),overflow:!!area&&area.scrollWidth>area.clientWidth+1,errors:document.querySelectorAll('.cloud-recording-error:not(.cloud-recording-capture-warning)').length,partialNotice:!!notice&&!!nr&&nr.width>0&&nr.height>0&&nr.top>=0&&nr.bottom<=innerHeight+1,chatHeight:hr?.height??0,chatWithinViewport:!!hr&&hr.height>200&&hr.top>=0&&hr.bottom<=innerHeight+1,hasEnabledSend:!!s?.querySelector('.btn_send:not(:disabled)')}})()"#;
const VIDEO: &str = r#"(()=>{const p=document.getElementById('movie_player'),v=p?.querySelector('video'),s=p?.getPlayerResponse?.()?.playabilityStatus?.status;if(v)v.muted=true;return {ready:v?.readyState??0,time:v?.currentTime??null,duration:Number.isFinite(v?.duration)?v.duration:null,paused:v?.paused??null,frames:v?.getVideoPlaybackQuality?.().totalVideoFrames??0,ad:!!p?.classList.contains('ad-showing'),capture:!!window.__atsumiYouTubeEncodedCapture,chat:!!document.querySelector('iframe#chatframe'),playabilityStatus:['OK','LOGIN_REQUIRED','ERROR','UNPLAYABLE','CONTENT_CHECK_REQUIRED','AGE_CHECK_REQUIRED','LIVE_STREAM_OFFLINE'].includes(s)?s:null}})()"#;
fn wait_until(
    deadline: Instant,
    mut read: impl FnMut() -> Result<bool, String>,
) -> Result<(), String> {
    loop {
        if Instant::now() >= deadline {
            return Err("QA_OBSERVATION_TIMEOUT".into());
        }
        if read()? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(150));
    }
}
fn clock(app: &tauri::AppHandle, token: &str) -> Result<VodClock, String> {
    app.state::<YouTubeOriginalHost>()
        .vod_clock(token)?
        .filter(|v| v.fields.valid && v.fields.ad_state == "content" && v.age_ms <= 1500)
        .ok_or("QA_CONTENT_CLOCK".into())
}
fn control(
    app: &tauri::AppHandle,
    token: &str,
    action: &str,
    seconds: Option<f64>,
) -> Result<(), String> {
    let observed = clock(app, token)?;
    app.state::<YouTubeOriginalHost>()
        .vod_control(
            app,
            token,
            VodControlRequest {
                request_id: uuid::Uuid::new_v4().to_string(),
                document_epoch: observed.document_epoch,
                source_id: observed.fields.source_id,
                source_generation: observed.fields.source_generation,
                action: action.into(),
                seconds,
                rate: None,
            },
        )
        .map_err(|_| "QA_NATIVE_CONTROL".into())
}
fn capture_document(view: &Webview, state: &QaState) -> Result<tauri::Url, String> {
    let url = view.url().map_err(|_| "QA_CAPTURE_DOCUMENT")?;
    let main = view.label() == "main"
        && url.path() == ENTRY
        && crate::frontend_origin::trusted_main(
            view.label(),
            &url,
            true,
            view.app_handle().config().build.dev_url.as_ref(),
        );
    let official = view.label().starts_with("youtube-original-vod-")
        && url.scheme() == "https"
        && url.host_str() == Some("www.youtube.com")
        && url.path() == "/watch"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query_pairs().filter(|(key, _)| key == "v").count() == 1
        && url.query_pairs().any(|(key, value)| {
            key == "v" && state.job.video_id.as_deref() == Some(value.as_ref())
        });
    if !main && !official {
        return Err("QA_CAPTURE_DOCUMENT".into());
    }
    if evaluate(
        view,
        r#"(()=>!document.querySelector('input[type="password"],input[autocomplete="current-password"],input[autocomplete="new-password"]'))()"#,
    )? != Value::Bool(true)
    {
        return Err("QA_CAPTURE_AUTH_FORM".into());
    }
    Ok(url)
}
fn capture(view: &Webview, path: &Path, state: &QaState) -> Result<(), String> {
    let original_url = capture_document(view, state)?;
    use webview2_com::{
        CapturePreviewCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
    };
    use windows::Win32::{
        System::Com::{STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET},
        UI::Shell::SHCreateMemStream,
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let Some(stream) = SHCreateMemStream(None) else {
            let _ = tx.try_send(Err("QA_CAPTURE_STREAM"));
            return;
        };
        let data = stream.clone();
        let completed = tx.clone();
        let callback = CapturePreviewCompletedHandler::create(Box::new(move |status| {
            let result = (|| -> Result<Vec<u8>, &'static str> {
                status.map_err(|_| "QA_CAPTURE")?;
                let mut stat = STATSTG::default();
                data.Stat(&mut stat, STATFLAG_NONAME)
                    .map_err(|_| "QA_CAPTURE_STAT")?;
                if stat.cbSize == 0 || stat.cbSize > 16 * 1024 * 1024 {
                    return Err("QA_CAPTURE_BOUND");
                }
                data.Seek(0, STREAM_SEEK_SET, None)
                    .map_err(|_| "QA_CAPTURE_SEEK")?;
                let mut bytes = vec![0; stat.cbSize as usize];
                let mut read = 0;
                data.Read(
                    bytes.as_mut_ptr().cast(),
                    bytes.len() as u32,
                    Some(&mut read),
                )
                .ok()
                .map_err(|_| "QA_CAPTURE_READ")?;
                if read as usize != bytes.len() || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                    return Err("QA_CAPTURE_PNG");
                }
                Ok(bytes)
            })();
            let _ = completed.try_send(result);
            Ok(())
        }));
        if platform
            .controller()
            .CoreWebView2()
            .and_then(|core| {
                core.CapturePreview(
                    COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                    &stream,
                    &callback,
                )
            })
            .is_err()
        {
            let _ = tx.try_send(Err("QA_CAPTURE_DISPATCH"));
        }
    })
    .map_err(|_| "QA_CAPTURE_DISPATCH")?;
    let bytes = rx
        .recv_timeout(Duration::from_secs(6))
        .map_err(|_| "QA_CAPTURE_TIMEOUT")?
        .map_err(str::to_owned)?;
    if capture_document(view, state)? != original_url {
        return Err("QA_CAPTURE_DOCUMENT_CHANGED".into());
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "QA_CAPTURE_CREATE")?;
    file.write_all(&bytes).map_err(|_| "QA_CAPTURE_SAVE".into())
}

const RECORDING: &str = "3bae70dd-ca33-401b-aa95-a508bab9ffc4";
const PROFILE: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
const DURATION: f64 = 118.177334;
const EVENT_MS: u64 = 73_773;
const ASSETS: [&str; 2] = [
    "e5b241c41d5249ffd392e173f1d34f2152dcdd2f58aa16bdac22b5dec49b8eec",
    "e5bcef4a72446230d15080d0a420965322e4ad0631462861b872075415fe1677",
];
fn source_membership(state: &QaState, lease: &str) -> Result<(), String> {
    let mut done = false;
    let deadline = (Instant::now() + Duration::from_secs(8)).min(state.deadline);
    while Instant::now() < deadline && !state.closed.load(Ordering::Acquire) {
        let page = state
            .media
            .cloud_chat_page(lease, 119_000, None, 120)
            .map_err(|_| "QA_SOURCE_PAGE")?;
        if page.state == "indexing" {
            thread::sleep(Duration::from_millis(75));
            continue;
        }
        let events = page
            .items
            .iter()
            .filter(|row| row.kind == "membership")
            .collect::<Vec<_>>();
        if page.items.len() != 93 || events.len() != 1 || events[0].time_ms != Some(EVENT_MS) {
            return Err("QA_SOURCE_MEMBERSHIP_CHANGED".into());
        }
        let meta = events[0]
            .soop_subscription
            .as_ref()
            .ok_or("QA_SOURCE_MEMBERSHIP_METADATA")?;
        let mut actual = meta.ids();
        actual.sort_unstable();
        let mut expected = ASSETS.to_vec();
        expected.sort_unstable();
        if actual != expected {
            return Err("QA_SOURCE_MEMBERSHIP_ASSETS".into());
        }
        for id in ASSETS {
            let image = state
                .media
                .cloud_chat_asset(lease, id)
                .map_err(|_| "QA_SOURCE_ASSET")?;
            if image.state != "ready"
                || image
                    .data_url
                    .as_deref()
                    .is_none_or(|value| !value.starts_with("data:image/"))
            {
                return Err("QA_SOURCE_ASSET_NOT_LOCAL".into());
            }
        }
        done = true;
        break;
    }
    if !done {
        return Err("QA_SOURCE_PAGE_PREPARATION_BOUND".into());
    }
    Ok(())
}
fn membership_seek(
    app: &tauri::AppHandle,
    state: &QaState,
    main: &Webview,
    token: &str,
    seconds: f64,
    count: u64,
) -> Result<Value, String> {
    control(app, token, "seek", Some(seconds))?;
    wait_until(
        (Instant::now() + Duration::from_secs(10)).min(state.deadline),
        || {
            let Ok(c) = clock(app, token) else {
                return Ok(false);
            };
            let ui = evaluate(main, DOM)?;
            Ok(c.fields.paused
                && !c.fields.seeking
                && c.fields
                    .current_time
                    .is_some_and(|t| (t - seconds).abs() < 0.2)
                && ui["ready"] == true
                && ui["loading"] == false
                && ui["gifts"] == count
                && ui["errors"] == 0)
        },
    )?;
    Ok(
        json!({"requestedSeconds":seconds,"actualMediaSeconds":clock(app,token)?.fields.current_time,"ui":evaluate(main,DOM)?}),
    )
}
fn exercise(app: &tauri::AppHandle, state: &QaState, output: &Path) -> Result<Value, String> {
    let main = app.get_webview("main").ok_or("QA_MAIN")?;
    let ready = (Instant::now() + Duration::from_secs(60)).min(state.deadline);
    wait_until(ready, || {
        Ok(evaluate(&main, DOM)?["ready"] == true
            && !state.video_tokens.lock().unwrap().is_empty()
            && !state.leases.lock().unwrap().is_empty())
    })?;
    let token = state
        .video_tokens
        .lock()
        .unwrap()
        .last()
        .cloned()
        .ok_or("QA_VIDEO_TOKEN")?;
    let lease = state
        .leases
        .lock()
        .unwrap()
        .last()
        .cloned()
        .ok_or("QA_LEASE")?;
    source_membership(state, &lease)?;
    wait_until(ready, || {
        Ok(clock(app, &token).is_ok_and(|c| {
            c.fields.video_id == state.job.video_id.as_deref().unwrap_or("")
                && c.fields
                    .duration
                    .is_some_and(|d| (d - DURATION).abs() < 1.0)
        }))
    })?;
    let remote = app
        .webviews()
        .into_values()
        .find(|v| v.label().starts_with("youtube-original-vod-"))
        .ok_or("QA_REMOTE_VIEW")?;
    let first = clock(app, &token)?;
    wait_until(ready, || {
        let Ok(c) = clock(app, &token) else {
            return Ok(false);
        };
        Ok(c.fields
            .current_time
            .zip(first.fields.current_time)
            .is_some_and(|(a, b)| a > b + 0.7)
            && evaluate(&remote, VIDEO)?["frames"]
                .as_u64()
                .is_some_and(|n| n > 0))
    })?;
    control(app, &token, "pause", None)?;
    wait_until(ready, || {
        Ok(clock(app, &token).is_ok_and(|c| c.fields.paused))
    })?;
    let mut samples = Vec::new();
    for (seconds, count) in [(0., 0), (73.2, 0), (74.4, 1), (73.2, 0), (74.4, 1)] {
        samples.push(membership_seek(app, state, &main, &token, seconds, count)?);
    }
    wait_until(
        (Instant::now() + Duration::from_secs(8)).min(state.deadline),
        || {
            let ui = evaluate(&main, DOM)?;
            Ok(ui["gifts"] == 1 && ui["customImages"] == 1 && ui["customImagesLoaded"] == 1)
        },
    )?;
    let presentation = evaluate(&main, DOM)?;
    if presentation["partialNotice"] != true
        || presentation["overflow"] != false
        || presentation["hasEnabledSend"] != false
        || presentation["chatWithinViewport"] != true
    {
        return Err("QA_MEMBERSHIP_PRESENTATION".into());
    }
    let pause_before = clock(app, &token)?;
    thread::sleep(Duration::from_millis(800));
    let pause_after = clock(app, &token)?;
    let drift = (pause_after.fields.current_time.unwrap_or(-1.)
        - pause_before.fields.current_time.unwrap_or(-2.))
    .abs();
    if drift > 0.15 || evaluate(&main, DOM)?["gifts"] != 1 {
        return Err("QA_PAUSE_DRIFT".into());
    }
    capture(&main, &output.join("private118-membership-chat.png"), state)?;
    capture(
        &remote,
        &output.join("private118-membership-video.png"),
        state,
    )?;
    membership_seek(app, state, &main, &token, 72.5, 0)?;
    let before_frames = evaluate(&remote, VIDEO)?["frames"].as_u64().unwrap_or(0);
    control(app, &token, "resume", None)?;
    wait_until(
        (Instant::now() + Duration::from_secs(10)).min(state.deadline),
        || {
            let Ok(c) = clock(app, &token) else {
                return Ok(false);
            };
            let ui = evaluate(&main, DOM)?;
            Ok(!c.fields.paused
                && c.fields.current_time.is_some_and(|t| t > 74.1 && t < 79.)
                && ui["gifts"] == 1
                && evaluate(&remote, VIDEO)?["frames"]
                    .as_u64()
                    .is_some_and(|n| n > before_frames))
        },
    )?;
    control(app, &token, "pause", None)?;
    wait_until(
        (Instant::now() + Duration::from_secs(5)).min(state.deadline),
        || Ok(clock(app, &token).is_ok_and(|c| c.fields.paused)),
    )?;
    if state.leases.lock().unwrap().as_slice() != [lease.clone()] {
        return Err("QA_LEASE_CHANGED".into());
    }
    evaluate(
        &main,
        "(()=>{window.__atsumiPrivate118Qa.close();return true})()",
    )?;
    wait_until(
        (Instant::now() + Duration::from_secs(8)).min(state.deadline),
        || Ok(state.leases.lock().unwrap().is_empty() && evaluate(&main, DOM)?["dialog"] == false),
    )?;
    if state.media.cloud_chat_page(&lease, 0, None, 1).is_ok() {
        return Err("QA_CLOSED_LEASE_REUSABLE".into());
    }
    evaluate(
        &main,
        "(()=>{window.__atsumiPrivate118Qa.reopen();return true})()",
    )?;
    wait_until(
        (Instant::now() + Duration::from_secs(20)).min(state.deadline),
        || {
            Ok(state
                .video_tokens
                .lock()
                .unwrap()
                .last()
                .is_some_and(|t| t != &token)
                && !state.leases.lock().unwrap().is_empty())
        },
    )?;
    let next = state
        .video_tokens
        .lock()
        .unwrap()
        .last()
        .cloned()
        .ok_or("QA_REOPEN")?;
    wait_until(
        (Instant::now() + Duration::from_secs(20)).min(state.deadline),
        || {
            Ok(clock(app, &next)
                .is_ok_and(|c| c.fields.video_id == state.job.video_id.as_deref().unwrap_or("")))
        },
    )?;
    control(app, &next, "pause", None)?;
    wait_until(
        (Instant::now() + Duration::from_secs(5)).min(state.deadline),
        || Ok(clock(app, &next).is_ok_and(|c| c.fields.paused)),
    )?;
    samples.push(membership_seek(app, state, &main, &next, 74.4, 1)?);
    wait_until(
        (Instant::now() + Duration::from_secs(8)).min(state.deadline),
        || {
            let ui = evaluate(&main, DOM)?;
            Ok(ui["customImagesLoaded"] == 1 && ui["partialNotice"] == true)
        },
    )?;
    let reopened = evaluate(&main, DOM)?;
    evaluate(
        &main,
        "(()=>{window.__atsumiPrivate118Qa.close();return true})()",
    )?;
    wait_until(
        (Instant::now() + Duration::from_secs(5)).min(state.deadline),
        || Ok(state.leases.lock().unwrap().is_empty()),
    )?;
    Ok(
        json!({"passed":true,"visibility":"private","actualDurableReceipt":true,"actualRemoteVideoProgress":true,"sameReceiptVideo":true,"recordedMembershipTimeMs":EVENT_MS,"sourceRows":93,"sourceCapturedAssets":2,"renderedCapturedPrimaryImage":true,"boundarySamples":samples,"presentation":presentation,"reopened":reopened,"naturalMembershipCrossing":true,"pauseDriftSeconds":drift,"closeRevokesSidecar":true,"partialCaptureNotice":true,"timelineQuality":"receive-time-approximate","framePerfectSyncVerified":false,"historicalDisplayExactnessVerified":false,"audioAudibilityVerified":false,"localVideoPlaybackReads":0,"newUpload":false,"loginActions":0,"desktopInput":0,"fullClipPlayed":false}),
    )
}

#[test]
#[ignore = "Exact ready Private SOOP118 receipt, existing isolated website profile, no account/upload action; explicit opt-in"]
fn actual_private_soop118_remote_video_membership_and_reopen() -> Result<(), String> {
    if std::env::var("ATSUMI_SOOP_PRIVATE_REPLAY_QA").as_deref() != Ok(RECORDING) {
        return Err("QA_PRIVATE_REPLAY_OPT_IN".into());
    }
    let mut host =
        crate::cloud_recording::replay_receipt_private_actual::PrivateReceiptHost::open()?;
    let media = no_workers(host.data(), host.source_job());
    let authorization = host.authorization();
    let job = host.job();
    media
        .validate_cloud_chat_source(&authorization)
        .map_err(|_| "QA_SOURCE")?;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("QA_WORKSPACE")?;
    let output = workspace
        .join(".runtime/requirements-audit/20261122-cloud-resume")
        .join(format!(
            "private118-native-{}",
            uuid::Uuid::new_v4().simple()
        ));
    fs::create_dir(&output).map_err(|_| "QA_OUTPUT")?;
    let profile = host.data().to_owned();
    let main_profile = output.join("main-webview");
    let url: tauri::Url = format!("http://127.0.0.1:5181{ENTRY}")
        .parse()
        .map_err(|_| "QA_ENTRY")?;
    let state = Arc::new(QaState {
        media: media.clone(),
        authorization,
        job,
        leases: Mutex::new(Vec::new()),
        video_tokens: Mutex::new(Vec::new()),
        deadline: Instant::now() + Duration::from_secs(180),
        closed: AtomicBool::new(false),
    });
    let outcome = Arc::new(Mutex::new(None::<Result<Value, String>>));
    let outcome_worker = outcome.clone();
    let worker_state = state.clone();
    let worker_output = output.clone();
    let worker = Arc::new(Mutex::new(None::<thread::JoinHandle<()>>));
    let setup_worker = worker.clone();
    let exited = Arc::new(AtomicBool::new(false));
    let exit_flag = exited.clone();
    let old_args = std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS");
    struct RestoreArgs(Option<std::ffi::OsString>);
    impl Drop for RestoreArgs {
        fn drop(&mut self) {
            if let Some(v) = &self.0 {
                std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", v)
            } else {
                std::env::remove_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")
            }
        }
    }
    let _restore = RestoreArgs(old_args);
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "--mute-audio");
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = PROFILE.into();
    context.config_mut().build.dev_url = Some(url.clone());
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app=tauri::Builder::default().any_thread().manage(state.clone()).manage(YouTubeOriginalHost::default())
        .invoke_handler(tauri::generate_handler![qa_private118_bootstrap,qa_private118_describe,qa_private118_open,qa_private118_page,qa_private118_asset,qa_private118_close,youtube_original_open,
            crate::youtube::original_commands::youtube_original_snapshot,crate::youtube::original_commands::youtube_original_vod_clock,crate::youtube::original_commands::youtube_original_vod_control,
            crate::youtube::original_commands::youtube_original_viewport,crate::youtube::original_commands::youtube_original_close,crate::youtube::original_commands::youtube_original_controls,crate::youtube::original_commands::youtube_original_fullscreen])
        .setup(move|app|{
            if app.path().app_data_dir()?!=profile{return Err("QA_PROFILE_PATH".into());}
            tauri::WebviewWindowBuilder::new(app,"main",tauri::WebviewUrl::External(url)).data_directory(main_profile).title("Atsumi isolated Private replay verification").inner_size(1280.,760.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
            let handle=app.handle().clone();let join=thread::spawn(move||{
                let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||exercise(&handle,&worker_state,&worker_output))).unwrap_or_else(|_|Err("QA_WORKER_PANIC".into()));
                if result.is_err(){
                    let main=handle.get_webview("main");let remote=handle.webviews().into_values().find(|v|v.label().starts_with("youtube-original-vod-"));
                    let facts=json!({"main":main.as_ref().and_then(|v|evaluate(v,DOM).ok()),"video":remote.as_ref().and_then(|v|evaluate(v,VIDEO).ok())});
                    let _=fs::write(worker_output.join("failure-facts.json"),serde_json::to_vec_pretty(&facts).unwrap());
                    if let Some(v)=main{let _=capture(&v,&worker_output.join("failure-main.png"),&worker_state);}if let Some(v)=remote{let _=capture(&v,&worker_output.join("failure-video.png"),&worker_state);}
                }
                worker_state.closed.store(true,Ordering::Release);
                for token in worker_state.leases.lock().unwrap().drain(..){worker_state.media.close_cloud_chat(&token);}
                let receiver=handle.state::<YouTubeOriginalHost>();for token in worker_state.video_tokens.lock().unwrap().drain(..){let _=receiver.close(&handle,&token);}receiver.shutdown(&handle);
                *outcome_worker.lock().unwrap()=Some(result);handle.exit(0);
            });*setup_worker.lock().map_err(|_|"QA_THREAD_LOCK")?=Some(join);Ok(())
        }).build(context).map_err(|_|"QA_APP_BUILD")?;
    let exit_code = app.run_return(move |_, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            exit_flag.store(true, Ordering::Release);
        }
    });
    let joined = worker
        .lock()
        .map_err(|_| "QA_THREAD_LOCK")?
        .take()
        .ok_or("QA_NO_WORKER")?
        .join()
        .is_ok();
    let normal_exit = exit_code == 0 && exited.load(Ordering::Acquire) && joined;
    let unchanged = host.unchanged();
    let no_leases = media.inner.playback.lock().unwrap().is_empty()
        && media.inner.cloud_chat.lock().unwrap().is_empty()
        && media.inner.upload_leases.lock().unwrap().is_empty();
    let no_work = {
        let v = media.inner.state.lock().unwrap();
        v.queue.is_empty() && v.cancel.is_empty() && !v.shutting_down
    };
    let result = outcome
        .lock()
        .unwrap()
        .take()
        .unwrap_or_else(|| Err("QA_NO_RESULT".into()));
    let report = json!({"result":result.as_ref().ok(),"errorCode":result.as_ref().err(),"original37AndCatalogAndPlanUnchanged":unchanged.is_ok(),"normalExit":normal_exit,"workerJoined":joined,"noLeasesRemaining":no_leases,"noWorkerStarted":no_work,"existingWebsiteProfileRetained":true,"profileCopied":false,"originalsDeleted":false});
    let target = output.join("result.json");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|_| "QA_REPORT")?;
    file.write_all(&serde_json::to_vec_pretty(&report).map_err(|_| "QA_REPORT")?)
        .and_then(|_| file.sync_all())
        .map_err(|_| "QA_REPORT")?;
    println!("QA_PRIVATE118_REPLAY_RESULT={}", target.display());
    unchanged?;
    if !normal_exit || !no_leases || !no_work {
        return Err("QA_FINAL_DRAIN_UNCONFIRMED".into());
    }
    result.map(|_| ())
}
