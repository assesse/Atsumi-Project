//! Geometry ownership for an official X document's native fullscreen element.
//! WebView2/Tauri own enter/exit and video controls. This module never starts,
//! seeks, focuses or recreates media, and accepts no remote bridge command.
use super::{allowed, pane_state, paint, registered_visible, AppHandle, Pane, Rect, Surfaces, Webview, XSession, ERROR};
use std::{sync::{atomic::Ordering, Arc}, time::Duration};
use tauri::Manager;

const EXIT: &str = "if(document.fullscreenElement){void document.exitFullscreen().catch(()=>{});}";

pub(super) struct Presentation {
    label: String,
    bounds: Rect,
}
impl Presentation {
    pub(super) fn rect(&self, pane: &Pane) -> Option<Rect> {
        (self.label == pane.label).then(|| self.bounds.clone())
    }
}

fn selected(data: &Surfaces, pane: &Pane) -> bool {
    if pane.feed == "preview" {
        data.preview.as_ref().is_some_and(|current| current.label == pane.label)
    } else {
        data.active == pane.feed
            && data.feeds.get(&data.active).is_some_and(|current| current.label == pane.label)
    }
}

/// Used in the existing interaction/media gate as well as painting. A hidden
/// second document cannot claim autoplay or receive actions over fullscreen.
pub(super) fn allows(data: &Surfaces, pane: &Pane) -> bool {
    let mut owners = data.feeds.values().chain(data.preview.iter())
        .filter(|other| other.revision == pane.revision && selected(data, other)
            && other.fullscreen.load(Ordering::Acquire));
    match owners.next() {
        None => true,
        Some(owner) => overlay_free(data) && owner.label == pane.label
            && owners.all(|other| other.label == pane.label),
    }
}

fn overlay_free(data: &Surfaces) -> bool {
    data.viewport.as_ref().is_some_and(|viewport| viewport.visible
        && !viewport.occlusions.iter().any(|hole| hole.width > 0.0 && hole.height > 0.0))
}

fn admission(current: bool, visible: bool, ready: bool, registered: bool, overlay_free: bool, origin: bool) -> bool {
    current && visible && ready && registered && overlay_free && origin
}

fn client_rect(width: u32, height: u32, scale: f64) -> Option<Rect> {
    if !scale.is_finite() || scale <= 0.0 { return None; }
    let (width, height) = (f64::from(width) / scale, f64::from(height) / scale);
    if !width.is_finite() || !height.is_finite() || width < 100.0 || height < 80.0
        || width > 32768.0 || height > 32768.0 { return None; }
    Some(Rect { x: 0.0, y: 0.0, width, height })
}

fn exit(view: &Webview) {
    // Privacy/navigation may revoke a previously admitted fullscreen element.
    // Do not synthesize Escape or request new fullscreen/media playback.
    let _ = view.eval(EXIT);
}

/// Called before hiding/disposing any native pane. The exact Pane owns its flag,
/// so a delayed exit from a retired document cannot revoke a newer document.
pub(super) fn conceal(pane: &Pane, view: &Webview) {
    if pane.fullscreen.swap(false, Ordering::AcqRel) { exit(view); }
}

fn changed(session: &XSession, app: &AppHandle, pane: &Arc<Pane>, entered: bool) {
    if !entered {
        let was_fullscreen = pane.fullscreen.swap(false, Ordering::AcqRel);
        // A privacy/navigation conceal may have revoked the flag before the
        // native exit notification. Still restore the current pane's slots.
        if was_fullscreen || super::is_current(session, pane) { paint(session, app); }
        return;
    }
    let view = app.get_webview(&pane.label);
    let origin = view.as_ref().is_some_and(|view| view.window().label() == "main"
        && view.url().is_ok_and(|url| allowed(&url) && !url.path().starts_with("/__atsumi_surface__/"))
        && view.window().is_visible().unwrap_or(false) && !view.window().is_minimized().unwrap_or(true));
    let admitted = {
        let data = session.surfaces.lock().unwrap_or_else(|error| error.into_inner());
        admission(pane.revision == session.revision.load(Ordering::Acquire),
            pane.visible.load(Ordering::Acquire), pane_state(pane) == "ready",
            registered_visible(&data, pane), overlay_free(&data), origin)
    };
    if admitted {
        pane.fullscreen.store(true, Ordering::Release);
    } else if let Some(view) = view {
        pane.fullscreen.store(false, Ordering::Release);
        exit(&view);
    }
    paint(session, app);
}

