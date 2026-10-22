//! Bounded, read-only projection of the user's officially rendered X feeds.
//! Credentials remain in WebView2; no private API calls or remote IPC grants.
use crate::interface::AppState;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::Read,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State, Url, Webview, WebviewUrl, WebviewWindowBuilder};
#[path = "x_accounts.rs"]
mod accounts;
#[path = "x_account_presentation.rs"]
mod account_presentation;
#[path = "x_surface.rs"]
pub mod surface;
#[path = "x_video.rs"]
pub mod video;
#[path = "x_action.rs"]
pub mod action;
#[path = "x_diagnostics.rs"]
mod diagnostics;
#[path = "x_archive.rs"]
pub mod archive;
#[path = "x_archive_ipc.rs"]
pub mod archive_ipc;
#[path = "x_surface_media.rs"]
mod surface_media;

const ACCOUNT: &str = "x-account";
const ERROR: &str =
    "X 피드를 읽지 못했습니다. 공식 로그인 창에서 상태를 확인한 뒤 다시 불러와 주세요.";
const STALE: &str = "X 탐색 상태가 변경되었거나 만료되었습니다. 새로고침해 주세요.";
const MAX_POSTS: usize = 1200;
const READER_IDLE: Duration = Duration::from_secs(300);

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Post {
    id: String,
    handle: String,
    name: String,
    text: String,
    time: String,
    media: Vec<String>,
    #[serde(default)]
    avatar: Option<String>,
    video: bool,
    #[serde(default)]
    gif: bool,
    #[serde(default)]
    relation: Option<String>,
    truncated: bool,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    repost: Option<String>,
    #[serde(default)]
    quote: Option<Box<Post>>,
    #[serde(default, rename = "quotePreview")]
    quote_preview: Option<QuotePreview>,
    #[serde(default)]
    stats: PostStats,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct QuotePreview {
    handle: String, name: String, text: String, time: String,
    media: Vec<String>, avatar: Option<String>, video: bool,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PostStats {
    replies: String,
    reposts: String,
    likes: String,
    views: String,
    bookmarks: String,
    liked: bool,
    reposted: bool,
    bookmarked: bool,
    can_reply: bool,
    can_like: bool,
    can_repost: bool,
    can_bookmark: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Community { id: String, name: String, path: String }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    posts: Vec<Post>,
    cursor: Option<String>,
    generation: u64,
    status: &'static str,
    limited: bool,
    communities: Vec<Community>,
}
#[derive(Serialize)]
pub struct Snapshot {
    status: &'static str,
    generation: u64,
    open: bool,
    accounts: Vec<accounts::Account>,
    presentations: HashMap<String, account_presentation::Presentation>,
    active: String,
    error: Option<String>,
}
struct Reader {
    label: String,
    feed: String,
    cursor: Option<String>,
    touched: Instant,
    seen: HashSet<String>,
    target: Arc<Mutex<Option<String>>>,
    owner: Option<String>,
    profile: String,
}
struct ObservedPoster {
    owner: String, handle: String, at: Instant, media: Vec<String>,
}
#[derive(Default)]
struct Data {
    owner: Option<String>,
    readers: HashMap<String, Reader>,
    posts: HashMap<String, Post>,
    order: VecDeque<String>,
    images: VecDeque<(String, String)>,
    communities: Vec<Community>,
    observed_posters: HashMap<String, ObservedPoster>,
}
// A detail reader may lose its poster when blocked background playback removes
// the source video. Carry only a recent, actually observed and validated poster.
fn normalize_gif(post:&mut Post) {
    // The list is this post's own media; quoted media is normalized separately.
    // Revalidate before interpreting the path, never trust the incoming flag.
    post.gif=post.video&&post.media.iter().any(|value|media_url(value).is_ok_and(|url|{
        url.path().strip_prefix("/tweet_video_thumb/").is_some_and(|file|{
            !file.contains('/')&&file.rsplit_once('.').is_some_and(|(_,extension)|matches!(extension,"jpg"|"jpeg"|"png"|"webp"))
        })
    }));
}
fn retain_observed_posters(data:&mut Data,posts:&mut [Post]) {
    let Some(owner)=data.owner.clone() else {data.observed_posters.clear();return;};
    data.observed_posters.retain(|_,p|p.owner==owner&&p.at.elapsed()<READER_IDLE);
    for post in posts {
        if !post.video {post.gif=false;data.observed_posters.remove(&post.id);continue;}
        let posters=post.media.iter().filter(|u|u.contains("_video_thumb/")).cloned().collect::<Vec<_>>();
        if !posters.is_empty() {
            data.observed_posters.insert(post.id.clone(),ObservedPoster{owner:owner.clone(),handle:post.handle.clone(),at:Instant::now(),media:posters});
        } else if post.media.is_empty() {
            if let Some(previous)=data.observed_posters.get(&post.id).filter(|p|p.handle.eq_ignore_ascii_case(&post.handle)) {
                post.media=previous.media.clone();
            }
        }
        normalize_gif(post);
    }
    while data.observed_posters.len()>200 {
        if let Some(oldest)=data.observed_posters.iter().min_by_key(|(_,p)|p.at).map(|(id,_)|id.clone()) {data.observed_posters.remove(&oldest);} else {break;}
    }
}
#[derive(Clone)]
pub struct XSession {
    revision: Arc<AtomicU64>,
    generation: Arc<AtomicU64>,
    data: Arc<Mutex<Data>>,
    pending: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    image_slots: Arc<tokio::sync::Semaphore>,
    account_lane: Arc<tokio::sync::Mutex<()>>,
    feed_lane: Arc<tokio::sync::Mutex<()>>,
    registry: Arc<Mutex<Option<accounts::Registry>>>,
    presentations: Arc<Mutex<account_presentation::Store>>,
    login_error: Arc<Mutex<Option<String>>>,
    surfaces: Arc<Mutex<surface::Surfaces>>,
    surface_lane: Arc<tokio::sync::Mutex<()>>,
    videos: Arc<Mutex<video::Videos>>,
    pub media_slots: Arc<tokio::sync::Semaphore>,
    archive: archive::XArchiveState,
}
impl Default for XSession {
    fn default() -> Self {
        Self {
            revision: Arc::new(AtomicU64::new(1)),
            generation: Arc::new(AtomicU64::new(1)),
            data: Arc::default(),
            pending: Arc::default(),
            image_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            account_lane: Arc::default(),
            feed_lane: Arc::default(),
            registry: Arc::default(),
            presentations: Arc::default(),
            login_error: Arc::default(),
            surfaces: Arc::default(),
            surface_lane: Arc::default(),
            videos: Arc::default(),
            media_slots: Arc::new(tokio::sync::Semaphore::new(4)),
            archive: archive::XArchiveState::default(),
        }
    }
}
fn allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(url.host_str(), Some("x.com" | "www.x.com"))
}
fn reader_url(url: &Url, target: Option<&str>) -> bool {
    allowed(url)
        && (url.path() == "/home"
            || url.path() == "/login"
            || url.path().starts_with("/i/flow/")
            || target.is_some_and(|path| {
                let aliases = (path.ends_with("/likes") && url.path() == "/i/history/likes")
                    || (matches!(path, "/i/bookmarks" | "/i/history/bookmarks" | "/i/history")
                        && matches!(url.path(), "/i/bookmarks" | "/i/history/bookmarks" | "/i/history"));
                (url.path() == path || aliases) && url.query().is_none() && url.fragment().is_none()
            }))
}
fn feed_target(feed: &str, owner: &str, path: &str) -> bool {
    if !valid_handle(owner) {
        return false;
    }
    match feed {
        "foryou" | "following" => path == "/home",
        "bookmarks" => matches!(path, "/i/bookmarks" | "/i/history/bookmarks" | "/i/history"),
        "likes" => path == format!("/{owner}/likes") || path == "/i/history/likes",
        "communities" => path == "/i/communities" || path == format!("/{owner}/communities"),
        _ => feed.strip_prefix("community:").is_some_and(|id| (valid_id(id) && path == format!("/i/communities/{id}")) || (valid_home_tab(id) && path=="/home"))
            || feed.strip_prefix("profile:").is_some_and(|handle| valid_handle(handle) && path == format!("/{handle}"))
            || feed.strip_prefix("post:").is_some_and(|id| {
                let parts = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
                valid_id(id) && parts.len() == 3 && valid_handle(parts[0]) && parts[1] == "status" && parts[2] == id
            }),
    }
}
fn valid_feed(feed: &str) -> bool {
    matches!(feed, "foryou" | "following" | "likes" | "bookmarks" | "communities")
        || feed.strip_prefix("community:").is_some_and(|id|valid_id(id)||valid_home_tab(id))
        || feed.strip_prefix("post:").is_some_and(valid_id)
        || feed.strip_prefix("profile:").is_some_and(valid_handle)
}
fn valid_home_tab(id:&str)->bool { id.strip_prefix("tab-").is_some_and(|n|n.len()<=2&&n.parse::<u8>().is_ok_and(|i|(2..80).contains(&i))) }
fn validated_communities(value: &Value) -> Result<Vec<Community>, String> {
    if value.is_null() { return Ok(vec![]); }
    let values = value.as_array().filter(|a|a.len()<=80).ok_or(ERROR)?;
    let mut result = Vec::new();
    for v in values {
        let c: Community = serde_json::from_value(v.clone()).map_err(|_|ERROR)?;
        if c.name.is_empty() || c.name.chars().count()>100 || !((valid_id(&c.id)&&c.path==format!("/i/communities/{}",c.id))||(valid_home_tab(&c.id)&&c.path=="/home")) { return Err(ERROR.into()); }
        if !result.iter().any(|known:&Community|known.id==c.id) { result.push(c); }
    }
    Ok(result)
}
fn trusted(window: &Webview) -> Result<(), String> {
    let url = window.url().map_err(|_| ERROR)?;
    if crate::frontend_origin::trusted_main(
        window.label(),
        &url,
        cfg!(debug_assertions),
        window.app_handle().config().build.dev_url.as_ref(),
    ) {
        Ok(())
    } else {
        Err(ERROR.into())
    }
}
fn valid_handle(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 15
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
fn valid_id(value: &str) -> bool {
    (5..=25).contains(&value.len())
        && !value.starts_with('0')
        && value.bytes().all(|c| c.is_ascii_digit())
}
fn media_url(value: &str) -> Result<Url, String> {
    let mut url = Url::parse(value).map_err(|_| ERROR)?;
    if url.scheme() != "https"
        || url.host_str() != Some("pbs.twimg.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return Err(ERROR.into());
    }
    let path = url.path().strip_prefix("/media/").or_else(|| {
        let parts = url.path().trim_start_matches('/').split('/').collect::<Vec<_>>();
        match parts.as_slice() {
            ["ext_tw_video_thumb", id, "pu", "img", file] if valid_id(id) => Some(*file),
            ["amplify_video_thumb", id, "img", file] if valid_id(id) => Some(*file),
            ["tweet_video_thumb", file] => Some(*file),
            _ => None,
        }
    }).ok_or(ERROR)?;
    let (stem, extension) = path.split_once('.').unwrap_or((path, ""));
    if stem.is_empty()
        || stem.len() > 150
        || !stem
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        || !matches!(extension, "" | "jpg" | "jpeg" | "png" | "webp")
    {
        return Err(ERROR.into());
    }
    if url.query_pairs().any(|(k, v)| {
        !matches!(k.as_ref(), "format" | "name")
            || k == "format" && !matches!(v.as_ref(), "jpg" | "jpeg" | "png" | "webp")
    }) {
        return Err(ERROR.into());
    }
    let format = url
        .query_pairs()
        .find(|(k, _)| k == "format")
        .map(|(_, v)| v.into_owned());
    url.set_query(None);
    if let Some(format) = format {
        url.query_pairs_mut().append_pair("format", &format);
    }
    url.query_pairs_mut().append_pair("name", "medium");
    Ok(url)
}
fn avatar_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| ERROR)?;
    let mut parts = url.path().split('/');
    let path_ok = parts.next() == Some("")
        && parts.next() == Some("profile_images")
        && parts.next().is_some_and(|id| {
            !id.is_empty()
                && id.len() <= 25
                && !id.starts_with('0')
                && id.bytes().all(|c| c.is_ascii_digit())
        })
        && parts.next().is_some_and(|file| {
            file.rsplit_once('.').is_some_and(|(name, ext)| {
                !name.is_empty()
                    && name.len() <= 150
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    && matches!(ext, "jpg" | "jpeg" | "png" | "webp")
            })
        })
        && parts.next().is_none();
    if !path_ok
        || url.scheme() != "https"
        || url.host_str() != Some("pbs.twimg.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ERROR.into());
    }
    Ok(url)
}
fn validate_post(mut post: Post, depth: u8) -> Result<Post, String> {
    if depth > 1 || !valid_id(&post.id) || !valid_handle(&post.handle)
        || post.name.chars().count()>120 || post.text.chars().count()>12000
        || post.time.len()>40 || post.media.len()>4
        || post.repost.as_ref().is_some_and(|s|s.chars().count()>160)
        || post.relation.as_deref().is_some_and(|r|!matches!(r,"reply"|"related"|"unknown"))
        || [&post.stats.replies,&post.stats.reposts,&post.stats.likes,&post.stats.views,&post.stats.bookmarks].iter().any(|s|s.chars().count()>32) {
        return Err(ERROR.into());
    }
    post.media = post.media.iter().map(|u|media_url(u).map(String::from)).collect::<Result<_,_>>()?;
    normalize_gif(&mut post);
    post.avatar = post.avatar.as_deref().map(avatar_url).transpose()?.map(String::from);
    if let Some(quote) = post.quote.take() { post.quote = Some(Box::new(validate_post(*quote, depth+1)?)); }
    if let Some(preview) = post.quote_preview.as_mut() {
        if depth!=0 || post.quote.is_some() || !valid_handle(&preview.handle)
            || preview.name.chars().count()>120 || preview.text.chars().count()>12000
            || preview.time.len()>40 || preview.media.len()>4 { return Err(ERROR.into()); }
        preview.media=preview.media.iter().map(|u|media_url(u).map(String::from)).collect::<Result<_,_>>()?;
        preview.avatar=preview.avatar.as_deref().map(avatar_url).transpose()?.map(String::from);
    }
    Ok(post)
}
fn validated_posts(value: &Value) -> Result<Vec<Post>, String> {
    let values = value
        .as_array()
        .filter(|items| items.len() <= 60)
        .ok_or(ERROR)?;
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let post = validate_post(serde_json::from_value(value.clone()).map_err(|_| ERROR)?, 0)?;
        if seen.insert(post.id.clone()) {
            result.push(post)
        }
    }
    Ok(result)
}
#[cfg(any(windows, test))]
fn trusted_reader_input_scope(root: usize, main: usize, reader: usize, main_parent: usize, reader_parent: usize) -> bool {
    root != 0 && main != 0 && reader != 0 && main != root && reader != root && main != reader
        && main_parent == root && reader_parent == root
}
#[cfg(any(windows, test))]
fn return_reader_focus(was_reader: bool, reader_or_empty: bool, root_foreground: bool, root_enabled: bool, main_enabled: bool) -> bool {
    was_reader && reader_or_empty && root_foreground && root_enabled && main_enabled
}
// Install before navigating the offcanvas reader: no native input or second
// background video player. Rendering stays visible so the page can hydrate.
// The interactive login window and trusted main WebView are not disabled.
#[cfg(windows)]
fn block_reader_media(view: &Webview, session: XSession, revision: u64) -> Result<(), String> {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA,
            ICoreWebView2_8,
        },
        WebResourceRequestedEventHandler,
    };
    use windows::{core::{HSTRING,PWSTR,Interface}, Win32::{
        Foundation::HWND,
        System::Com::{IStream,CoTaskMemFree},
        UI::{Input::KeyboardAndMouse::{EnableWindow,GetFocus,IsWindowEnabled,SetFocus},
            WindowsAndMessaging::{GetForegroundWindow,GetParent,IsChild}},
    }};
    session.current(revision, &AtomicBool::new(false))?;
    let main = view.app_handle().get_webview("main").ok_or(ERROR)?;
    if !view.label().starts_with("x-feed-reader-") || main.label() == view.label()
        || main.window().label() != view.window().label() {
        return Err(ERROR.into());
    }
    // HWND is not Send. Resolve only trusted Tauri identities, then pass their
    // addresses between callbacks. Never call Tauri from with_webview.
    let root = view.window().hwnd().map_err(|_| ERROR)?.0 as usize;
    let (main_tx, main_rx) = std::sync::mpsc::sync_channel(1);
    main.with_webview(move |platform| unsafe {
        let mut hwnd = HWND::default();
        let address = platform.controller().ParentWindow(&mut hwnd).ok().map(|_| hwnd.0 as usize);
        let _ = main_tx.send(address);
    }).map_err(|_| ERROR)?;
    let main_address = main_rx.recv_timeout(Duration::from_secs(2)).ok().flatten().ok_or(ERROR)?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let root = HWND(root as *mut std::ffi::c_void);
        let main = HWND(main_address as *mut std::ffi::c_void);
        let controller = platform.controller();
        let mut reader = HWND::default();
        let safe_input = (|| -> bool {
            if session.revision.load(Ordering::Acquire) != revision
                || controller.ParentWindow(&mut reader).is_err()
                || !trusted_reader_input_scope(root.0 as usize, main.0 as usize, reader.0 as usize,
                    GetParent(main).ok().map_or(0, |h| h.0 as usize),
                    GetParent(reader).ok().map_or(0, |h| h.0 as usize)) {
                return false;
            }
            let before = GetFocus();
            let was_reader = before == reader || IsChild(reader, before).as_bool();
            let _ = EnableWindow(reader, false);
            if IsWindowEnabled(reader).as_bool() { return false; }
            let focused = GetFocus();
            let is_reader = focused == reader || IsChild(reader, focused).as_bool();
            if return_reader_focus(was_reader, is_reader || focused.0.is_null(),
                GetForegroundWindow() == root, IsWindowEnabled(root).as_bool(), IsWindowEnabled(main).as_bool()) {
                // Wry forwards this exact child WM_SETFOCUS to its controller.
                // Do not focus the application/root or any other child/window.
                let _ = SetFocus(Some(main));
            }
            let focused = GetFocus();
            focused != reader && !IsChild(reader, focused).as_bool()
                && session.revision.load(Ordering::Acquire) == revision
        })();
        if !safe_input {
            let _ = tx.send(false);
            return;
        }
        let result = (|| -> windows::core::Result<()> {
            let core = controller.CoreWebView2()?;
            core.cast::<ICoreWebView2_8>()?.SetIsMuted(true)?;
            let environment = platform.environment();
            let mut token = 0;
            core.add_WebResourceRequested(
                &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        let request=args.Request()?;
                        let mut uri=PWSTR::null();
                        if request.Uri(&mut uri).is_ok()&&!uri.is_null(){
                            let value=uri.to_string().unwrap_or_default();
                            CoTaskMemFree(Some(uri.0.cast()));
                            #[cfg(debug_assertions)]
                            if let Ok(url)=Url::parse(&value) {
                                let scheme=match url.scheme(){"https"=>"https","blob"=>"blob","data"=>"data",_=>"other"};
                                let host=match url.host_str(){Some("video.twimg.com")=>"video.twimg.com",Some("x.com")=>"x.com",_=>"other"};
                                let extension=match url.path().rsplit('.').next(){Some("m3u8")=>"m3u8",Some("mp4")=>"mp4",Some("m4s")=>"m4s",Some("ts")=>"ts",Some("aac")=>"aac",_=>"other"};
                                debug_shape("video","request-blocked",&json!({"scheme":scheme,"host":host,"extension":extension,"recognized":video::scope(&url).is_some(),"scopeRejected":video::rejected_scope_shape(&url)}));
                            }
                            video::observed(&session,&value,revision);
                        }
                        let response = environment.CreateWebResourceResponse(
                            None::<&IStream>,
                            204,
                            &HSTRING::from("No Content"),
                            &HSTRING::from("Cache-Control: no-store\r\n"),
                        )?;
                        args.SetResponse(&response)?;
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            core.AddWebResourceRequestedFilter(
                &HSTRING::from("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA,
            )?;
            core.AddWebResourceRequestedFilter(
                &HSTRING::from("https://video.twimg.com/*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            )?;
            Ok(())
        })();
        let _ = tx.send(result.is_ok());
    })
    .map_err(|_| ERROR)?;
    if rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false) {
        Ok(())
    } else {
        Err(ERROR.into())
    }
}
#[cfg(not(windows))]
fn block_reader_media(_: &Webview, _: XSession, _: u64) -> Result<(), String> {
    Err("X 피드의 미디어 차단은 현재 Windows에서 지원됩니다.".into())
}
impl XSession {
    fn registry(&self, app: &AppHandle) -> Result<accounts::Registry, String> {
        let mut saved = self.registry.lock().unwrap_or_else(|p| p.into_inner());
        if saved.is_none() {
            *saved = Some(accounts::Registry::load(app)?);
        }
        Ok(saved.as_ref().unwrap().clone())
    }
    fn remember_owner(
        &self,
        app: &AppHandle,
        profile: &str,
        owner: &str,
        revision: u64,
    ) -> Result<(), String> {
        let mut saved = self.registry.lock().unwrap_or_else(|p| p.into_inner());
        let current = saved.as_ref().ok_or(STALE)?;
        if current.active != profile || self.revision.load(Ordering::Acquire) != revision {
            return Err(STALE.into());
        }
        if current
            .accounts
            .iter()
            .any(|a| a.id == profile && a.handle == owner)
        {
            return Ok(());
        }
        let mut next = current.clone();
        next.remember(profile, owner)?;
        next.save(app)?;
        *saved = Some(next);
        Ok(())
    }
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    fn current(&self, revision: u64, cancel: &AtomicBool) -> Result<(), String> {
        if cancel.load(Ordering::Acquire) || self.revision.load(Ordering::Acquire) != revision {
            Err(STALE.into())
        } else {
            Ok(())
        }
    }
    pub fn detach(&self, app: &AppHandle) {
        self.archive.invalidate_drafts();
        self.archive.cancel_all();
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.generation.fetch_add(1, Ordering::AcqRel);
        surface::close_all(self, app);
        for cancel in self.pending.lock().unwrap_or_else(|p| p.into_inner()).values() {
            cancel.store(true, Ordering::Release)
        }
        *self.data.lock().unwrap_or_else(|p| p.into_inner()) = Data::default();
        *self.videos.lock().unwrap_or_else(|p|p.into_inner())=video::Videos::default();
        for (label, view) in app.webviews() {
            if label.starts_with("x-feed-reader-") {
                let _ = view.close();
            }
        }
    }
    fn suspend(&self, app: &AppHandle) {
        self.archive.invalidate_drafts();
        self.archive.cancel_all();
        self.revision.fetch_add(1, Ordering::AcqRel);
        surface::close_all(self, app);
        for cancel in self.pending.lock().unwrap_or_else(|p| p.into_inner()).values() {
            cancel.store(true, Ordering::Release)
        }
        self.data.lock().unwrap_or_else(|p| p.into_inner()).readers.clear();
        *self.videos.lock().unwrap_or_else(|p|p.into_inner())=video::Videos::default();
        for (label, view) in app.webviews() {
            if label.starts_with("x-feed-reader-") {
                let _ = view.close();
            }
        }
        if let Some(view) = app.get_webview_window(ACCOUNT) {
            let _ = view.close();
        }
    }
    pub fn shutdown(&self, app: &AppHandle) {
        surface::search::shutdown(app);
        self.detach(app);
        for (label, view) in app.webview_windows() {
            if label == ACCOUNT || label.starts_with("x-account-social-") {
                let _ = view.close();
            }
        }
    }
    fn snapshot(&self, app: &AppHandle) -> Result<Snapshot, String> {
        let registry = self.registry(app)?;
        let presentations = account_presentation::snapshot(self, app, &registry);
        Ok(Snapshot {
            status: if self
                .data
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .owner
                .is_some()
            {
                "signedIn"
            } else {
                "unknown"
            },
            generation: self.generation(),
            open: app.get_webview_window(ACCOUNT).is_some(),
            accounts: registry.accounts,
            presentations,
            active: registry.active,
            error: self
                .login_error
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
        })
    }
    fn expire_reader(&self, app: AppHandle, label: String) {
        let host = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let expired = {
                    let mut data = host.data.lock().unwrap_or_else(|p| p.into_inner());
                    let key = data.readers.iter().find(|(_, r)| r.label == label).map(|(k,_)| k.clone());
                    match key {
                        Some(key) => {
                            let reader = &data.readers[&key];
                            if reader.touched.elapsed() < READER_IDLE {
                                false
                            } else {
                                data.readers.remove(&key);
                                true
                            }
                        }
                        _ => return,
                    }
                };
                if expired {
                    if let Some(view) = app.get_webview(&label) {
                        let _ = view.close();
                    }
                    return;
                }
            }
        });
    }
    fn create_reader(
        &self,
        app: &AppHandle,
        feed: &str,
        revision: u64,
        cancel: &AtomicBool,
    ) -> Result<String, String> {
        self.current(revision, cancel)?;
        let old = self
            .data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .readers
            .remove(feed);
        if let Some(old) = old {
            if let Some(view) = app.get_webview(&old.label) {
                let _ = view.close();
            }
        }
        let evicted = {
            let mut data = self.data.lock().unwrap_or_else(|p| p.into_inner());
            if data.readers.len() >= 8 {
                let key = data.readers.iter().max_by_key(|(_,r)|r.touched.elapsed()).map(|(k,_)|k.clone());
                key.and_then(|k|data.readers.remove(&k))
            } else { None }
        };
        if let Some(old) = evicted {
            if let Some(view) = app.get_webview(&old.label) { let _ = view.close(); }
        }
        let label = format!("x-feed-reader-{}", uuid::Uuid::new_v4().simple());
        let name = label.clone();
        let registry = self.registry(app)?;
        let profile = registry.active;
        let owner = registry
            .accounts
            .iter()
            .find(|a| a.id == profile)
            .map(|a| a.handle.clone());
        let directory = accounts::profile(app, &profile)?;
        let owned = app.clone();
        let session = self.clone();
        let wanted = Arc::new(AtomicBool::new(true));
        let live = wanted.clone();
        let target: Arc<Mutex<Option<String>>> = Arc::default();
        let scope = target.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            if !live.load(Ordering::Acquire) || session.revision.load(Ordering::Acquire) != revision
            {
                return;
            }
            if owned
                .state::<AppState>()
                .begin_platform_work(|| Ok(()))
                .is_err()
            {
                let _ = tx.send(Err(ERROR.to_string()));
                return;
            }
            let builder = tauri::webview::WebviewBuilder::new(
                &name,
                WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .focused(false)
            .initialization_script(include_str!("x_reader_accessibility.js").replace(
                "__ATSUMI_X_READER_DIAGNOSTICS__", if cfg!(debug_assertions) { "true" } else { "false" }))
            .initialization_script(include_str!("x_rendered_media.js"))
            .data_directory(directory)
            .browser_extensions_enabled(false)
            .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
            .on_navigation(move |url| {
                let accepted=reader_url(
                    url,
                    scope.lock().unwrap_or_else(|p| p.into_inner()).as_deref(),
                ) || url.as_str() == "about:blank";
                if !accepted&&allowed(url){debug_shape("navigation","blocked",&json!({"path":url.path()}));}
                accepted
            })
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .on_download(|_, _| false);
            // A hidden WebView suspends the site's renderer and never hydrates
            // video. This child is rendered outside the parent's client bounds,
            // clipped on every monitor, with no OS window or focus takeover.
            let Some(parent) = owned.get_window("main") else {
                let _ = tx.send(Err(ERROR.to_string()));
                return;
            };
            let built = parent.add_child(builder,
                tauri::LogicalPosition::new(-4096.0, 0.0),
                tauri::LogicalSize::new(1024.0, 900.0))
                .and_then(|view| { view.set_auto_resize(false)?; Ok(view) });
            if !live.load(Ordering::Acquire)
                || session.revision.load(Ordering::Acquire) != revision
                || owned
                    .state::<AppState>()
                    .begin_platform_work(|| Ok(()))
                    .is_err()
            {
                if let Ok(view) = built {
                    let _ = view.close();
                }
                let _ = tx.send(Err(STALE.into()));
                return;
            }
            let _ = tx.send(built.map(|_| ()).map_err(|_| ERROR.to_string()));
        })
        .map_err(|_| ERROR)?;
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| ERROR.to_string())
            .and_then(|v| v);
        wanted.store(false, Ordering::Release);
        if let Err(error) = result {
            if let Some(view) = app.get_webview(&label) {
                let _ = view.close();
            }
            return Err(error);
        }
        if let Err(error) = self.current(revision, cancel) {
            if let Some(view) = app.get_webview(&label) {
                let _ = view.close();
            }
            return Err(error);
        }
        let view = app.get_webview(&label).ok_or(ERROR)?;
        if let Err(error) = block_reader_media(&view, self.clone(), revision)
            .and_then(|_| self.current(revision, cancel))
            .and_then(|_| {
                view.navigate("https://x.com/home".parse().unwrap())
                    .map_err(|_| ERROR.into())
            })
        {
            let _ = view.close();
            return Err(error);
        }
        let mut data = self.data.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(error) = self.current(revision, cancel) {
            let _ = view.close();
            return Err(error);
        }
        data.readers.insert(feed.into(), Reader {
            label: label.clone(),
            feed: feed.into(),
            cursor: None,
            touched: Instant::now(),
            seen: HashSet::new(),
            target,
            owner,
            profile,
        });
        drop(data);
        self.expire_reader(app.clone(), label.clone());
        Ok(label)
    }
    fn evaluate(
        &self,
        app: &AppHandle,
        label: &str,
        feed: &str,
        advance: bool,
        revision: u64,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        self.current(revision, cancel)?;
        let view = app.get_webview(label).ok_or(STALE)?;
        let (owner, target, destination, binding) = {
            let data = self.data.lock().unwrap_or_else(|p| p.into_inner());
            let reader = data
                .readers
                .get(feed)
                .filter(|r| r.label == label)
                .ok_or(STALE)?;
            let target = reader
                .target
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            let destination = if let Some(id)=feed.strip_prefix("post:") {
                data.posts.get(id).map(|p|format!("/{}/status/{}",p.handle,p.id))
            } else if let Some(handle)=feed.strip_prefix("profile:") {
                // The frontend may open only an author already projected for
                // this account, never an arbitrary URL or another profile's cache.
                data.posts.values().any(|p|p.handle.eq_ignore_ascii_case(handle)
                    ||p.quote.as_ref().is_some_and(|q|q.handle.eq_ignore_ascii_case(handle))
                    ||p.quote_preview.as_ref().is_some_and(|q|q.handle.eq_ignore_ascii_case(handle)))
                    .then(||format!("/{handle}"))
            } else if let Some(id)=feed.strip_prefix("community:") {
                data.communities.iter().find(|c|c.id==id).map(|c|c.path.clone())
            } else { None };
            let binding=feed.strip_prefix("community:").and_then(|id|data.communities.iter().find(|c|c.id==id)).cloned();
            (reader.owner.clone(), target, destination, binding)
        };
        let before = view.url().map_err(|_| ERROR)?;
        if !reader_url(&before, target.as_deref()) {
            return Ok(json!({"state":"pending"}));
        }
        let nonce = uuid::Uuid::new_v4().to_string();
        let script = include_str!("x_projection.js").replace(
            "__ATSUMI_X_REQUEST__",
            &json!({"nonce":nonce,"feed":feed,"advance":advance,"owner":owner,"target":target,"destination":destination,"binding":binding,"diagnostics":cfg!(debug_assertions)})
                .to_string(),
        );
        #[cfg(debug_assertions)]
        let script = format!("(()=>{{const result=({script});if(result?.shape){{try{{result.shape.readerAccessibility=window.__atsumiXReaderAccessibility?.snapshot()??{{installed:false}};}}catch{{result.shape.readerAccessibility={{installed:false}};}}}}return result;}})()");
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        view.eval_with_callback(script, move |value| {
            let _ = tx.send(value);
        })
        .map_err(|_| ERROR)?;
        let raw = rx.recv_timeout(Duration::from_secs(2)).map_err(|_| ERROR)?;
        self.current(revision, cancel)?;
        if raw.len() > 1024 * 1024 {
            return Err(ERROR.into());
        }
        if !view.url().is_ok_and(|u|u==before&&reader_url(&u,target.as_deref())) {
            return Ok(json!({"state":"pending"}));
        }
        let value: Value = serde_json::from_str(&raw).map_err(|_| ERROR)?;
        if value["nonce"] != nonce {
            return Err(ERROR.into());
        }
        Ok(value)
    }
    fn load(
        &self,
        app: &AppHandle,
        feed: &str,
        cursor: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<Page, String> {
        if !valid_feed(feed) {
            return Err(ERROR.into());
        }
        {
            let data=self.data.lock().unwrap_or_else(|p|p.into_inner());
            if feed.strip_prefix("post:").is_some_and(|id|!data.posts.contains_key(id))
                || feed.strip_prefix("profile:").is_some_and(|handle|!data.posts.values().any(|p|p.handle.eq_ignore_ascii_case(handle)||p.quote.as_ref().is_some_and(|q|q.handle.eq_ignore_ascii_case(handle))||p.quote_preview.as_ref().is_some_and(|q|q.handle.eq_ignore_ascii_case(handle))))
                || feed.strip_prefix("community:").is_some_and(|id|!data.communities.iter().any(|c|c.id==id)) {
                return Err(STALE.into());
            }
        }
        let revision = self.revision.load(Ordering::Acquire);
        let label = if let Some(cursor) = cursor {
            let data = self.data.lock().unwrap_or_else(|p| p.into_inner());
            let reader = data.readers.get(feed).ok_or(STALE)?;
            if reader.feed != feed
                || reader.cursor.as_deref() != Some(cursor)
                || reader.touched.elapsed() > READER_IDLE
                || uuid::Uuid::parse_str(cursor).is_err()
            {
                return Err(STALE.into());
            }
            reader.label.clone()
        } else {
            self.create_reader(app, feed, revision, cancel)?
        };
        if cursor.is_some() {
            let advanced = self.evaluate(app, &label, feed, true, revision, cancel)?;
            if advanced["state"] == "signedOut" || advanced["state"] == "ownerChanged" {
                self.detach(app);
                return Ok(Page {
                    posts: vec![],
                    cursor: None,
                    generation: self.generation(),
                    status: "signedOut",
                    limited: false, communities: vec![],
                });
            }
            if advanced["state"] != "advanced" {
                return Err(STALE.into());
            }
        }
        let started = Instant::now();
        let mut observed_ready = false;
        let mut navigated = false;
        let mut unsupported = false;
        let mut last_shape = Value::Null;
        let mut first_ready:Option<Instant>=None;
        let mut stable_since=Instant::now();
        let mut fingerprint=String::new();
        let mut empty_advances=0;
        while started.elapsed() < Duration::from_secs(25) {
            let value = self.evaluate(app, &label, feed, false, revision, cancel)?;
            if !value["shape"].is_null() { last_shape=value["shape"].clone(); }
            if value["state"] == "signedOut" || value["state"] == "ownerChanged" {
                self.detach(app);
                return Ok(Page {
                    posts: vec![],
                    cursor: None,
                    generation: self.generation(),
                    status: "signedOut",
                    limited: false, communities: vec![],
                });
            }
            if value["state"] == "navigate" {
                if cursor.is_some() || navigated {
                    return Err(STALE.into());
                }
                let owner = value["owner"]
                    .as_str()
                    .filter(|v| valid_handle(v))
                    .ok_or(ERROR)?;
                let path = value["path"]
                    .as_str()
                    .filter(|v| feed_target(feed, owner, v))
                    .ok_or(ERROR)?;
                {
                    let mut data = self.data.lock().unwrap_or_else(|p| p.into_inner());
                    self.current(revision, cancel)?;
                    let communities=validated_communities(&value["communities"])?;
                    if !communities.is_empty() { data.communities=communities; }
                    let reader = data
                        .readers
                        .get_mut(feed)
                        .filter(|r| r.label == label)
                        .ok_or(STALE)?;
                    reader.owner = Some(owner.into());
                    *reader.target.lock().unwrap_or_else(|p| p.into_inner()) = Some(path.into());
                }
                self.current(revision, cancel)?;
                app.get_webview(&label)
                    .ok_or(STALE)?
                    .navigate(format!("https://x.com{path}").parse().map_err(|_| ERROR)?)
                    .map_err(|_| ERROR)?;
                navigated = true;
            }
            if value["state"] == "unsupported" {
                unsupported = true;
            }
            if value["state"] == "ready" || value["state"] == "empty" {
                let first=first_ready.get_or_insert_with(Instant::now);
                let next=value["posts"].to_string();
                if fingerprint!=next{fingerprint=next;stable_since=Instant::now();}
                if first.elapsed()<Duration::from_millis(1400)||(stable_since.elapsed()<Duration::from_millis(440)&&first.elapsed()<Duration::from_secs(3)){
                    std::thread::sleep(Duration::from_millis(220));continue;
                }
                let owner = value["owner"]
                    .as_str()
                    .filter(|v| valid_handle(v))
                    .ok_or(ERROR)?;
                let path = value["path"]
                    .as_str()
                    .filter(|v| feed_target(feed, owner, v))
                    .ok_or(ERROR)?;
                if !app
                    .get_webview(&label)
                    .and_then(|view| view.url().ok())
                    .is_some_and(|url| allowed(&url) && url.path() == path)
                {
                    return Err(STALE.into());
                }
                let mut posts = validated_posts(&value["posts"])?;
                debug_reader_shape(app, &label, feed, value["state"].as_str().unwrap_or("unknown"), &last_shape);
                let profile = self
                    .data
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .readers
                    .get(feed)
                    .filter(|r| r.label == label)
                    .ok_or(STALE)?
                    .profile
                    .clone();
                self.remember_owner(app, &profile, owner, revision)?;
                let mut data = self.data.lock().unwrap_or_else(|p| p.into_inner());
                self.current(revision, cancel)?;
                if data.owner.as_deref() != Some(owner) {
                    if cursor.is_some() {
                        drop(data);
                        self.detach(app);
                        return Err(STALE.into());
                    }
                    data.owner = Some(owner.into());
                    data.posts.clear();
                    data.order.clear();
                    data.images.clear();
                    data.communities.clear();
                    data.observed_posters.clear();
                    self.generation.fetch_add(1, Ordering::AcqRel);
                }
                let communities=validated_communities(&value["communities"])?;
                if !communities.is_empty() { data.communities=communities; }
                let communities=data.communities.clone();
                retain_observed_posters(&mut data,&mut posts);
                let reader = data
                    .readers
                    .get_mut(feed)
                    .filter(|r| r.label == label)
                    .ok_or(STALE)?;
                if reader.owner.as_deref().is_some_and(|bound| bound != owner) {
                    drop(data);
                    self.detach(app);
                    return Err(STALE.into());
                }
                reader.owner = Some(owner.into());
                if value["state"] == "empty" {
                    if !posts.is_empty() || cursor.is_some() {
                        return Err(ERROR.into());
                    }
                    reader.cursor = None;
                    reader.touched = Instant::now();
                    return Ok(Page {
                        posts: vec![],
                        cursor: None,
                        generation: self.generation(),
                        status: "signedIn",
                        limited: false, communities,
                    });
                }
                let fresh: Vec<_> = posts
                    .into_iter()
                    .filter(|p| !reader.seen.contains(&p.id))
                    .take(MAX_POSTS.saturating_sub(reader.seen.len()))
                    .collect();
                observed_ready = true;
                if !fresh.is_empty() {
                    for post in &fresh {
                        reader.seen.insert(post.id.clone());
                    }
                    let limited = reader.seen.len() >= MAX_POSTS;
                    let token = (!limited).then(|| uuid::Uuid::new_v4().to_string());
                    reader.cursor = token.clone();
                    reader.touched = Instant::now();
                    for post in &fresh {
                        if let Some(quote)=&post.quote {
                            if !data.posts.contains_key(&quote.id) { data.order.push_back(quote.id.clone()); }
                            data.posts.insert(quote.id.clone(), *quote.clone());
                        }
                        if !data.posts.contains_key(&post.id) {
                            data.order.push_back(post.id.clone());
                        }
                        data.posts.insert(post.id.clone(), post.clone());
                    }
                    while data.order.len() > MAX_POSTS {
                        if let Some(id) = data.order.pop_front() {
                            data.posts.remove(&id);
                        }
                    }
                    return Ok(Page {
                        posts: fresh,
                        cursor: token,
                        generation: self.generation(),
                        status: "signedIn",
                        limited, communities,
                    });
                }
                if cursor.is_some()&&empty_advances<3 {
                    drop(data);
                    self.evaluate(app,&label,feed,true,revision,cancel)?;
                    empty_advances+=1;first_ready=None;fingerprint.clear();
                }
            }
            std::thread::sleep(Duration::from_millis(220));
        }
        debug_reader_shape(app, &label, feed, "timeout", &last_shape);
        if observed_ready && cursor.is_some() {
            return Ok(Page {
                posts: vec![],
                cursor: None,
                generation: self.generation(),
                status: "signedIn",
                limited: true, communities: self.data.lock().unwrap_or_else(|p|p.into_inner()).communities.clone(),
            });
        }
        if unsupported {
            return Err(if feed=="communities"{"가입된 커뮤니티 전용 타임라인을 확인하지 못했습니다. 추천 커뮤니티 게시물은 대신 표시하지 않습니다."}else{"X 목록 구성을 확인하지 못했습니다. 공식 화면에서 한국어 또는 영어 목록을 확인해 주세요."}.into());
        }
        Err(ERROR.into())
    }
}

