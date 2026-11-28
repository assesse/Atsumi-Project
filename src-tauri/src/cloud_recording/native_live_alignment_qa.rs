//! Opt-in native helper for an already completed selected Live receipt.
//! Parent is live_alignment: private matcher functions stay test-only here.
//! No catalog mutation, account action, media file, remote edit or deletion.
use super::*;
use super::super::{CloudService, live_archive::{ArchiveState, Identity}, live_sidecar::VerifiedLiveReferences};
use crate::{platform_native_geometry::{NativeViewport, Rect}, youtube::original::{NativeFrameWindow, VodClock, VodControlRequest, YouTubeOriginalHost}};
use serde_json::{json, Value};
use std::{sync::{Arc, atomic::{AtomicBool, Ordering}}, thread, time::{Duration, Instant}};
use tauri::{AppHandle, Manager, Webview};

#[path = "native_live_alignment_runner_qa.rs"]
mod runner;

type Checked<T> = std::result::Result<T, String>;
const MAX_CAPTURE_CALLS: usize = 24;
const MAX_SEARCH_CALLS: usize = 15;
const MAX_SECONDS: u64 = 210;

#[derive(Clone, PartialEq)]
struct Selected { identity: Identity, duration: f64 }
fn selected(service: &CloudService, selection: &str) -> Checked<Selected> {
    let inner = service.lock().map_err(|_| "QA_ALIGNMENT_CATALOG")?;
    let receipt = inner.catalog.live_receipts.iter().find(|r| r.view.selection_id == selection)
        .filter(|r| r.valid()).ok_or("QA_ALIGNMENT_SELECTION")?;
    let archive = receipt.archive.as_ref().filter(|r| r.valid() && r.state == ArchiveState::MetadataVerified)
        .ok_or("QA_ALIGNMENT_ARCHIVE_PENDING")?;
    let identity = receipt.archive_identity().filter(|id| id == &archive.identity)
        .ok_or("QA_ALIGNMENT_ARCHIVE_IDENTITY")?;
    let metadata = archive.snapshot();
    if metadata.video_id.as_deref() != Some(identity.broadcast_id.as_str()) || metadata.embeddable != Some(true) {
        return Err("QA_ALIGNMENT_ARCHIVE_IDENTITY".into());
    }
    let duration = metadata.duration_seconds.filter(|d| d.is_finite() && (20.0..=120.0).contains(d))
        .ok_or("QA_ALIGNMENT_SHORT_TRIAL_REQUIRED")?;
    Ok(Selected { identity, duration })
}
fn check(deadline: Instant, cancel: &AtomicBool) -> Checked<()> {
    if cancel.load(Ordering::Acquire) { return Err("QA_ALIGNMENT_CANCELLED".into()); }
    if Instant::now() >= deadline { return Err("QA_ALIGNMENT_DEADLINE".into()); }
    Ok(())
}
fn frame_error(error: String) -> String {
    if error.starts_with("remote_frame_") && error.len() <= 80
        && error.bytes().all(|c| c.is_ascii_lowercase() || c == b'_') {
        format!("QA_ALIGNMENT_{error}")
    } else { "QA_ALIGNMENT_NATIVE_FRAME".into() }
}
fn qa_profile(app: &AppHandle) -> Checked<()> {
    let id = &app.config().identifier;
    let suffix = id.strip_prefix("local.atsumi.selectedliveqa.")
        .or_else(|| id.strip_prefix("local.atsumi.livealignmentqa.")).ok_or("QA_ALIGNMENT_FRESH_APP")?;
    if suffix.len() != 32 || !suffix.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        || app.path().app_data_dir().ok().and_then(|p| p.file_name().map(|n| n.to_owned())).as_deref() != Some(std::ffi::OsStr::new(id))
        || app.webviews().keys().any(|label| label.starts_with("youtube-original-")) {
        return Err("QA_ALIGNMENT_FRESH_APP".into());
    }
    // Caller owns a freshly created QA identifier. Never fall back to the main
    // application's browser profile or an existing official-player token.
    Ok(())
}
fn clock(app: &AppHandle, token: &str, video: &str) -> Checked<VodClock> {
    app.state::<YouTubeOriginalHost>().vod_clock(token).map_err(|_| "QA_ALIGNMENT_CLOCK")?
        .filter(|c| c.age_ms <= 1500 && c.fields.valid && c.fields.video_id == video && c.fields.ad_state == "content")
        .ok_or("QA_ALIGNMENT_CONTENT_CLOCK".into())
}
fn wait_clock(app: &AppHandle, token: &str, video: &str, deadline: Instant, cancel: &AtomicBool,
    predicate: impl Fn(&VodClock) -> bool) -> Checked<VodClock> {
    loop {
        check(deadline, cancel)?;
        if let Ok(c) = clock(app, token, video) { if predicate(&c) { return Ok(c); } }
        thread::sleep(Duration::from_millis(50));
    }
}
fn control(app: &AppHandle, token: &str, video: &str, action: &str, seconds: Option<f64>) -> Checked<VodClock> {
    let c = clock(app, token, video)?;
    app.state::<YouTubeOriginalHost>().vod_control(app, token, VodControlRequest {
        request_id: Uuid::new_v4().to_string(), document_epoch: c.document_epoch,
        source_id: c.fields.source_id.clone(), source_generation: c.fields.source_generation,
        action: action.into(), seconds, rate: None,
    }).map_err(|_| "QA_ALIGNMENT_CONTROL")?;
    Ok(c)
}
fn same_owner(binding: &AlignmentBinding, c: &VodClock) -> bool {
    c.fields.video_id == binding.video_id && c.document_epoch == binding.document_epoch
        && c.fields.source_id == binding.remote_source_id.to_string()
        && c.fields.source_generation == binding.source_generation
}
struct OwnedVod { app: AppHandle, token: String, view: Webview, closed: bool }
impl OwnedVod {
    fn open(app: &AppHandle, video: &str) -> Checked<Self> {
        qa_profile(app)?;
        let opened = app.state::<YouTubeOriginalHost>().open_vod(app, video.into(), Uuid::new_v4().to_string())
            .map_err(|_| "QA_ALIGNMENT_OPEN")?;
        let views: Vec<_> = app.webviews().into_values().filter(|v| v.label().starts_with("youtube-original-vod-")).collect();
        if views.len() != 1 {
            let _ = app.state::<YouTubeOriginalHost>().close(app, &opened.token);
            return Err("QA_ALIGNMENT_SOLE_DOCUMENT".into());
        }
        let owned = Self { app: app.clone(), token: opened.token, view: views[0].clone(), closed: false };
        app.state::<YouTubeOriginalHost>().viewport(app, &owned.token, NativeViewport {
            sequence: 1, player: Some(Rect { x: 0., y: 0., width: 960., height: 540. }), chat: None,
            visible: true, muted: true, occlusions: Vec::new(),
        }).map_err(|_| "QA_ALIGNMENT_VIEWPORT")?;
        Ok(owned)
    }
    fn quiet(&self) -> Checked<()> {
        // Playback admission requires a visible unmuted viewport; silence only
        // this owned QA document before resume, without changing account prefs.
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.view.eval_with_callback(r#"(()=>{const v=document.querySelector('#movie_player video');if(!v)return false;const q=()=>{if(v.volume!==0)v.volume=0;if(!v.muted)v.muted=true};v.addEventListener('volumechange',q);q();return v.muted&&v.volume===0})()"#,
            move |s| { let _ = tx.try_send(s); }).map_err(|_| "QA_ALIGNMENT_MUTE")?;
        if rx.recv_timeout(Duration::from_secs(3)).ok().as_deref() != Some("true") { return Err("QA_ALIGNMENT_MUTE".into()); }
        self.app.state::<YouTubeOriginalHost>().viewport(&self.app, &self.token, NativeViewport {
            sequence: 2, player: Some(Rect { x: 0., y: 0., width: 960., height: 540. }), chat: None,
            visible: true, muted: false, occlusions: Vec::new(),
        }).map_err(|_| "QA_ALIGNMENT_VIEWPORT")?;
        Ok(())
    }
    fn close(mut self) -> Checked<()> {
        self.app.state::<YouTubeOriginalHost>().close(&self.app, &self.token).map_err(|_| "QA_ALIGNMENT_CLOSE")?;
        self.closed = true;
        let end = Instant::now() + Duration::from_secs(3);
        while self.app.get_webview(self.view.label()).is_some() {
            if Instant::now() >= end { return Err("QA_ALIGNMENT_CLOSE_PENDING".into()); }
            thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    }
}
impl Drop for OwnedVod {
    fn drop(&mut self) { if !self.closed { let _ = self.app.state::<YouTubeOriginalHost>().close(&self.app, &self.token); } }
}
fn sample_at(owned: &OwnedVod, binding: &AlignmentBinding, seconds: f64, deadline: Instant,
    cancel: &AtomicBool, calls: &mut usize) -> Checked<RemoteWindow> {
    check(deadline, cancel)?;
    if *calls >= MAX_CAPTURE_CALLS { return Err("QA_ALIGNMENT_CAPTURE_BOUND".into()); }
    *calls += 1;
    let end = (Instant::now() + Duration::from_secs(12)).min(deadline);
    let before = control(&owned.app, &owned.token, &binding.video_id, "pause", None)?;
    if !same_owner(binding, &before) { return Err("QA_ALIGNMENT_SOURCE_CHANGED".into()); }
    wait_clock(&owned.app, &owned.token, &binding.video_id, end, cancel, |c| same_owner(binding,c) && c.fields.paused)?;
    let paused = control(&owned.app, &owned.token, &binding.video_id, "seek", Some(seconds))?;
    wait_clock(&owned.app, &owned.token, &binding.video_id, end, cancel, |c| same_owner(binding,c)
        && c.fields.sequence > paused.fields.sequence && !c.fields.seeking && !c.fields.buffering
        && c.fields.current_time.is_some_and(|t| (t-seconds).abs() <= 0.35))?;
    control(&owned.app, &owned.token, &binding.video_id, "resume", None)?;
    let playing = wait_clock(&owned.app, &owned.token, &binding.video_id, end, cancel, |c| same_owner(binding,c)
        && !c.fields.paused && !c.fields.seeking && !c.fields.buffering && !c.fields.ended && (c.fields.playback_rate-1.).abs() < 1e-9)?;
    let native: NativeFrameWindow = owned.app.state::<YouTubeOriginalHost>()
        .vod_sample_frames(&owned.app, &owned.token, &binding.video_id, cancel).map_err(frame_error)?;
    let after = clock(&owned.app, &owned.token, &binding.video_id)?;
    check(deadline, cancel)?;
    if !same_owner(binding, &after) || native.video_id != binding.video_id || native.source_id != binding.remote_source_id
        || native.document_epoch != binding.document_epoch || native.source_generation != binding.source_generation
        || native.seek_generation != playing.fields.seek_generation || native.seek_generation != after.fields.seek_generation
        || after.fields.paused || after.fields.seeking || after.fields.buffering || after.fields.ended {
        return Err("QA_ALIGNMENT_CAPTURE_BOUNDARY".into());
    }
    // false means checked absent throughout by the native sampler, not omitted
    // renderer flags. Only NativeFrameWindow from that sampler reaches here.
    Ok(RemoteWindow { capture_id:native.capture_id, video_id:native.video_id, document_epoch:native.document_epoch,
        source_id:native.source_id, source_generation:native.source_generation, seek_generation:native.seek_generation,
        ad_observed:false, seeking_observed:false, buffering_observed:false,
        frames:native.frames.into_iter().map(|f|RemoteFrame { media_pts90k:f.media_pts90k,
            uncertainty_ticks:f.uncertainty_ticks, feature:f.feature }).collect() })
}
fn slices(window: &RemoteWindow) -> Vec<RemoteWindow> {
    // Keep real contiguous observations, not interpolation or synthetic frame
    // clocks. A shorter overlap avoids blind spots between 2-second captures.
    let count = window.frames.len();
    if count < 8 { return Vec::new(); }
    let width = count.min(12);
    let mut starts: Vec<_> = (0..=count-width).step_by(4).collect();
    if starts.last() != Some(&(count-width)) { starts.push(count-width); }
    starts.into_iter().map(|start| { let mut part=window.clone(); part.frames=window.frames[start..start+width].to_vec(); part })
        .filter(|part| validate_clock(part.frames.iter().map(|f|f.media_pts90k)).is_ok()
            && dynamic(part.frames.iter().map(|f|&f.feature))).collect()
}
fn candidate(reference: &VerifiedLiveReferences, binding: &AlignmentBinding, observed: &RemoteWindow,
    wanted: Option<usize>) -> Result<(usize, WindowMatch, RemoteWindow)> {
    validate_source(reference.references(), reference.timing(), binding)?;
    if observed.video_id != binding.video_id || observed.source_id != binding.remote_source_id
        || observed.document_epoch != binding.document_epoch || observed.source_generation != binding.source_generation
        || observed.ad_observed || observed.seeking_observed || observed.buffering_observed { return Err(AlignmentError::Boundary); }
    let mut choices = Vec::new();
    for part in slices(observed) {
        let mut matches = Vec::new();
        for (index, source) in reference.references().windows.iter().enumerate() {
            if let Ok(Some(fit)) = fit_window(source, &part) { matches.push((index, fit)); }
        }
        matches.sort_by_key(|(_, fit)| fit.mean_hamming_milli);
        let Some((index, fit)) = matches.first() else { continue; };
        if wanted.is_some_and(|wanted| wanted != *index)
            || matches.get(1).is_some_and(|(_,second)| second.mean_hamming_milli < fit.mean_hamming_milli + UNIQUE_WINDOW_MARGIN_MILLI) { continue; }
        choices.push((*index, fit.clone(), part));
    }
    choices.sort_by_key(|(_, fit, _)| fit.mean_hamming_milli);
    choices.into_iter().next().ok_or(AlignmentError::NoMatch)
}
fn targets(reference: &VerifiedLiveReferences, duration: f64) -> Checked<Vec<f64>> {
    let source_duration = reference.timing().transport_end_90k.ok_or("QA_ALIGNMENT_SOURCE_DURATION")? as f64 / HZ as f64;
    let difference = duration-source_duration;
    let mut out = Vec::new();
    // These are search hints only, never proof of first-frame placement.
    for adjustment in [0., difference/2., difference] {
        for window in &reference.references().windows {
            let start = window.frames.first().ok_or("QA_ALIGNMENT_SOURCE_EMPTY")?.pts90k as f64/HZ as f64;
            for delta in [0., -0.7, 0.7] {
                let value = (start + adjustment + delta).clamp(0., (duration-3.).max(0.));
                if out.iter().all(|old:&f64| (*old-value).abs() >= 0.15) { out.push(value); }
                if out.len() >= MAX_SEARCH_CALLS { return Ok(out); }
            }
        }
    }
    Ok(out)
}
/// Parent creates the fresh QA app and opens its exact native vault in place.
/// This function never starts a relay, refreshes credentials or writes a receipt.
pub(crate) fn verify_selected_live_alignment(app: &AppHandle, service: &Arc<CloudService>, selection: &str,
    cancel: &AtomicBool) -> Checked<Value> {
    let deadline = Instant::now() + Duration::from_secs(MAX_SECONDS);
    check(deadline,cancel)?;
    let exact = selected(service,selection)?;
    let history = service.open_live_history(selection).map_err(|_| "QA_ALIGNMENT_HISTORY")?;
    let reference = history.references().map_err(|_| "QA_ALIGNMENT_SOURCE_REFERENCE")?
        .ok_or("QA_ALIGNMENT_SOURCE_REFERENCE_MISSING")?;
    if reference.references().run_id.to_string() != exact.identity.run_id { return Err("QA_ALIGNMENT_RUN_IDENTITY".into()); }
    let owned = OwnedVod::open(app,&exact.identity.broadcast_id)?;
    let initial = wait_clock(app,&owned.token,&exact.identity.broadcast_id,(Instant::now()+Duration::from_secs(45)).min(deadline),cancel,
        |c| c.fields.duration.is_some_and(|d| d.is_finite() && (d-exact.duration).abs() <= 1.) && !c.fields.seeking)?;
    owned.quiet()?;
    let binding = AlignmentBinding { run_id:reference.references().run_id, source_id:reference.references().source_id,
        stream_id:reference.references().stream_id.ok_or("QA_ALIGNMENT_STREAM")?, video_id:exact.identity.broadcast_id.clone(),
        remote_source_id:Uuid::parse_str(&initial.fields.source_id).map_err(|_|"QA_ALIGNMENT_REMOTE_SOURCE")?,
        document_epoch:initial.document_epoch, source_generation:initial.fields.source_generation };
    validate_source(reference.references(),reference.timing(),&binding).map_err(|_|"QA_ALIGNMENT_SOURCE_INCOMPLETE")?;
    let mut calls = 0usize;
    let mut located = None;
    for target in targets(reference,exact.duration)? {
        let window = sample_at(&owned,&binding,target,deadline,cancel,&mut calls)?;
        if let Ok((_, fit, _)) = candidate(reference,&binding,&window,None) { located=Some(fit); break; }
    }
    let located = located.ok_or("QA_ALIGNMENT_FIRST_WINDOW_NOT_FOUND")?;
    let guess = (located.offset.lower as f64+located.offset.upper as f64)/2.;
    let mut final_windows = Vec::new();
    for (index, source) in reference.references().windows.iter().enumerate() {
        let predicted = (source.frames[0].pts90k as f64-guess)/HZ as f64;
        let mut accepted = None;
        for adjustment in [0., -0.35, 0.35] {
            let target=(predicted+adjustment).clamp(0.,exact.duration-3.);
            let window=sample_at(&owned,&binding,target,deadline,cancel,&mut calls)?;
            if let Ok((_, _, part))=candidate(reference,&binding,&window,Some(index)) { accepted=Some(part);break; }
        }
        match accepted { Some(window)=>final_windows.push(window), None if final_windows.len()>=2=>break,
            None=>return Err("QA_ALIGNMENT_CONFIRMATION_NOT_FOUND".into()) }
    }
    // Search observations are deliberately absent. Acceptance uses newly
    // requested windows in source order with distinct capture/seek identities.
    let measured=align(reference.references(),reference.timing(),&binding,&final_windows)
        .map_err(|reason|format!("QA_ALIGNMENT_FINAL_{reason:?}"))?;
    if selected(service,selection)? != exact { return Err("QA_ALIGNMENT_SELECTION_CHANGED".into()); }
    check(deadline,cancel)?;
    let result=json!({"measuredAlignment":true,"selectionId":selection,"windowCount":measured.windows.len(),
        "captureCalls":calls,"offsetLowerTicks90k":measured.offset.lower,"offsetUpperTicks90k":measured.offset.upper,
        "twoWindowEstimate":measured.two_window_estimate,"historyIncomplete":measured.history_incomplete,
        "matches":measured.windows.iter().map(|w|json!({"sourceOrdinal":w.source_ordinal,"frames":w.matched_frames,
            "meanHammingMilli":w.mean_hamming_milli,"sourceFirst90k":w.source_first90k,"sourceLast90k":w.source_last90k,
            "remoteFirst90k":w.remote_first90k,"remoteLast90k":w.remote_last90k})).collect::<Vec<_>>(),
        "sourceLeaseRetained":true,"freshOwnedDocument":true,"sourceVideoFilesRead":false,
        "pixelsSaved":false,"remoteMutations":0,"receiptPromoted":false,"chatSynchronizationVerified":false});
    owned.close()?;
    Ok(result)
}

#[test]
fn exploratory_slices_keep_actual_frames_and_exact_capture_identity() {
    let window = RemoteWindow { capture_id:Uuid::new_v4(),video_id:"abcdefghijk".into(),document_epoch:3,
        source_id:Uuid::new_v4(),source_generation:4,seek_generation:5,ad_observed:false,seeking_observed:false,
        buffering_observed:false,frames:(0..25u64).map(|i|RemoteFrame {media_pts90k:90000+i*7500,
            uncertainty_ticks:4500,feature:FrameFeature {gradient:[(i*7919)as u32,(i*8123)as u32,(i*4421)as u32,(i*1237)as u32],
                mean:100,spread:32}}).collect() };
    let parts=slices(&window);
    assert!(!parts.is_empty()); assert!(parts.len()<=8);
    for part in parts {
        assert_eq!(part.capture_id,window.capture_id);assert_eq!(part.source_id,window.source_id);
        assert_eq!(part.document_epoch,window.document_epoch);assert_eq!(part.seek_generation,window.seek_generation);
        let start=window.frames.iter().position(|f|f.media_pts90k==part.frames[0].media_pts90k).unwrap();
        for (old,new) in window.frames[start..].iter().zip(&part.frames) {
            assert_eq!(old.media_pts90k,new.media_pts90k);assert_eq!(old.uncertainty_ticks,new.uncertainty_ticks);
            assert_eq!(old.feature,new.feature);
        }
    }
    let mut static_window=window;
    for frame in &mut static_window.frames {frame.feature.gradient=[0;4];frame.feature.spread=0;}
    assert!(slices(&static_window).is_empty());
}