/// Re-evaluated on each normal viewport update, including DPI/resize and
/// privacy/overlay changes. Original slot rectangles are never overwritten.
pub(super) fn presentation(session: &XSession, app: &AppHandle) -> Option<Presentation> {
    let panes = {
        let data = session.surfaces.lock().unwrap_or_else(|error| error.into_inner());
        data.feeds.values().chain(data.preview.iter())
            .filter(|pane| pane.fullscreen.load(Ordering::Acquire)).cloned().collect::<Vec<_>>()
    };
    let mut result = None;
    for pane in panes {
        let view = app.get_webview(&pane.label);
        let valid = {
            let data = session.surfaces.lock().unwrap_or_else(|error| error.into_inner());
            pane.revision == session.revision.load(Ordering::Acquire)
                && pane_state(&pane) == "ready" && registered_visible(&data, &pane) && overlay_free(&data)
        };
        let bounds = view.as_ref().filter(|view| valid && view.window().label() == "main"
            && view.url().is_ok_and(|url| allowed(&url) && !url.path().starts_with("/__atsumi_surface__/"))
            && view.window().is_visible().unwrap_or(false) && !view.window().is_minimized().unwrap_or(true))
            .and_then(|view| {
                let size = view.window().inner_size().ok()?;
                client_rect(size.width, size.height, view.window().scale_factor().ok()?)
            });
        if let Some(bounds) = bounds.filter(|_| result.is_none()) {
            result = Some(Presentation { label: pane.label.clone(), bounds });
        } else {
            pane.fullscreen.store(false, Ordering::Release);
            if let Some(view) = view { exit(&view); }
        }
    }
    result
}

#[cfg(windows)]
pub(super) fn install(view: &Webview, session: XSession, pane: Arc<Pane>) -> Result<(), String> {
    use webview2_com::ContainsFullScreenElementChangedEventHandler;
    if view.label() != pane.label || view.window().label() != "main" { return Err(ERROR.into()); }
    let app = view.app_handle().clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let installed = (|| -> windows::core::Result<()> {
            let core = platform.controller().CoreWebView2()?;
            let mut token = 0;
            core.add_ContainsFullScreenElementChanged(
                &ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _| {
                    let Some(sender) = sender else { return Ok(()); };
                    let mut entered = windows::core::BOOL::default();
                    sender.ContainsFullScreenElement(&mut entered)?;
                    changed(&session, &app, &pane, entered.as_bool());
                    Ok(())
                })), &mut token)?;
            Ok(())
        })();
        let _ = tx.send(installed.is_ok());
    }).map_err(|_| ERROR)?;
    if rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false) { Ok(()) } else { Err(ERROR.into()) }
}

