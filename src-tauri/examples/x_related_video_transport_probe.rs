//! Bounded, opt-in official-page observation. No Atsumi services or IPC handlers.
//! Reuses the active native profile without reading/copying its credentials.
//! This is NOT a playback, next-scroll, recommendation relevance, or UI test.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{Manager, Url};
#[path = "../src/x_source_link_parse.rs"]
mod source_link_parse;
use source_link_parse::Projection;
#[path = "../src/x_source_link_observer.rs"]
mod source_link_observer;

const LABEL: &str = "x-related-video-transport-probe";
const BROWSER_ARGS: &str = "--autoplay-policy=no-user-gesture-required --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows";
type Ledger = Arc<Mutex<ResponseLedger>>;

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponseLedger {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    public_assets: BTreeMap<String, u16>,
    // Retain the existing count/latest-status shape for prior report readers.
    operations: BTreeMap<&'static str, (u32, i32)>,
    operation_status_counts: BTreeMap<&'static str, BTreeMap<u16, u32>>,
}
impl ResponseLedger {
    fn observe(&mut self, url: &Url, status: i32) {
        let Ok(code) = u16::try_from(status) else {
            return;
        };
        if !(100..=599).contains(&code) {
            return;
        }
        if public_source_asset(url) && self.public_assets.len() < 96 {
            self.public_assets.insert(url.to_string(), code);
        }
        let Some(name) = operation(url) else { return };
        // Both maps have at most eight closed operation names. Each histogram
        // has at most 500 HTTP status keys; counters cannot wrap or grow text.
        let row = self.operations.entry(name).or_default();
        row.0 = row.0.saturating_add(1);
        row.1 = status;
        let count = self
            .operation_status_counts
            .entry(name)
            .or_default()
            .entry(code)
            .or_default();
        *count = count.saturating_add(1);
    }

    fn clear(&mut self) {
        self.public_assets.clear();
        self.operations.clear();
        self.operation_status_counts.clear();
    }
}
fn public_source_asset(url: &Url) -> bool {
    if url.scheme() != "https"
        || url.host_str() != Some("abs.twimg.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let name = url
        .path()
        .strip_prefix("/responsive-web/client-web/")
        .or_else(|| {
            url.path()
                .strip_prefix("/responsive-web/client-web-legacy/")
        });
    name.is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 460
            && name.ends_with(".js")
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"~._-".contains(&c))
    })
}