// Local, debug-only bounded structure diagnostics. No post text, account names,
// image bodies, credentials, HTML or arbitrary IPC arguments are recorded.
fn debug_shape(feed:&str,status:&str,shape:&Value){
    diagnostics::record(feed,status,shape);
}

// Read-only provenance: UIA's focused document is not necessarily the HWND that
// receives keys. Never focus/enable a window from a diagnostic observation.
#[cfg(all(debug_assertions, windows))]
fn reader_native_focus_shape(view: &Webview) -> Option<Value> {
    use windows::Win32::{
        Foundation::HWND,
        UI::{Input::KeyboardAndMouse::{GetFocus,IsWindowEnabled},
            WindowsAndMessaging::{GetForegroundWindow,GetParent,IsChild}},
    };
    if !view.label().starts_with("x-feed-reader-") || view.window().label() != "main" { return None; }
    let root = view.window().hwnd().ok()?.0 as usize;
    let (tx,rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let root = HWND(root as *mut std::ffi::c_void);
        let mut reader = HWND::default();
        if platform.controller().ParentWindow(&mut reader).is_err()
            || reader.0.is_null() || reader == root || GetParent(reader).ok() != Some(root) {
            let _ = tx.send(None);
            return;
        }
        let focused = GetFocus();
        let _ = tx.send(Some(json!({
            "trustedReaderParent":true,
            "readerDisabled":!IsWindowEnabled(reader).as_bool(),
            "readerHasNativeFocus":focused==reader||IsChild(reader,focused).as_bool(),
            "applicationForeground":GetForegroundWindow()==root,
        })));
    }).ok()?;
    rx.recv_timeout(Duration::from_millis(250)).ok().flatten()
}
fn debug_reader_shape(app:&AppHandle,label:&str,feed:&str,status:&str,shape:&Value){
    #[cfg(debug_assertions)] {
        let mut shape=shape.clone();
        #[cfg(windows)]
        if let Some(fields)=shape.as_object_mut(){
            fields.insert("readerNativeFocus".into(), app.get_webview(label)
                .and_then(|view|reader_native_focus_shape(&view)).unwrap_or(Value::Null));
        }
        debug_shape(feed,status,&shape);
    }
    #[cfg(not(debug_assertions))]
    let _=(app,label,feed,status,shape);
}

