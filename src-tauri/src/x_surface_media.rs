//! Passive observations from one interactive official X pane. Requests always
//! continue normally; this module neither fetches nor modifies source media.
use super::{allowed, video};
use std::{
    collections::VecDeque,
    sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{Manager, Url, Webview};

const MAX_OBSERVED: usize = 400;
const TTL: Duration = Duration::from_secs(600);
const ERROR: &str = "X 원본 미디어 관찰을 준비하지 못했습니다.";

struct Entry { url: String, touched: Instant }
pub(super) struct Observations {
    revision: u64,
    revoked: AtomicBool,
    entries: Mutex<VecDeque<Entry>>,
}
impl Observations {
    pub(super) fn new(revision: u64) -> Self {
        Self { revision, revoked: AtomicBool::new(false), entries: Mutex::new(VecDeque::new()) }
    }
    fn current(&self, revision: u64) -> bool {
        self.revision == revision && !self.revoked.load(Ordering::Acquire)
    }
    pub(super) fn clear(&self) {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }
    pub(super) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
        self.clear();
    }
    fn record(&self, revision: u64, source: &str, method: &str, resource: &str, now: Instant) {
        if !self.current(revision) || !accepted(source, method, resource) { return; }
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        if !self.current(revision) { return; }
        entries.retain(|entry| now.saturating_duration_since(entry.touched) < TTL);
        if let Some(entry) = entries.iter_mut().find(|entry| entry.url == resource) {
            entry.touched = now;
        } else {
            entries.push_back(Entry { url: resource.into(), touched: now });
        }
        while entries.len() > MAX_OBSERVED { entries.pop_front(); }
    }
    pub(super) fn snapshot(&self, revision: u64) -> Vec<String> {
        self.snapshot_at(revision, Instant::now())
    }
    fn snapshot_at(&self, revision: u64, now: Instant) -> Vec<String> {
        if !self.current(revision) { return Vec::new(); }
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        if !self.current(revision) { return Vec::new(); }
        entries.retain(|entry| now.saturating_duration_since(entry.touched) < TTL);
        entries.iter().map(|entry| entry.url.clone()).collect()
    }
}
fn accepted(source: &str, method: &str, resource: &str) -> bool {
    method == "GET"
        && Url::parse(source).is_ok_and(|url| allowed(&url))
        && resource.len() <= 2048
        && Url::parse(resource).is_ok_and(|url| video::scope(&url).is_some()
            && matches!(url.path().rsplit('.').next(), Some("m3u8" | "mp4")))
}