#[cfg(not(windows))]
pub(super) fn install(_: &Webview, _: XSession, _: Arc<Pane>) -> Result<(), String> { Err(ERROR.into()) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_native_geometry::{Hole, NativeViewport};
    use std::{sync::{atomic::AtomicBool, Mutex}, time::Instant};

    fn pane(label: &str, feed: &str, revision: u64) -> Arc<Pane> {
        Arc::new(Pane { label: label.into(), nonce: "fixture".into(), feed: feed.into(),
            owner: "alice".into(), profile: "profile".into(), revision,
            state: Mutex::new("ready".into()), loading_since: Mutex::new(Instant::now()),
            document_load: Mutex::new(Default::default()), visible: AtomicBool::new(true),
            root_recovery: Mutex::new(Default::default()),
            fullscreen: AtomicBool::new(false),
            media: Arc::new(super::super::super::surface_media::Observations::new(revision)) })
    }
    fn surfaces() -> (Surfaces, Arc<Pane>, Arc<Pane>) {
        let feed = pane("feed", "foryou", 7);
        let preview = pane("preview", "preview", 7);
        let mut data = Surfaces::default();
        data.active = "foryou".into();
        data.feeds.insert(data.active.clone(), feed.clone());
        data.preview = Some(preview.clone());
        data.viewport = Some(NativeViewport { sequence: 1, visible: true, muted: false,
            player: Some(Rect { x: 220.0, y: 90.0, width: 600.0, height: 900.0 }),
            chat: Some(Rect { x: 840.0, y: 90.0, width: 600.0, height: 900.0 }), occlusions: vec![] });
        (data, feed, preview)
    }
    #[test]
    fn native_admission_requires_every_current_visible_ready_origin_gate() {
        assert!(admission(true, true, true, true, true, true));
        for gate in 0..6 {
            let mut values = [true; 6]; values[gate] = false;
            assert!(!admission(values[0], values[1], values[2], values[3], values[4], values[5]));
        }
    }
    #[test]
    fn client_bounds_use_logical_dpi_and_reject_invalid_geometry() {
        assert_eq!(client_rect(3440, 1440, 1.0), Some(Rect { x: 0.0, y: 0.0, width: 3440.0, height: 1440.0 }));
        assert_eq!(client_rect(3840, 2160, 2.0), Some(Rect { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0 }));
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] { assert!(client_rect(1920, 1080, scale).is_none()); }
        assert!(client_rect(0, 0, 1.0).is_none());
        assert!(client_rect(u32::MAX, 1080, 1.0).is_none());
    }
    #[test]
    fn fullscreen_owner_is_exclusive_without_mutating_normal_slots() {
        let (data, feed, preview) = surfaces();
        let original = data.viewport.as_ref().unwrap().player.clone();
        assert!(registered_visible(&data, &feed)); assert!(registered_visible(&data, &preview));
        feed.fullscreen.store(true, Ordering::Release);
        assert!(allows(&data, &feed)); assert!(!allows(&data, &preview));
        assert!(registered_visible(&data, &feed)); assert!(!registered_visible(&data, &preview));
        let presentation = Presentation { label: feed.label.clone(), bounds: client_rect(3440, 1440, 1.0).unwrap() };
        assert_eq!(presentation.rect(&feed).unwrap().width, 3440.0);
        assert!(presentation.rect(&preview).is_none());
        assert_eq!(data.viewport.as_ref().unwrap().player, original);
        feed.fullscreen.store(false, Ordering::Release);
        assert!(allows(&data, &feed)); assert!(allows(&data, &preview));
        assert_eq!(data.viewport.as_ref().unwrap().player, original);
    }
    #[test]
    fn old_document_exit_cannot_clear_new_fullscreen_and_hidden_feeds_do_not_own() {
        let (mut data, feed, preview) = surfaces();
        let retired = pane("retired", "preview", 7);
        retired.fullscreen.store(true, Ordering::Release);
        preview.fullscreen.store(true, Ordering::Release);
        retired.fullscreen.store(false, Ordering::Release);
        assert!(allows(&data, &preview)); assert!(!allows(&data, &feed));
        preview.fullscreen.store(false, Ordering::Release);
        let hidden = pane("hidden", "likes", 7);
        hidden.fullscreen.store(true, Ordering::Release);
        data.feeds.insert("likes".into(), hidden);
        assert!(allows(&data, &feed)); assert!(allows(&data, &preview));
    }
    #[test]
    fn privacy_or_any_overlay_revokes_fullscreen_even_outside_old_slot() {
        let (mut data, feed, preview) = surfaces();
        feed.fullscreen.store(true, Ordering::Release);
        assert!(overlay_free(&data));
        assert!(allows(&data, &feed)); assert!(!allows(&data, &preview));
        data.viewport.as_mut().unwrap().occlusions.push(Hole { x: 2000.0, y: 0.0, width: 100.0, height: 100.0, radii: [0.0; 4] });
        assert!(!overlay_free(&data));
        assert!(!allows(&data, &feed)); assert!(!allows(&data, &preview));
        assert!(!registered_visible(&data, &feed)); assert!(!registered_visible(&data, &preview));
        data.viewport.as_mut().unwrap().occlusions.clear();
        data.viewport.as_mut().unwrap().visible = false;
        assert!(!overlay_free(&data));
        data.viewport = None; assert!(!overlay_free(&data));
    }
    #[test]
    fn revocation_script_only_exits_and_never_plays_or_requests_fullscreen() {
        assert!(EXIT.contains("document.exitFullscreen()"));
        for forbidden in [".play(", "requestFullscreen", "focus(", "currentTime", "location", "fetch("] {
            assert!(!EXIT.contains(forbidden));
        }
    }
}