#[tauri::command]
pub fn x_account_status(window: Webview, session: State<'_, XSession>) -> Result<Snapshot, String> {
    trusted(&window)?;
    session.snapshot(window.app_handle())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn x_account_open(
    window: Webview,
    state: State<'_, AppState>,
    session: State<'_, XSession>,
    add: Option<bool>,
) -> Result<Snapshot, String> {
    trusted(&window)?;
    state.begin_platform_work(|| Ok(()))?;
    let _lane = session.account_lane.lock().await;
    let app = window.app_handle().clone();
    if let Some(view) = app.get_webview_window(ACCOUNT) {
        let _ = view.show();
        let _ = view.set_focus();
        return session.snapshot(&app);
    }
    let registry = session.registry(&app)?;
    if add == Some(true) && registry.accounts.len() >= 20 {
        return Err("저장 가능한 X 계정은 최대 20개입니다.".into());
    }
    let profile = if add == Some(true) {
        uuid::Uuid::new_v4().simple().to_string()
    } else {
        registry.active
    };
    let directory = accounts::profile(&app, &profile)?;
    *session
        .login_error
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = None;
    session.detach(&app);
    let revision = session.revision.load(Ordering::Acquire);
    let active = session.inner().clone();
    let popups = crate::browser_identity::LoginPopups::new(
        crate::browser_identity::Service::X,
        ACCOUNT,
        move || active.revision.load(Ordering::Acquire) == revision,
    )
    .with_profile(directory.clone());
    let popup = popups.clone();
    let popup_app = app.clone();
    let built = WebviewWindowBuilder::new(
        &app,
        ACCOUNT,
        WebviewUrl::External("https://x.com/i/flow/login".parse().unwrap()),
    )
    .title("X 로그인")
    .inner_size(1000.0, 800.0)
    .data_directory(directory)
    .browser_extensions_enabled(false)
    .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
    .on_navigation(|url| {
        crate::browser_identity::account_url(crate::browser_identity::Service::X, url)
    })
    .on_new_window(move |url, features| popup.open(&popup_app, url, features))
    .on_download(|_, _| false)
    .build()
    .map_err(|_| ERROR)?;
    if session.revision.load(Ordering::Acquire) != revision
        || state.begin_platform_work(|| Ok(())).is_err()
    {
        popups.close(&app);
        let _ = built.close();
        return Err(STALE.into());
    }
    let owned = app.clone();
    let host = session.inner().clone();
    built.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            popups.close(&owned);
            if host.revision.load(Ordering::Acquire) == revision {
                host.detach(&owned);
            }
        }
    });
    let host = session.inner().clone();
    let owned = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last = None;
        // Two independent rendered owner observations; login/challenge pages
        // never complete a flow. No click, credential or hidden-state access.
        for _ in 0..1200 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if host.revision.load(Ordering::Acquire) != revision
                || owned.get_webview_window(ACCOUNT).is_none()
            {
                return;
            }
            let probe_app = owned.clone();
            let identity = tauri::async_runtime::spawn_blocking(move || login_identity(&probe_app))
                .await
                .ok()
                .flatten();
            let Some(owner) = stable_login_owner(&mut last, identity) else {
                continue;
            };
            let _lane = host.account_lane.lock().await;
            if host.revision.load(Ordering::Acquire) != revision
                || owned.get_webview_window(ACCOUNT).is_none()
            {
                return;
            }
            let result = (|| -> Result<(), String> {
                let mut saved = host.registry.lock().unwrap_or_else(|p| p.into_inner());
                let mut next = saved.as_ref().ok_or(STALE)?.clone();
                next.remember(&profile, &owner)?;
                next.active = profile.clone();
                next.save(&owned)?;
                *saved = Some(next);
                Ok(())
            })();
            if let Err(error) = result {
                *host.login_error.lock().unwrap_or_else(|p| p.into_inner()) = Some(error);
                return; // Keep the official window and old active account intact.
            }
            host.detach(&owned);
            if let Some(view) = owned.get_webview_window(ACCOUNT) {
                let _ = view.close();
            }
            return;
        }
        if host.revision.load(Ordering::Acquire) == revision {
            *host.login_error.lock().unwrap_or_else(|p| p.into_inner()) =
                Some("로그인 확인 시간이 초과되었습니다. 창을 닫고 다시 시도해 주세요.".into());
        }
    });
    session.snapshot(&app)
}
fn stable_login_owner(previous: &mut Option<String>, observed: Option<String>) -> Option<String> {
    let confirmed = observed
        .as_ref()
        .filter(|owner| previous.as_ref() == Some(owner))
        .cloned();
    *previous = observed;
    confirmed
}
fn login_identity(app: &AppHandle) -> Option<String> {
    let view = app.get_webview_window(ACCOUNT)?;
    let before = view.url().ok()?;
    if !allowed(&before) || before.path() == "/login" || before.path().starts_with("/i/flow/") {
        return None;
    }
    let nonce = uuid::Uuid::new_v4().to_string();
    let script = include_str!("x_identity.js")
        .replace("__ATSUMI_X_NONCE__", &serde_json::to_string(&nonce).ok()?);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(script, move |value| {
        let _ = tx.send(value);
    })
    .ok()?;
    let raw = rx.recv_timeout(Duration::from_secs(2)).ok()?;
    if raw.len() > 2048 || view.url().ok()? != before {
        return None;
    }
    let value: Value = serde_json::from_str(&raw).ok()?;
    let owner = value["handle"].as_str()?;
    (value["nonce"] == nonce && valid_handle(owner)).then(|| owner.to_string())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn x_account_select(
    window: Webview,
    state: State<'_, AppState>,
    session: State<'_, XSession>,
    account_id: String,
) -> Result<Snapshot, String> {
    trusted(&window)?;
    state.begin_platform_work(|| Ok(()))?;
    let _lane = session.account_lane.lock().await;
    let app = window.app_handle();
    if app.get_webview_window(ACCOUNT).is_some() {
        return Err("진행 중인 로그인 창을 먼저 닫아 주세요.".into());
    }
    let mut next = session.registry(app)?;
    if !next.accounts.iter().any(|a| a.id == account_id) {
        return Err("등록된 X 계정을 선택해 주세요.".into());
    }
    next.active = account_id;
    {
        let mut saved = session.registry.lock().unwrap_or_else(|p| p.into_inner());
        next.save(app)?;
        *saved = Some(next);
    }
    session.detach(app);
    *session
        .login_error
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = None;
    session.snapshot(app)
}
#[tauri::command(rename_all = "camelCase")]
pub async fn x_feed_load(
    window: Webview,
    state: State<'_, AppState>,
    session: State<'_, XSession>,
    feed: String,
    cursor: Option<String>,
    request_id: String,
) -> Result<Page, String> {
    trusted(&window)?;
    state.begin_platform_work(|| Ok(()))?;
    if uuid::Uuid::parse_str(&request_id).is_err() {
        return Err(ERROR.into());
    }
    if window.app_handle().get_webview_window(ACCOUNT).is_some() {
        return Err("공식 X 로그인 창을 닫은 뒤 불러와 주세요.".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut pending = session.pending.lock().unwrap_or_else(|p| p.into_inner());
        if pending.len() >= 8 || pending.contains_key(&request_id) {
            return Err("진행 중인 X 요청이 많습니다. 잠시 후 다시 시도해 주세요.".into());
        }
        pending.insert(request_id.clone(), cancel.clone());
    }
    // A cancelled worker must finish its native window cleanup before another
    // worker can replace that reader. Cancellation remains immediately visible.
    let _lane = session.feed_lane.lock().await;
    let app = window.app_handle().clone();
    let host = session.inner().clone();
    let done = host.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        host.load(&app, &feed, cursor.as_deref(), &cancel)
    })
    .await
    .map_err(|_| ERROR.to_string())
    .and_then(|v| v);
    let mut pending = done.pending.lock().unwrap_or_else(|p| p.into_inner());
    pending.remove(&request_id);
    outcome
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_feed_cancel(
    window: Webview,
    session: State<'_, XSession>,
    request_id: String,
) -> Result<(), String> {
    trusted(&window)?;
    if let Some(cancel) = session
        .pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&request_id)
    {
        cancel.store(true, Ordering::Release)
    }
    Ok(())
}
#[tauri::command]
pub fn x_feed_suspend(window: Webview, session: State<'_, XSession>) -> Result<(), String> {
    trusted(&window)?;
    session.suspend(window.app_handle());
    Ok(())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn x_image_get(
    window: Webview,
    state: State<'_, AppState>,
    session: State<'_, XSession>,
    post_id: String,
    index: i32,
    generation: u64,
) -> Result<String, String> {
    trusted(&window)?;
    state.begin_platform_work(|| Ok(()))?;
    let host = session.inner().clone();
    let _permit = host.image_slots.acquire().await.map_err(|_| ERROR)?;
    let source = {
        let data = host.data.lock().unwrap_or_else(|p| p.into_inner());
        if host.generation() != generation {
            return Err(STALE.into());
        }
        data.posts
            .get(&post_id)
            .and_then(|p| {
                if index == -2 {
                    p.quote_preview.as_ref().and_then(|q|q.avatar.as_ref())
                } else if index == -1 {
                    p.avatar.as_ref()
                } else if (4..8).contains(&index) {
                    p.quote_preview.as_ref().and_then(|q|q.media.get((index-4) as usize))
                } else {
                    usize::try_from(index).ok().and_then(|i| p.media.get(i))
                }
            })
            .cloned()
            .ok_or(
                "이미지의 탐색 상태가 만료되었습니다. 피드를 새로고침해 주세요. (IMAGE_CONTEXT)",
            )?
    };
    let key = format!("{generation}:{post_id}:{index}:{source}");
    {
        let data=host.data.lock().unwrap_or_else(|p|p.into_inner());
        if host.generation()!=generation {return Err(STALE.into());}
        if let Some((_,cached))=data.images.iter().find(|(k,_)|k==&key) {return Ok(cached.clone());}
    }
    let worker = host.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        if worker.generation() != generation {
            return Err(STALE.into());
        }
        let url = if index < 0 {
            avatar_url(&source)?
        } else {
            media_url(&source)?
        };
        let response = reqwest::blocking::Client::builder()
            .user_agent("Atsumi/2.0")
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "X 이미지 연결을 준비하지 못했습니다. (IMAGE_CLIENT)")?
            .get(url)
            .header(reqwest::header::ACCEPT, "image/jpeg,image/png,image/webp")
            .header(reqwest::header::REFERER, "https://x.com/")
            .send()
            .map_err(|error| {
                if error.is_timeout() {
                    "X 이미지 응답 시간이 초과되었습니다. (IMAGE_TIMEOUT)"
                } else {
                    "X 이미지 서버에 연결하지 못했습니다. (IMAGE_NETWORK)"
                }
            })?;
        if !response.status().is_success() {
            return Err(format!(
                "X 이미지 서버 응답 오류 (IMAGE_HTTP_{})",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|n| n > 8 * 1024 * 1024)
        {
            return Err("X 이미지 크기 제한을 초과했습니다. (IMAGE_SIZE)".into());
        }
        let mut bytes = Vec::new();
        response
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "X 이미지를 끝까지 읽지 못했습니다. (IMAGE_READ)")?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("X 이미지 크기 제한을 초과했습니다. (IMAGE_SIZE)".into());
        }
        if worker.generation() != generation {
            return Err(STALE.into());
        }
        let result = image_data(&bytes, index < 0)?;
        let mut data = worker.data.lock().unwrap_or_else(|p| p.into_inner());
        if worker.generation() != generation {
            return Err(STALE.into());
        }
        data.images.push_back((key, result.clone()));
        while data.images.len() > 48
            || data.images.iter().map(|(_, s)| s.len()).sum::<usize>() > 12 * 1024 * 1024
        {
            data.images.pop_front();
        }
        Ok(result)
    })
    .await
    .map_err(|_| ERROR)?;
    if let Err(error) = &outcome {
        if let Some(diagnostics) = window
            .app_handle()
            .try_state::<Arc<crate::ui_diagnostics::UiDiagnostics>>()
        {
            diagnostics.record(
                "x_image_error",
                json!({"reason":error,"avatar":index == -1}),
            );
        }
    }
    outcome
}
fn image_data(bytes: &[u8], avatar: bool) -> Result<String, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "X 이미지 형식을 읽지 못했습니다. (IMAGE_FORMAT)")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| "X 이미지가 손상됐거나 지원하지 않는 형식입니다. (IMAGE_DECODE)")?;
    // thumbnail() also upscales small originals. Never inflate avatars/media;
    // a bounded smaller display copy is preferable to rejecting a valid image.
    for maximum in if avatar {
        [96, 72, 48]
    } else {
        [1280, 960, 640]
    } {
        let reduced = if decoded.width().max(decoded.height()) > maximum {
            decoded.thumbnail(maximum, maximum).to_rgb8()
        } else {
            decoded.to_rgb8()
        };
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 82)
            .encode_image(&reduced)
            .map_err(|_| "X 이미지 표시용 변환 오류 (IMAGE_ENCODE)")?;
        if jpeg.len() <= 1024 * 1024 {
            return Ok(format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg)));
        }
    }
    Err("X 이미지 표시용 크기 제한을 초과했습니다. (IMAGE_OUTPUT_SIZE)".into())
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_post_open(
    window: Webview,
    session: State<'_, XSession>,
    post_id: String,
) -> Result<(), String> {
    trusted(&window)?;
    let url = {
        let data = session.data.lock().unwrap_or_else(|p| p.into_inner());
        let post = data.posts.get(&post_id).ok_or(STALE)?;
        format!("https://x.com/{}/status/{}", post.handle, post.id)
    };
    #[cfg(windows)]
    {
        use windows::{
            core::PCWSTR,
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        let wide = url.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let result = unsafe {
            ShellExecuteW(
                None,
                windows::core::w!("open"),
                PCWSTR(wide.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize <= 32 {
            return Err(ERROR.into());
        }
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        return Err(ERROR.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reader_input_requires_distinct_trusted_siblings() {
        assert!(trusted_reader_input_scope(1, 2, 3, 1, 1));
        for (root, main, reader, main_parent, reader_parent) in [
            (0, 2, 3, 0, 0), (1, 0, 3, 1, 1), (1, 2, 0, 1, 1),
            (1, 1, 3, 1, 1), (1, 2, 1, 1, 1), (1, 2, 2, 1, 1),
            (1, 2, 3, 4, 1), (1, 2, 3, 1, 4),
        ] {
            assert!(!trusted_reader_input_scope(root, main, reader, main_parent, reader_parent));
        }
    }
    #[test]
    fn reader_focus_return_never_steals_main_modal_or_another_apps_focus() {
        assert!(return_reader_focus(true, true, true, true, true));
        for state in [
            [false, true, true, true, true], [true, false, true, true, true],
            [true, true, false, true, true], [true, true, true, false, true],
            [true, true, true, true, false],
        ] {
            assert!(!return_reader_focus(state[0], state[1], state[2], state[3], state[4]));
        }
    }
    #[test]
    fn poster_retention_requires_current_owner_handle_video_and_recent_evidence() {
        let raw=json!({"id":"123456789","handle":"fixture","name":"name","text":"text","time":"","media":["https://pbs.twimg.com/amplify_video_thumb/987654321/img/Poster.jpg"],"video":true,"truncated":false});
        let post=validated_posts(&json!([raw])).unwrap().remove(0);
        let mut data=Data::default();data.owner=Some("alice".into());
        retain_observed_posters(&mut data,&mut [post.clone()]);
        let captured=data.observed_posters[&post.id].at;
        let mut empty=post.clone();empty.media.clear();
        let mut fresh=empty.clone();retain_observed_posters(&mut data,std::slice::from_mut(&mut fresh));
        assert_eq!(fresh.media,post.media);assert_eq!(data.observed_posters[&post.id].at,captured);
        let mut wrong=empty.clone();wrong.handle="other".into();retain_observed_posters(&mut data,std::slice::from_mut(&mut wrong));assert!(wrong.media.is_empty());
        data.observed_posters.get_mut(&post.id).unwrap().at=Instant::now()-Duration::from_secs(301);
        let mut expired=empty.clone();retain_observed_posters(&mut data,std::slice::from_mut(&mut expired));assert!(expired.media.is_empty());
        retain_observed_posters(&mut data,&mut [post.clone()]);data.owner=Some("bob".into());
        let mut changed=empty.clone();retain_observed_posters(&mut data,std::slice::from_mut(&mut changed));assert!(changed.media.is_empty());
        retain_observed_posters(&mut data,&mut [post.clone()]);empty.video=false;
        retain_observed_posters(&mut data,std::slice::from_mut(&mut empty));assert!(empty.media.is_empty());assert!(data.observed_posters.is_empty());
    }
    #[test]
    fn restored_own_gif_posters_restore_loop_classification_without_renewing_ttl() {
        let raw=json!({"id":"123456789","handle":"fixture","name":"name","text":"text","time":"","media":["https://pbs.twimg.com/tweet_video_thumb/GifFixture.jpg"],"video":true,"gif":false,"truncated":false});
        let post=validated_posts(&json!([raw.clone()])).unwrap().remove(0);
        assert!(post.gif);
        let mut data=Data::default();data.owner=Some("alice".into());
        retain_observed_posters(&mut data,&mut [post.clone()]);
        let captured=data.observed_posters[&post.id].at;
        let mut detail_raw=raw;detail_raw["media"]=json!([]);
        let mut detail=validated_posts(&json!([detail_raw])).unwrap().remove(0);
        assert!(!detail.gif);
        retain_observed_posters(&mut data,std::slice::from_mut(&mut detail));
        assert_eq!(detail.media,post.media);assert!(detail.gif);
        assert_eq!(data.observed_posters[&post.id].at,captured);
    }
    #[test]
    fn gif_retention_does_not_cross_owner_handle_ttl_or_video_boundaries() {
        let raw=json!({"id":"123456789","handle":"fixture","name":"name","text":"text","time":"","media":["https://pbs.twimg.com/tweet_video_thumb/GifFixture.jpg"],"video":true,"truncated":false});
        let post=validated_posts(&json!([raw])).unwrap().remove(0);
        for boundary in ["owner","handle","expired","non_video"]{
            let mut data=Data::default();data.owner=Some("alice".into());
            retain_observed_posters(&mut data,&mut [post.clone()]);
            let mut detail=post.clone();detail.media.clear();detail.gif=false;
            match boundary{
                "owner"=>data.owner=Some("bob".into()),
                "handle"=>detail.handle="other".into(),
                "expired"=>data.observed_posters.get_mut(&post.id).unwrap().at=Instant::now()-READER_IDLE-Duration::from_secs(1),
                "non_video"=>{detail.video=false;detail.gif=true;},
                _=>unreachable!(),
            }
            retain_observed_posters(&mut data,std::slice::from_mut(&mut detail));
            assert!(detail.media.is_empty(),"{boundary}");assert!(!detail.gif,"{boundary}");
        }
    }
    #[test]
    fn gif_normalization_requires_exact_validated_own_thumbnail_and_video() {
        let raw=json!({"id":"123456789","handle":"fixture","name":"name","text":"text","time":"","media":[],"video":true,"gif":true,"truncated":false});
        let mut post=validated_posts(&json!([raw])).unwrap().remove(0);
        assert!(!post.gif);
        for suffix in ["jpg","jpeg","png","webp"]{
            post.media=vec![format!("https://pbs.twimg.com/tweet_video_thumb/GifFixture.{suffix}")];
            normalize_gif(&mut post);assert!(post.gif);
        }
        for url in [
            "https://pbs.twimg.com/amplify_video_thumb/987654321/img/Poster.jpg",
            "https://pbs.twimg.com/media/tweet_video_thumb.jpg",
            "https://pbs.twimg.com/tweet_video_thumb/GifFixture",
            "https://pbs.twimg.com/tweet_video_thumb/GifFixture.gif",
            "https://pbs.twimg.com/tweet_video_thumb/nested/GifFixture.jpg",
            "https://pbs.twimg.com/tweet_video_thumb/GifFixture.jpg?token=not_allowed",
            "https://evil.test/tweet_video_thumb/GifFixture.jpg",
            "https://user@pbs.twimg.com/tweet_video_thumb/GifFixture.jpg",
        ]{
            post.gif=true;post.media=vec![url.into()];normalize_gif(&mut post);assert!(!post.gif,"{url}");
        }
        post.media=vec!["https://pbs.twimg.com/tweet_video_thumb/GifFixture.jpg".into()];post.video=false;
        normalize_gif(&mut post);assert!(!post.gif);
    }
    #[test]
    fn quoted_gif_does_not_mark_the_enclosing_post_as_a_gif() {
        let raw=json!({"id":"123456789","handle":"fixture","name":"name","text":"text","time":"","media":["https://pbs.twimg.com/amplify_video_thumb/987654321/img/Poster.jpg"],"video":true,"gif":true,"truncated":false,
            "quote":{"id":"987654321","handle":"quoted","name":"quote","text":"text","time":"","media":["https://pbs.twimg.com/tweet_video_thumb/QuotedGif.jpg"],"video":true,"gif":false,"truncated":false}});
        let mut post=validated_posts(&json!([raw])).unwrap().remove(0);
        assert!(!post.gif);assert!(post.quote.as_ref().unwrap().gif);
        let mut data=Data::default();data.owner=Some("alice".into());
        retain_observed_posters(&mut data,std::slice::from_mut(&mut post));
        assert!(!post.gif);assert!(post.quote.as_ref().unwrap().gif);
        assert!(!data.observed_posters.contains_key("987654321"));
    }
    #[test]
    fn login_completion_requires_two_consecutive_matching_observations() {
        let mut previous = None;
        assert_eq!(stable_login_owner(&mut previous, Some("one".into())), None);
        assert_eq!(stable_login_owner(&mut previous, None), None);
        assert_eq!(stable_login_owner(&mut previous, Some("one".into())), None);
        assert_eq!(stable_login_owner(&mut previous, Some("two".into())), None);
        assert_eq!(
            stable_login_owner(&mut previous, Some("two".into())),
            Some("two".into())
        );
    }
    #[test]
    fn avatars_are_known_bounded_raster_paths_only() {
        assert!(
            avatar_url("https://pbs.twimg.com/profile_images/123456789/photo_normal.jpg").is_ok()
        );
        for url in [
            "http://pbs.twimg.com/profile_images/123/a.jpg",
            "https://pbs.twimg.com.evil.test/profile_images/123/a.jpg",
            "https://user@pbs.twimg.com/profile_images/123/a.jpg",
            "https://pbs.twimg.com/profile_images/123/a.svg",
            "https://pbs.twimg.com/profile_images/123/a.jpg?token=secret",
            "https://pbs.twimg.com/profile_images/../a.jpg",
            "https://pbs.twimg.com/media/a.jpg",
        ] {
            assert!(avatar_url(url).is_err(), "{url}");
        }
    }
    #[test]
    fn native_media_conversion_accepts_supported_rasters_and_preserves_limits() {
        let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            160,
            120,
            image::Rgb([40, 90, 120]),
        ));
        for format in [
            image::ImageFormat::Jpeg,
            image::ImageFormat::Png,
            image::ImageFormat::WebP,
        ] {
            let mut encoded = std::io::Cursor::new(Vec::new());
            source.write_to(&mut encoded, format).unwrap();
            let result = image_data(encoded.get_ref(), false).unwrap();
            let jpeg = STANDARD
                .decode(result.strip_prefix("data:image/jpeg;base64,").unwrap())
                .unwrap();
            assert_eq!(image::load_from_memory(&jpeg).unwrap().width(), 160);
            let avatar = image_data(encoded.get_ref(), true).unwrap();
            let bytes = STANDARD
                .decode(avatar.strip_prefix("data:image/jpeg;base64,").unwrap())
                .unwrap();
            assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 96);
        }
        assert!(image_data(b"not an image", false)
            .unwrap_err()
            .contains("IMAGE_DECODE"));
    }
    #[test]
    fn exact_origin_and_reader_paths() {
        for v in ["https://x.com/home", "https://www.x.com/i/flow/login"] {
            assert!(reader_url(&v.parse().unwrap(), None));
        }
        for v in [
            "http://x.com/home",
            "https://x.com.evil.test/home",
            "https://user@x.com/home",
            "https://x.com:444/home",
            "https://x.com/messages",
            "https://api.x.com/home",
            "https://x.com/i/bookmarks",
            "https://x.com/another/likes",
        ] {
            assert!(!reader_url(&v.parse().unwrap(), None));
        }
    }
    #[test]
    fn private_routes_are_owner_and_feed_scoped() {
        assert!(feed_target("likes", "my_account", "/my_account/likes"));
        assert!(!feed_target("likes", "my_account", "/another/likes"));
        assert!(feed_target("communities", "my_account", "/i/communities"));
        assert!(feed_target(
            "communities",
            "my_account",
            "/my_account/communities"
        ));
        assert!(!feed_target(
            "communities",
            "my_account",
            "/other/communities"
        ));
        assert!(feed_target("bookmarks", "my_account", "/i/bookmarks"));
        assert!(!feed_target("bookmarks", "my_account", "/i/bookmarks/123"));
        assert!(!feed_target("unknown", "my_account", "/home"));
        assert!(reader_url(
            &"https://x.com/my_account/likes".parse().unwrap(),
            Some("/my_account/likes")
        ));
        for url in [
            "https://x.com/other/likes",
            "https://x.com/my_account/likes?redirect=evil",
            "https://x.com/my_account/likes#other",
            "https://x.com/i/bookmarks",
        ] {
            assert!(!reader_url(
                &url.parse().unwrap(),
                Some("/my_account/likes")
            ));
        }
    }
    #[test]
    fn no_arbitrary_image_urls_or_redirect_context() {
        assert_eq!(
            media_url("https://pbs.twimg.com/media/Fixture_1?format=jpg&name=orig")
                .unwrap()
                .as_str(),
            "https://pbs.twimg.com/media/Fixture_1?format=jpg&name=medium"
        );
        for v in [
            "https://pbs.twimg.com.evil.test/media/a",
            "https://user@pbs.twimg.com/media/a",
            "https://pbs.twimg.com/profile_images/a",
            "https://pbs.twimg.com/media/a.svg",
            "https://pbs.twimg.com/media/../secret",
            "https://pbs.twimg.com/media/a?access_token=secret",
            "http://pbs.twimg.com/media/a",
            "https://pbs.twimg.com:444/media/a",
        ] {
            assert!(media_url(v).is_err(), "{v}")
        }
    }
    #[test]
    fn projection_is_bounded_and_deduplicated() {
        let p = json!({"id":"123456789","handle":"fixture","name":"name","text":"text","time":"","media":[],"video":false,"truncated":false});
        assert_eq!(
            validated_posts(&json!([p.clone(), p.clone()]))
                .unwrap()
                .len(),
            1
        );
        let mut bad = p.clone();
        bad["id"] = json!("javascript:alert(1)");
        assert!(validated_posts(&json!([bad])).is_err());
        let mut bad = p.clone();
        bad["text"] = json!("a".repeat(12001));
        assert!(validated_posts(&json!([bad])).is_err());
        assert!(validated_posts(&json!(vec![p; 61])).is_err());
    }
    #[test]
    fn lifecycle_fences_cancel_and_generation() {
        let session = XSession::default();
        let cancel = AtomicBool::new(false);
        assert!(session.current(1, &cancel).is_ok());
        cancel.store(true, Ordering::Release);
        assert!(session.current(1, &cancel).is_err());
        assert!(!valid_handle("bad/name"));
        assert!(!valid_id("0123456"));
    }
    #[test]
    fn linkless_quote_preview_cannot_supply_an_identity_or_unvalidated_media() {
        let p=json!({"id":"123456789","handle":"fixture","name":"name","text":"text","time":"","media":[],"video":false,"truncated":false,
            "quotePreview":{"handle":"quoted","name":"Quoted","text":"visible text","time":"","media":["https://pbs.twimg.com/media/Test?format=jpg"],"avatar":null,"video":false}});
        assert!(validated_posts(&json!([p.clone()])).is_ok());
        let mut bad=p.clone();bad["quotePreview"]["id"]=json!("999999999");
        assert!(validated_posts(&json!([bad])).is_err());
        let mut bad=p.clone();bad["quotePreview"]["media"]=json!(["https://evil.test/media/Test"]);
        assert!(validated_posts(&json!([bad])).is_err());
        let mut bad=p;bad["quotePreview"]["handle"]=json!("../other");
        assert!(validated_posts(&json!([bad])).is_err());
    }
    #[test]
    fn projection_never_reads_credentials_or_uses_private_endpoints() {
        let source = include_str!("x_feed.js");
        for forbidden in [
            "document.cookie",
            "localStorage",
            "sessionStorage",
            "fetch(",
            "XMLHttpRequest",
            "graphql",
            "__INITIAL_STATE__",
        ] {
            assert!(!source.contains(forbidden));
        }
    }
}