fn official(url: &Url) -> bool {
    url.scheme() == "https"
        && matches!(url.host_str(), Some("x.com" | "www.x.com"))
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}
fn valid_seed(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 20
        && !value.starts_with('0')
        && value.bytes().all(|b| b.is_ascii_digit())
}
fn pinned_seed(url: &Url) -> Option<&str> {
    if !official(url) || url.path() != "/i/immersive_timeline" || url.fragment().is_some() {
        return None;
    }
    let seed = url.query()?.strip_prefix("pinned_tweet_id=")?;
    valid_seed(seed).then_some(seed)
}
fn source_video_seed(path: &str) -> bool {
    path.strip_prefix("/NASA/status/")
        .is_some_and(|id| valid_seed(id) && id.len() >= 5)
}
fn source_viewer(url: &Url) -> bool {
    official(url)
        && url.query().is_none()
        && url.fragment().is_none()
        && viewer_seed(url).is_some_and(source_video_seed)
}
fn viewer_seed(url: &Url) -> Option<&str> {
    url.path()
        .strip_suffix("/mediaviewer")
        .or_else(|| url.path().strip_suffix("/mediaViewer"))
}
fn public_player_script(value: &str) -> bool {
    let Some(name) = value.strip_prefix("https://abs.twimg.com/responsive-web/client-web/") else {
        return false;
    };
    value.len() <= 384
        && ["bundle.", "loader.", "ondemand."]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        && name.ends_with(".js")
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"~._-".contains(&c))
        && ["media", "video", "immersive"]
            .iter()
            .any(|part| name.to_ascii_lowercase().contains(part))
}
fn route(url: &Url) -> &'static str {
    if url.as_str() == "about:blank" {
        return "blank";
    }
    if !official(url) {
        return "foreign";
    }
    if url.path() == "/login"
        || url.path().starts_with("/i/flow/")
        || url.path().starts_with("/account/")
    {
        return "login";
    }
    if url.fragment().is_some() || url.query().is_some() && pinned_seed(url).is_none() {
        return "other";
    }
    match url.path() {
        "/home" => "home",
        "/NASA" => "neutral_profile",
        "/i/immersive_timeline" => "immersive",
        _ if source_viewer(url) => "source_viewer",
        _ if source_video_seed(url.path()) && url.query().is_none() && url.fragment().is_none() => {
            "neutral_post"
        }
        _ => "other",
    }
}
fn operation(url: &Url) -> Option<&'static str> {
    if !official(url) {
        return None;
    }
    let pieces: Vec<_> = url.path().trim_start_matches('/').split('/').collect();
    if pieces.len() != 5 || pieces[..3] != ["i", "api", "graphql"] {
        return None;
    }
    match pieces[4] {
        "HomeTimeline" => Some("HomeTimeline"),
        "HomeLatestTimeline" => Some("HomeLatestTimeline"),
        "ImmersiveViewerExploreMixerTimeline" => Some("ImmersiveViewerExploreMixerTimeline"),
        "ImmersiveViewerProfileMixerTimeline" => Some("ImmersiveViewerProfileMixerTimeline"),
        "ExplorePage" => Some("ExplorePage"),
        "TweetDetail" => Some("TweetDetail"),
        "ImmersiveMedia" => Some("ImmersiveMedia"),
        "ImmersiveProfile" => Some("ImmersiveProfile"),
        _ => None,
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Account {
    id: String,
    handle: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    active: String,
    accounts: Vec<Account>,
}
fn valid_profile(id: &str) -> bool {
    id == "legacy"
        || id.len() == 32
            && id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn valid_handle(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 15
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn active_profile(app: &tauri::AppHandle) -> Result<(std::path::PathBuf, String), &'static str> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|_| "app_directory_unavailable")?;
    let file = root.join("x-accounts/accounts.json");
    if std::fs::metadata(&file)
        .map_err(|_| "registry_unavailable")?
        .len()
        > 16384
    {
        return Err("registry_invalid");
    }
    let data: Registry =
        serde_json::from_slice(&std::fs::read(file).map_err(|_| "registry_unavailable")?)
            .map_err(|_| "registry_invalid")?;
    if !valid_profile(&data.active)
        || data.accounts.len() > 20
        || data
            .accounts
            .iter()
            .any(|a| !valid_profile(&a.id) || !valid_handle(&a.handle))
        || data
            .accounts
            .iter()
            .map(|a| &a.id)
            .collect::<std::collections::HashSet<_>>()
            .len()
            != data.accounts.len()
    {
        return Err("registry_invalid");
    }
    let owner = data
        .accounts
        .iter()
        .find(|a| a.id == data.active)
        .ok_or("active_owner_unavailable")?
        .handle
        .to_ascii_lowercase();
    let profile = if data.active == "legacy" {
        root.join("youtube-original-profile")
    } else {
        root.join("x-accounts/profiles").join(&data.active)
    };
    if !profile.is_dir() {
        return Err("existing_profile_unavailable");
    }
    Ok((profile, owner))
}
const DOM: &str = r#"(()=>{
  const expected=__OWNER__;
  const seedMode=__SEEDED__;
  const sourceViewer=__MEDIAVIEWER__;
  if(location.href==='about:blank')return {state:'loading',route:''};
  if(location.protocol!=='https:'||!['x.com','www.x.com'].includes(location.hostname)||location.port||location.username||location.password)return {state:'foreign'};
  const path=location.pathname;
  const route=path==='/home'?'home':path==='/NASA'?'neutral_profile':path==='/i/immersive_timeline'?'immersive':/^\/NASA\/status\/[1-9][0-9]{4,19}\/(?:mediaviewer|mediaViewer)$/.test(path)?'source_viewer':/^\/NASA\/status\/[1-9][0-9]{4,19}$/.test(path)?'neutral_post':'other';
  const profile=document.querySelector('a[data-testid="AppTabBar_Profile_Link"]');
  let owner=null;try{const href=profile?.getAttribute('href');if(href){const u=new URL(href,location.href);if(u.origin===location.origin)owner=/^\/([A-Za-z0-9_]{1,15})\/?$/.exec(u.pathname)?.[1]?.toLowerCase()||null;}}catch{}
  const account=!!document.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]');
  const challenge=!!document.querySelector('iframe[src*="arkoselabs"],iframe[src*="captcha"],iframe[src*="challenges.cloudflare"],input[name="cf-turnstile-response"],#challenge-running');
  const login=path==='/login'||path.startsWith('/i/flow/')||path.startsWith('/account/')||(!owner&&!!document.querySelector('input[type="password"],a[href="/login"],a[href="/i/flow/login"]'));
  const ownerMatch=owner===expected&&account;
  const presented=node=>!!node?.getClientRects().length&&getComputedStyle(node).display!=='none'&&getComputedStyle(node).visibility!=='hidden'&&!node.closest('[hidden],[aria-hidden="true"],[inert]');
  const layers=[...document.querySelectorAll('[role="dialog"],[aria-modal="true"]')].slice(0,16).filter(presented);
  const sourceContainer=route==='source_viewer'?document.querySelector('[data-testid="modalContainer"]'):null;
  const topLayer=layers.at(-1)||(presented(sourceContainer)?sourceContainer:null),errorRoot=topLayer||document.querySelector('main[role="main"]');
  const missing=new Set(['이 페이지는 존재하지 않습니다. 다른 페이지를 검색해 보세요.',"Hmm...this page doesn’t exist. Try searching for something else.","Hmm...this page doesn't exist. Try searching for something else."]);
  const sourceRouteUnavailable=!!errorRoot&&[...errorRoot.querySelectorAll('span,p,h1,[role="alert"]')].slice(0,256).some(n=>!n.closest('article,[data-testid="tweet"]')&&presented(n)&&missing.has((n.textContent||'').replace(/\s+/g,' ').trim()));
  const topVideos=topLayer?[...topLayer.querySelectorAll('video')].slice(0,8).filter(presented):[];
  const topLayerVideos=topVideos.length,topLayerReadyVideos=topVideos.filter(v=>v.readyState>=2&&v.videoWidth>0&&v.videoHeight>0).length;
  const topLayerNativePlayers=topLayer?[...topLayer.querySelectorAll('[data-testid="videoPlayer"]')].slice(0,8).filter(presented).length:0;
  const sourceViewerAvailable=window.__atsumiXSurface?.sourceMediaViewerAvailable?.()===true;
  const alerts=[...document.querySelectorAll('[role="alert"],[data-testid="error-detail"],[data-testid="error-message"]')].slice(0,8);
  const sourceError=sourceRouteUnavailable||alerts.some(n=>/something went wrong|try again|page doesn.t exist|문제가 발생|다시 시도|페이지가 존재/.test((n.textContent||'').toLowerCase()));
  const primary=!!document.querySelector('[data-testid="primaryColumn"]');
  let immersiveLinks=0,seededImmersiveLinks=0,postVideoLinks=0;const videoRoutes=new Set();
  for(const a of [...document.querySelectorAll('a[href]')].slice(0,2000)){
    try{const u=new URL(a.getAttribute('href'),location.href);if(u.protocol!=='https:'||!['x.com','www.x.com'].includes(u.hostname)||u.port||u.username||u.password)continue;
      if(u.pathname==='/i/immersive_timeline'){immersiveLinks++;if(u.searchParams.has('pinned_tweet_id'))seededImmersiveLinks++;videoRoutes.add('/i/immersive_timeline');}
      if(u.pathname==='/i/videos'||u.pathname==='/i/video')videoRoutes.add(u.pathname);
      if(/^\/[A-Za-z0-9_]{1,15}\/status\/[0-9]{1,20}\/video\/[1-4]$/.test(u.pathname)){postVideoLinks++;videoRoutes.add('/:handle/status/:id/video/:ordinal');}
    }catch{}
  }
  let seedId=null,seedPath=null;
  if(seedMode&&ownerMatch&&!challenge&&!login&&route===(sourceViewer?'neutral_profile':'home')){
    for(const article of [...document.querySelectorAll('article[data-testid="tweet"]')].slice(0,100)){
      if(![...article.querySelectorAll('video')].some(v=>v.closest('article')===article&&!v.closest('[data-testid="quoteTweet"],[data-testid="quotedTweet"]')))continue;
      const candidates=new Set();
      for(const time of article.querySelectorAll('time')){
        if(time.closest('article')!==article||time.closest('[data-testid="quoteTweet"],[data-testid="quotedTweet"]'))continue;
        const link=time.closest('a[href]');if(!link)continue;
        try{const u=new URL(link.getAttribute('href'),location.href);if(u.protocol!=='https:'||!['x.com','www.x.com'].includes(u.hostname)||u.port||u.username||u.password||u.search||u.hash)continue;
          const id=(sourceViewer?/^\/NASA\/status\/([1-9][0-9]{4,19})\/?$/:/^\/[A-Za-z0-9_]{1,15}\/status\/([1-9][0-9]{0,19})\/?$/).exec(u.pathname)?.[1];if(id)candidates.add(id);
        }catch{}
      }
      // An incomplete or ambiguous first video does not rule out a later
      // proved video inside this same bounded document scan.
      if(candidates.size===1){if(sourceViewer)seedPath='/NASA/status/'+[...candidates][0];else seedId=[...candidates][0];break;}
    }
  }
  const publicPlayerScripts=sourceViewer&&typeof performance.getEntriesByType==='function'?[...new Set(performance.getEntriesByType('resource').slice(0,500).map(e=>e.name).filter(v=>typeof v==='string'&&v.length<=384&&/^https:\/\/abs\.twimg\.com\/responsive-web\/client-web\/(?:bundle|loader|ondemand)\.[A-Za-z0-9~._-]+\.js$/.test(v)&&/media|video|immersive/i.test(v.split('/').at(-1))))].slice(0,8):[];
  return {state:challenge?'challenge':login?'login':owner&&owner!==expected?'owner_mismatch':sourceError?'source_error':ownerMatch?'owner_verified':'loading',route,ownerMatch,primary,articles:Math.min(document.querySelectorAll('article[data-testid="tweet"]').length,1000),videos:Math.min(document.querySelectorAll('video').length,1000),dialogs:Math.min(document.querySelectorAll('[role="dialog"]').length,100),loadingIndicators:Math.min(document.querySelectorAll('[role="progressbar"]').length,100),immersiveLinks,seededImmersiveLinks,postVideoLinks,videoRoutes:[...videoRoutes],seedId,seedPath,publicPlayerScripts,sourceViewerAvailable,sourceRouteUnavailable,topLayerVideos,topLayerReadyVideos,topLayerNativePlayers};
})()"#;
#[derive(Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct Snapshot {
    state: String,
    route: String,
    owner_match: bool,
    primary: bool,
    articles: u32,
    videos: u32,
    dialogs: u32,
    loading_indicators: u32,
    immersive_links: u32,
    seeded_immersive_links: u32,
    post_video_links: u32,
    video_routes: Vec<String>,
    public_player_scripts: Vec<String>,
    source_viewer_available: bool,
    source_route_unavailable: bool,
    top_layer_videos: u32,
    top_layer_ready_videos: u32,
    top_layer_native_players: u32,
    // Only current memory may carry the observed ID. It never enters JSON evidence.
    #[serde(skip_serializing)]
    seed_id: Option<String>,
    #[serde(skip_serializing)]
    seed_path: Option<String>,
}
fn read_dom(view: &tauri::Webview, script: &str) -> Option<Snapshot> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(script, move |raw| {
        let _ = tx.try_send(raw);
    })
    .ok()?;
    let raw = rx.recv_timeout(Duration::from_secs(2)).ok()?;
    if raw.len() > 4096 {
        return None;
    }
    let sample: Snapshot = serde_json::from_str(&raw).ok()?;
    if !matches!(
        sample.state.as_str(),
        "foreign"
            | "challenge"
            | "login"
            | "owner_mismatch"
            | "source_error"
            | "owner_verified"
            | "loading"
    ) || !matches!(
        sample.route.as_str(),
        "" | "home" | "immersive" | "neutral_profile" | "neutral_post" | "source_viewer" | "other"
    ) || sample.video_routes.len() > 4
        || sample.video_routes.iter().any(|route| {
            !matches!(
                route.as_str(),
                "/i/immersive_timeline"
                    | "/i/videos"
                    | "/i/video"
                    | "/:handle/status/:id/video/:ordinal"
            )
        })
        || sample.seed_id.as_deref().is_some_and(|id| !valid_seed(id))
        || sample
            .seed_path
            .as_deref()
            .is_some_and(|path| !source_video_seed(path))
        || sample.public_player_scripts.len() > 8
        || sample
            .public_player_scripts
            .iter()
            .any(|url| !public_player_script(url))
    {
        return None;
    }
    Some(sample)
}
#[cfg(windows)]
fn install_observer(view: &tauri::Webview, ledger: Ledger) -> Result<(), &'static str> {
    use webview2_com::{
        CoTaskMemPWSTR,
        Microsoft::Web::WebView2::Win32::{ICoreWebView2_2, ICoreWebView2_8},
        WebResourceResponseReceivedEventHandler,
    };
    use windows::core::{Interface, PWSTR};
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let result = (|| -> windows::core::Result<()> {
            let core = platform.controller().CoreWebView2()?;
            core.cast::<ICoreWebView2_8>()?.SetIsMuted(true)?;
            let core = core.cast::<ICoreWebView2_2>()?;
            let mut token = 0;
            core.add_WebResourceResponseReceived(
                &WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
                    // URL is transiently inspected; only a closed operation enum is retained.
                    // No request/response headers, contents or complete URL enter evidence.
                    if let Some(args) = args {
                        let request = args.Request()?;
                        let mut uri = PWSTR::null();
                        request.Uri(&mut uri)?;
                        if let Ok(url) = Url::parse(&CoTaskMemPWSTR::from(uri).to_string()) {
                            if operation(&url).is_some() || public_source_asset(&url) {
                                let mut status = 0;
                                args.Response()?.StatusCode(&mut status)?;
                                let mut rows = ledger.lock().unwrap_or_else(|p| p.into_inner());
                                rows.observe(&url, status);
                            }
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            Ok(())
        })();
        let _ = tx.try_send(result.is_ok());
    })
    .map_err(|_| "observer_dispatch_failed")?;
    if rx.recv_timeout(Duration::from_secs(3)).unwrap_or(false) {
        Ok(())
    } else {
        Err("observer_install_failed")
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows-only diagnostic");
}
#[cfg(windows)]
fn main() {
    if std::env::var("ATSUMI_RUN_X_RELATED_TRANSPORT_PROBE").as_deref() != Ok("1") {
        eprintln!("Opt-in required; no browser or session opened.");
        return;
    }
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let runtime = tauri::Builder::default().setup(|app| {
        let (profile, owner) = active_profile(app.handle()).map_err(std::io::Error::other)?;
        let source_viewer_mode = std::env::var("ATSUMI_X_RELATED_PROBE_MEDIAVIEWER").as_deref() == Ok("1");
        let thread_mode=std::env::var("ATSUMI_X_RELATED_PROBE_THREAD_LINKS").as_deref()==Ok("1");
        let neutral_mode=source_viewer_mode||thread_mode;
        let source_adapter=std::env::var("ATSUMI_X_RELATED_PROBE_SOURCE_ADAPTER").as_deref()==Ok("1");
        let seeded = neutral_mode || std::env::var("ATSUMI_X_RELATED_PROBE_SEEDED").as_deref() == Ok("1");
        let adapter_config=serde_json::json!({"nonce":"bounded-neutral-probe","feed":"foryou"}).to_string();
        let adapter_script=if source_adapter{[include_str!("../src/x_surface_route_adapter.js").replace("__ATSUMI_X_SOURCE_COMPAT__",&adapter_config),include_str!("../src/x_surface_source_delivery.js").replace("__ATSUMI_X_SOURCE_COMPAT__",&adapter_config),r#"(()=>{let available=false;window.__atsumiXSurface=Object.freeze({registerSourceMediaViewer(nonce,value=true){if(nonce==='bounded-neutral-probe')available=value===true;},sourceMediaViewerAvailable:()=>available});})();"#.into()].join("\n")}else{String::new()};
        let expected_seed = Arc::new(Mutex::new(None::<String>)); let navigation_seed = expected_seed.clone();
        let blocked = Arc::new(Mutex::new(None::<&'static str>));
        let nav_blocked = blocked.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let window = tauri::WebviewWindowBuilder::new(app, LABEL, tauri::WebviewUrl::External("about:blank".parse()?))
            .title("Atsumi X transport diagnostic").visible(false).focused(false).inner_size(if source_adapter {560.0} else {1100.0}, 800.0)
            .data_directory(profile).browser_extensions_enabled(false).additional_browser_args(BROWSER_ARGS)
            .initialization_script(adapter_script)
            .on_navigation(move |url| {
                let value = route(url);
                let source_allowed = neutral_mode && (value=="neutral_profile" || source_viewer_mode && value=="source_viewer" && viewer_seed(url).is_some_and(|path|navigation_seed.lock().unwrap_or_else(|p|p.into_inner()).as_deref()==Some(path)) || thread_mode && value=="neutral_post" && navigation_seed.lock().unwrap_or_else(|p|p.into_inner()).as_deref()==Some(url.path()));
                if source_allowed || matches!(value, "blank"|"home"|"immersive") && !neutral_mode && (url.query().is_none() || seeded && pinned_seed(url).is_some_and(|id| navigation_seed.lock().unwrap_or_else(|p|p.into_inner()).as_deref()==Some(id))) || value=="blank" { true }
                else { *nav_blocked.lock().unwrap_or_else(|p| p.into_inner()) = Some(value); false }
            })
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny).on_download(|_, _| false).build()?;
        let close_flag = closed.clone();
        window.on_window_event(move |event| { if matches!(event, tauri::WindowEvent::Destroyed) { close_flag.store(true, Ordering::Release); } });
        let handle = app.handle().clone(); let watchdog = handle.clone(); let watchdog_done = closed.clone();
        std::thread::spawn(move || { std::thread::sleep(Duration::from_secs(55)); if !watchdog_done.load(Ordering::Acquire) { println!("{{\"result\":\"watchdog_timeout\"}}"); watchdog.exit(2); } });
        std::thread::spawn(move || {
            let result = (|| -> Result<serde_json::Value, &'static str> {
                let view = handle.get_webview(LABEL).ok_or("probe_view_missing")?;
                let ledger: Ledger = Arc::default(); install_observer(&view, ledger.clone())?;
                let script = DOM.replace("__OWNER__", &serde_json::to_string(&owner).map_err(|_| "owner_encoding_failed")?).replace("__SEEDED__", if seeded {"true"} else {"false"}).replace("__MEDIAVIEWER__",if neutral_mode {"true"} else {"false"});
                let mut phases = Vec::new(); let mut home_seed: Option<String> = None;
                for phase in ["home", "immersive"] {
                    let expected_phase = if neutral_mode {if phase=="home" {"neutral_profile"} else if thread_mode {"neutral_post"} else {"source_viewer"}} else {phase};
                    let path = if phase == "home" {if neutral_mode {"/NASA"} else {"/home"}.to_string()} else if neutral_mode {
                        let seed=home_seed.take().ok_or("observed_neutral_video_unavailable")?;
                        if !source_video_seed(&seed) {return Err("observed_neutral_video_invalid");}
                        let path=if thread_mode{seed.clone()}else{format!("{seed}/mediaviewer")};
                        *expected_seed.lock().unwrap_or_else(|p|p.into_inner())=Some(seed);path
                    } else if seeded {
                        let seed=home_seed.take().ok_or("observed_video_seed_unavailable")?;
                        if !valid_seed(&seed) {return Err("observed_video_seed_invalid");}
                        let path=format!("/i/immersive_timeline?pinned_tweet_id={seed}");
                        *expected_seed.lock().unwrap_or_else(|p|p.into_inner())=Some(seed);path
                    } else {"/i/immersive_timeline".to_string()};
                    ledger.lock().unwrap_or_else(|p| p.into_inner()).clear();
                    let post_payload:Arc<Mutex<Option<Projection>>>=Arc::default();let post_stop=Arc::new(AtomicBool::new(false));
                    let post_observer=if thread_mode&&phase!="home"{Some(source_link_observer::observe(&view,format!("https://x.com{path}").parse().unwrap(),post_stop.clone(),post_payload.clone()).map_err(|_|"post_observer_failed")?)}else{None};
                    view.navigate(format!("https://x.com{path}").parse().unwrap()).map_err(|_| "navigation_failed")?;
                    let start = std::time::Instant::now(); let mut samples = Vec::new(); let mut matched = false;
                    while start.elapsed() < Duration::from_secs(20) {
                        if let Some(class) = *blocked.lock().unwrap_or_else(|p| p.into_inner()) { return Err(match class { "login" => "login_required", "foreign" => "foreign_navigation_blocked", _ => "unsupported_navigation_blocked" }); }
                        if closed.load(Ordering::Acquire) { return Err("probe_closed"); }
                        std::thread::sleep(Duration::from_millis(500));
                        let Some(mut sample) = read_dom(&view, &script) else { continue; };
                        if matches!(sample.state.as_str(), "challenge"|"login"|"owner_mismatch"|"foreign") { return Err(match sample.state.as_str() { "challenge"=>"human_verification_required", "login"=>"login_required", "owner_mismatch"=>"owner_mismatch", _=>"foreign_document" }); }
                        // A direct source viewer can omit account navigation.
                        // Initial owner proof is required before choosing its
                        // neutral public path; counts are not authentication/UI proof.
                        let ready = sample.route == expected_phase && (sample.owner_match || source_viewer_mode && phase!="home");
                        if phase == "home" && ready && home_seed.is_none() {home_seed=if neutral_mode {sample.seed_path.take()} else {sample.seed_id.take()};}
                        sample.seed_id=None;
                        sample.seed_path=None;
                        let source_error = sample.state == "source_error";
                        matched |= ready;
                        samples.push(sample);
                        // Give ordinary home anchors time to hydrate; never click or scroll them.
                        if phase == "home" && ready && (!seeded || home_seed.is_some()) && start.elapsed() >= Duration::from_secs(6) || phase == "immersive" && (source_error || thread_mode&&ready&&post_payload.lock().unwrap_or_else(|p|p.into_inner()).is_some()&&start.elapsed()>=Duration::from_secs(6) || start.elapsed() >= Duration::from_secs(if seeded {20} else {12})) { break; }
                    }
                    let latest = samples.last().map(|s| s.route.as_str()).unwrap_or("unknown");
                    let outcome = if !matched { if latest == "home" && phase == "immersive" { "redirected_home" } else { "unverified" } } else if phase == "home" { "owner_verified" } else { "route_observed_not_ui_verified" };
                    let observed = ledger.lock().unwrap_or_else(|p| p.into_inner());
                    let projected=post_payload.lock().unwrap_or_else(|p|p.into_inner());let post_result=post_observer.map(|observer|serde_json::json!({"stats":*observer.lock().unwrap_or_else(|p|p.into_inner()),"matchedRoot":projected.is_some(),"links":projected.as_ref().map(|p|p.links.len()),"limited":projected.as_ref().map(|p|p.limited)}));post_stop.store(true,Ordering::Release);
                    phases.push(serde_json::json!({"phase":expected_phase,"outcome":outcome,"elapsedMs":start.elapsed().as_millis(),"samples":samples,"operations":observed.operations,"operationStatusCounts":observed.operation_status_counts,"publicAssets":observed.public_assets,"publishedLinks":post_result}));
                    if phase == "home" && !matched { break; }
                }
                Ok(serde_json::json!({"scope":"read_only_transport_only","seed":if source_viewer_mode {"observed_neutral_profile_video_path_not_logged"} else if seeded {"observed_home_video_id_not_logged"} else {"none"},"phases":phases,"uiVerified":false,"relatednessVerified":false,"nextScrollVerified":false}))
            })();
            let exit_code = if result.is_ok() { 0 } else { 2 };
            match result { Ok(report)=>println!("{}",report), Err(code)=>println!("{}",serde_json::json!({"scope":"read_only_transport_only","error":code,"uiVerified":false})) }
            closed.store(true, Ordering::Release); handle.exit(exit_code);
        });
        Ok(())
    }).build(context);
    match runtime {
        Ok(app) => {
            let code = app.run_return(|_, _| {});
            std::process::exit(code);
        }
        Err(_) => {
            println!("{{\"error\":\"probe_setup_failed\"}}");
            std::process::exit(2);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    // Evaluates the exact probe DOM observer against synthetic documents only.
    // jsdom does not load resources and no browser, profile or network is used.
    const DOM_CASES: &str = r#"
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {JSDOM} from 'jsdom';
const source=readFileSync(0,'utf8');
const stamp=id=>`<a href="/author/status/${id}"><time></time></a>`;
const article=body=>`<article data-testid="tweet">${body}</article>`;
const video=id=>article(`<video></video>${stamp(id)}`);
let checked=0;
function inspect(body,{owner='owner',seeded=true,sourceViewer=false}={}){
  const dom=new JSDOM(`<a data-testid="AppTabBar_Profile_Link" href="/${owner}"></a><button data-testid="SideNav_AccountSwitcher_Button"></button><main data-testid="primaryColumn">${body}</main>`,{url:sourceViewer?'https://x.com/NASA':'https://x.com/home',runScripts:'outside-only'});
  try{return dom.window.eval(source.replace('__OWNER__',JSON.stringify('owner')).replace('__SEEDED__',String(seeded)).replace('__MEDIAVIEWER__',String(sourceViewer)));}
  finally{dom.window.close();}
}
function seed(body,expected,options){assert.equal(inspect(body,options).seedId,expected);checked++;}
seed(article('<video></video>')+video('22'),'22');
seed(article(`<video></video>${stamp('11')}${stamp('12')}`)+video('22'),'22');
seed(article(`<div data-testid="quoteTweet"><video></video>${stamp('11')}</div>`)+video('22'),'22');
seed(article(`<video></video><div data-testid="quotedTweet">${stamp('11')}</div>`)+video('22'),'22');
seed(article(`<video></video>${stamp('22')}${stamp('22')}`),'22');
seed(article(`<video></video>${stamp('11')}${stamp('12')}`),null);
for(const href of ['https://evil.test/author/status/11','https://x.com:444/author/status/11','https://user:secret@x.com/author/status/11','/author/status/11?other=1','/author/status/11#other','/author/status/01','/author/status/123456789012345678901']){
  seed(article(`<video></video><a href="${href}"><time></time></a>`)+video('22'),'22');
}
seed(Array.from({length:100},()=>article('<video></video>')).join('')+video('22'),null);
seed(video('22'),null,{owner:'different'});
seed(video('22'),null,{seeded:false});
seed('<iframe src="https://captcha.invalid/"></iframe>'+video('22'),null);
assert.equal(inspect(video('123456'),{sourceViewer:true}).seedPath,null);checked++;
const neutral=article('<video></video><a href="/NASA/status/123456"><time></time></a>');
assert.equal(inspect(neutral,{sourceViewer:true}).seedPath,'/NASA/status/123456');checked++;
console.log(`offline DOM cases: ${checked}`);
"#;

    #[test]
    fn dom_seed_skips_ambiguous_articles_without_guessing_or_crossing_scope() {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };
        let mut child = Command::new("node")
            .args(["--input-type=module", "-e", DOM_CASES])
            .current_dir(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("offline DOM tests require the project's Node.js and jsdom dependencies");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(DOM.as_bytes())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "offline DOM regression failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn response_status_history_keeps_failures_and_legacy_latest_shape() {
        let url = "https://x.com/i/api/graphql/fixture/HomeTimeline?variables=PRIVATE"
            .parse()
            .unwrap();
        let mut ledger = ResponseLedger::default();
        for status in [403, 200, 429, 200] {
            ledger.observe(&url, status);
        }
        let evidence = serde_json::to_value(&ledger).unwrap();
        assert_eq!(
            evidence,
            serde_json::json!({
                "operations": {"HomeTimeline": [4, 200]},
                "operationStatusCounts": {"HomeTimeline": {"200": 2, "403": 1, "429": 1}}
            })
        );
        let raw = evidence.to_string();
        for forbidden in [
            "PRIVATE",
            "variables",
            "fixture",
            "x.com",
            "headers",
            "cookies",
        ] {
            assert!(!raw.contains(forbidden));
        }
        ledger.clear();
        assert_eq!(
            serde_json::to_value(&ledger).unwrap(),
            serde_json::json!({"operations":{},"operationStatusCounts":{}})
        );
    }

    #[test]
    fn response_status_history_rejects_unbounded_keys_and_saturates_counts() {
        let mut ledger = ResponseLedger::default();
        let url = "https://x.com/i/api/graphql/fixture/HomeTimeline"
            .parse()
            .unwrap();
        for status in [-1, 0, 99, 600, 65536, i32::MAX] {
            ledger.observe(&url, status);
        }
        for path in [
            "https://evil.test/i/api/graphql/fixture/HomeTimeline",
            "https://x.com/i/api/graphql/fixture/UnknownOperation",
        ] {
            ledger.observe(&path.parse().unwrap(), 200);
        }
        assert!(ledger.operations.is_empty());
        assert!(ledger.operation_status_counts.is_empty());
        ledger.operations.insert("HomeTimeline", (u32::MAX, 403));
        ledger
            .operation_status_counts
            .entry("HomeTimeline")
            .or_default()
            .insert(200, u32::MAX);
        ledger.observe(&url, 200);
        assert_eq!(ledger.operations["HomeTimeline"], (u32::MAX, 200));
        assert_eq!(
            ledger.operation_status_counts["HomeTimeline"][&200],
            u32::MAX
        );
        for name in [
            "HomeTimeline",
            "HomeLatestTimeline",
            "ImmersiveViewerExploreMixerTimeline",
            "ImmersiveViewerProfileMixerTimeline",
            "ExplorePage",
            "TweetDetail",
        ] {
            let url = format!("https://x.com/i/api/graphql/fixture/{name}")
                .parse()
                .unwrap();
            for code in 100..=599 {
                ledger.observe(&url, code);
            }
        }
        assert_eq!(ledger.operations.len(), 6);
        assert_eq!(ledger.operation_status_counts.len(), 6);
        assert!(ledger
            .operation_status_counts
            .values()
            .all(|counts| counts.len() == 500));
        assert!(serde_json::to_string(&ledger).unwrap().len() < 65536);
    }

    #[test]
    fn routes_are_closed_and_auth_never_automated() {
        assert!(public_player_script(
            "https://abs.twimg.com/responsive-web/client-web/bundle.TweetMediaDetail.fixture.js"
        ));
        for url in [
            "https://abs.twimg.com.evil.test/responsive-web/client-web/bundle.Video.fixture.js",
            "https://abs.twimg.com/responsive-web/client-web/bundle.Video.fixture.js?secret=1",
            "https://x.com/i/api/graphql/private/Video",
        ] {
            assert!(!public_player_script(url));
        }
        assert_eq!(
            route(
                &"https://x.com/NASA/status/123456/mediaviewer"
                    .parse()
                    .unwrap()
            ),
            "source_viewer"
        );
        assert_eq!(
            route(
                &"https://x.com/NASA/status/123456/mediaViewer"
                    .parse()
                    .unwrap()
            ),
            "source_viewer"
        );
        for raw in [
            "https://x.com/other/status/123456/mediaviewer",
            "https://x.com/NASA/status/123456/mediaviewer?q=extra",
            "https://x.com/NASA/status/001234/mediaviewer",
        ] {
            assert!(!source_viewer(&raw.parse().unwrap()));
        }
        assert_eq!(route(&"https://x.com/home".parse().unwrap()), "home");
        assert_eq!(
            route(&"https://x.com/i/immersive_timeline".parse().unwrap()),
            "immersive"
        );
        for value in [
            "http://x.com/home",
            "https://x.com.evil.test/home",
            "https://user:secret@x.com/home",
            "https://x.com:444/home",
        ] {
            assert_eq!(route(&value.parse().unwrap()), "foreign");
        }
        assert_eq!(
            route(&"https://x.com/i/flow/login".parse().unwrap()),
            "login"
        );
        assert_eq!(
            route(
                &"https://x.com/i/immersive_timeline?pinned_tweet_id=123"
                    .parse()
                    .unwrap()
            ),
            "immersive"
        );
    }
    #[test]
    fn seeded_route_accepts_exactly_one_literal_numeric_parameter() {
        for tail in [
            "",
            "0",
            "01",
            "1&other=2",
            "1&pinned_tweet_id=2",
            "%31",
            "+1",
            "-1",
            "1.0",
            "123456789012345678901",
        ] {
            assert_eq!(
                route(
                    &format!("https://x.com/i/immersive_timeline?pinned_tweet_id={tail}")
                        .parse()
                        .unwrap()
                ),
                "other"
            );
        }
        assert_eq!(
            route(&"https://x.com/home?pinned_tweet_id=123".parse().unwrap()),
            "other"
        );
        assert_eq!(
            route(
                &"https://x.com/i/immersive_timeline?pinned_tweet_id=123#other"
                    .parse()
                    .unwrap()
            ),
            "other"
        );
    }
    #[test]
    fn observed_seed_is_never_serialized_into_evidence() {
        let sample = Snapshot {
            seed_id: Some("1234567890123456789".into()),
            state: "owner_verified".into(),
            ..Default::default()
        };
        let evidence = serde_json::to_string(&sample).unwrap();
        assert!(!evidence.contains("1234567890123456789"));
        assert!(!evidence.contains("seedId"));
        assert!(!evidence.contains("seed_id"));
    }
    #[test]
    fn network_projection_never_returns_arbitrary_paths_or_queries() {
        assert_eq!(operation(&"https://x.com/i/api/graphql/fixture/ImmersiveViewerExploreMixerTimeline?variables=SECRET".parse().unwrap()), Some("ImmersiveViewerExploreMixerTimeline"));
        assert_eq!(
            operation(
                &"https://x.com/i/api/graphql/fixture/SecretAccountMutation"
                    .parse()
                    .unwrap()
            ),
            None
        );
        assert_eq!(
            operation(
                &"https://evil.test/i/api/graphql/fixture/HomeTimeline"
                    .parse()
                    .unwrap()
            ),
            None
        );
    }
    #[test]
    fn profile_and_owner_are_closed_identifiers() {
        assert!(valid_profile("legacy"));
        assert!(valid_profile("0123456789abcdef0123456789abcdef"));
        for value in [
            "../Cookies",
            "C:/profile",
            "",
            "ABCDEF0123456789ABCDEF0123456789",
        ] {
            assert!(!valid_profile(value));
        }
        for value in ["", "owner/../../", "a@b", "sixteenletters16"] {
            assert!(!valid_handle(value));
        }
        assert!(valid_handle("owner_1"));
    }
    #[test]
    fn dom_observer_has_no_request_input_or_credential_access() {
        for forbidden in [
            "fetch(",
            "XMLHttpRequest",
            "document.cookie",
            "localStorage",
            "sessionStorage",
            ".click(",
            ".play(",
            "dispatchEvent",
            "window.open",
            "scrollTo(",
        ] {
            assert!(!DOM.contains(forbidden), "unexpected effect: {forbidden}");
        }
        assert!(DOM.contains("ownerMatch"));
        assert!(DOM.contains("Math.min"));
        assert!(!DOM.contains("innerHTML"));
    }
}