// Install after native child creation and before its first navigation. The
// caller owns the pane/profile; no remote document can register an observer.
#[cfg(windows)]
pub(super) fn install(view: &Webview, observations: Arc<Observations>, revision: Arc<AtomicU64>) -> Result<(), String> {
    use webview2_com::{Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL, WebResourceRequestedEventHandler};
    use windows::{core::{HSTRING, PWSTR}, Win32::System::Com::CoTaskMemFree};
    unsafe fn read_string(read: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> Option<String> {
        let mut pointer = PWSTR::null();
        let result = read(&mut pointer);
        let value = if pointer.is_null() { None } else { pointer.to_string().ok() };
        if !pointer.is_null() { CoTaskMemFree(Some(pointer.0.cast())); }
        result.ok().and(value)
    }
    let main = view.app_handle().get_webview("main").ok_or(ERROR)?;
    if !view.label().starts_with("x-surface-") || view.label() == main.label()
        || view.window().label() != main.window().label()
        || !observations.current(revision.load(Ordering::Acquire)) {
        return Err(ERROR.into());
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let installed = (|| -> windows::core::Result<()> {
            let core = platform.controller().CoreWebView2()?;
            let mut token = 0;
            core.add_WebResourceRequested(&WebResourceRequestedEventHandler::create(Box::new(move |sender, args| {
                // Observation failure must never fail, defer or replace the
                // source request. Read only the public URL, method and origin.
                if observations.current(revision.load(Ordering::Acquire)) {
                    if let (Some(sender), Some(args)) = (sender, args) {
                        if let Ok(request) = args.Request() {
                            if let (Some(source), Some(method), Some(resource)) = (
                                read_string(|p| sender.Source(p)),
                                read_string(|p| request.Method(p)),
                                read_string(|p| request.Uri(p)),
                            ) {
                                observations.record(revision.load(Ordering::Acquire), &source, &method, &resource, Instant::now());
                            }
                        }
                    }
                }
                Ok(())
            })), &mut token)?;
            core.AddWebResourceRequestedFilter(&HSTRING::from("https://video.twimg.com/*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
            Ok(())
        })();
        let _ = tx.send(installed.is_ok());
    }).map_err(|_| ERROR)?;
    if rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false) { Ok(()) } else { Err(ERROR.into()) }
}
#[cfg(not(windows))]
pub(super) fn install(_: &Webview, _: Arc<Observations>, _: Arc<AtomicU64>) -> Result<(), String> {
    // Other platforms retain the read-only DOM resource observation fallback.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const HOME: &str = "https://x.com/home";
    const MP4: &str = "https://video.twimg.com/ext_tw_video/123456/pu/vid/avc1/640x360/fixture.mp4?tag=12&v=1&variant_version=1";
    const HLS: &str = "https://video.twimg.com/amplify_video/123456/pl/fixture.m3u8?tag=14";
    #[test]
    fn only_actual_public_get_manifest_or_mp4_urls_are_candidates() {
        assert!(accepted(HOME, "GET", MP4));
        assert!(accepted("https://www.x.com/creator/status/123456", "GET", HLS));
        for method in ["POST", "HEAD", "get", ""] { assert!(!accepted(HOME, method, MP4)); }
        for source in ["about:blank", "https://evil.test/", "https://x.com.evil.test/", "https://user@x.com/home", "https://x.com:444/home", "http://x.com/home"] {
            assert!(!accepted(source, "GET", MP4));
        }
        for resource in ["https://video.twimg.com/ext_tw_video/123456/pu/seg/fixture.ts", "https://video.twimg.com/ext_tw_video/123456/pu/seg/fixture.m4s",
            "https://video.twimg.com/ext_tw_video/123456/pu/seg/fixture.aac", "https://video.twimg.com.evil.test/ext_tw_video/123456/fixture.mp4",
            "https://user@video.twimg.com/ext_tw_video/123456/fixture.mp4", "https://video.twimg.com:444/ext_tw_video/123456/fixture.mp4",
            "https://video.twimg.com/ext_tw_video/123456/fixture.mp4?token=fixture", "https://video.twimg.com/ext_tw_video/123456/fixture.mp4?tag=bad%20value",
            "https://video.twimg.com/ext_tw_video/123456/a%2fb.mp4", "https://video.twimg.com/ext_tw_video/012345/fixture.mp4", "blob:https://x.com/fixture"] {
            assert!(!accepted(HOME, "GET", resource), "invalid resource fixture");
        }
    }
    #[test]
    fn observations_remain_pane_local_deduplicated_and_expire() {
        let left = Observations::new(3); let right = Observations::new(3); let now = Instant::now();
        left.record(3, HOME, "GET", MP4, now);
        left.record(3, HOME, "GET", MP4, now + Duration::from_secs(10));
        left.record(3, HOME, "GET", HLS, now);
        assert_eq!(left.snapshot_at(3, now).len(), 2);
        assert!(right.snapshot_at(3, now).is_empty());
        assert_eq!(left.snapshot_at(3, now + TTL), vec![MP4.to_string()]);
        assert!(left.snapshot_at(3, now + TTL + Duration::from_secs(10)).is_empty());
    }
    #[test]
    fn stale_or_revoked_observers_cannot_read_or_accept_more_resources() {
        let observations = Observations::new(5); let now = Instant::now();
        observations.record(4, HOME, "GET", MP4, now); assert!(observations.snapshot_at(5, now).is_empty());
        observations.record(5, HOME, "GET", MP4, now); assert!(observations.snapshot_at(6, now).is_empty());
        observations.clear(); assert!(observations.snapshot_at(5, now).is_empty());
        observations.record(5, HOME, "GET", MP4, now); observations.revoke();
        observations.record(5, HOME, "GET", HLS, now); assert!(observations.snapshot_at(5, now).is_empty());
    }
    #[test]
    fn observation_memory_is_bounded_without_synthesizing_urls() {
        let observations = Observations::new(1); let now = Instant::now();
        for i in 0..405 { observations.record(1, HOME, "GET", &format!("https://video.twimg.com/ext_tw_video/123456/pl/fixture{i}.m3u8"), now); }
        let snapshot = observations.snapshot_at(1, now);
        assert_eq!(snapshot.len(), MAX_OBSERVED);
        assert!(snapshot.first().unwrap().ends_with("fixture5.m3u8"));
        assert!(snapshot.last().unwrap().ends_with("fixture404.m3u8"));
    }
}
