//! Explicit, bounded profile-declared link evidence; never image authorship inference.
use super::*;
use crate::x_feed::block_reader_media;
use serde_json::json;
use std::collections::HashSet;
use std::sync::atomic::AtomicU64;

const LOOKUP_ERROR: &str =
    "X 프로필의 등록 링크를 확인하지 못했습니다. 프로필 페이지에서 확인해 주세요.";
const LOOKUP_TTL: Duration = Duration::from_secs(18);
const ADMISSION_TTL: Duration = Duration::from_secs(6);
const CACHE_TTL: Duration = Duration::from_secs(600);
const LOOKUP_BUSY: &str = "이전 프로필 확인을 마무리하고 있습니다. 잠시 후 다시 시도해 주세요.";
static ACTIVE: AtomicBool = AtomicBool::new(false);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorLinks {
    profile_url: String,
    observed_at: String,
    links: Vec<SearchLink>,
    message: String,
}
#[derive(Clone)]
struct Cached {
    id: String,
    source: Url,
    revision: u64,
    cancel: Arc<AtomicBool>,
    result: CreatorLinks,
    created: Instant,
}
pub(super) fn busy() -> bool {
    ACTIVE.load(Ordering::Acquire)
}
pub(super) fn registered_links(session:&XSession,app:&AppHandle,id:&str)->Vec<SearchLink>{
    let Ok((source,revision,cancel))=creator_source(session,app,id,None,None)else{return Vec::new();};
    cached_result(id,&source,revision,&cancel).map(|v|v.links).unwrap_or_default()
}
struct Active<'a>(&'a AtomicBool);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
pub(super) struct Reader(pub(super) Webview);
impl Drop for Reader {
    fn drop(&mut self) {
        let _ = self.0.close();
    }
}

fn original_only(candidate: Option<usize>, link: Option<usize>) -> bool {
    candidate.is_none() && link.is_none()
}
fn acquire<'a>(
    active: &'a AtomicBool,
    budget: Duration,
    mut check: impl FnMut() -> Result<(), String>,
) -> Result<Active<'a>, String> {
    let started = Instant::now();
    loop {
        check()?;
        if active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let guard = Active(active);
            check()?;
            return Ok(guard);
        }
        if started.elapsed() >= budget {
            return Err(LOOKUP_BUSY.into());
        }
        std::thread::sleep(Duration::from_millis(50).min(budget.saturating_sub(started.elapsed())));
    }
}
fn cache_matches(
    cached: &Cached,
    id: &str,
    source: &Url,
    revision: u64,
    cancel: &Arc<AtomicBool>,
) -> bool {
    cached.id == id
        && cached.source == *source
        && cached.revision == revision
        && Arc::ptr_eq(&cached.cancel, cancel)
        && !cancel.load(Ordering::Acquire)
        && cached.created.elapsed() < CACHE_TTL
}
fn cached_result(
    id: &str,
    source: &Url,
    revision: u64,
    cancel: &Arc<AtomicBool>,
) -> Option<CreatorLinks> {
    CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .filter(|cached| cache_matches(cached, id, source, revision, cancel))
        .map(|cached| cached.result.clone())
}

fn profile_url(value: &Url) -> Option<Url> {
    if value.scheme() != "https"
        || !matches!(
            value.host_str(),
            Some("x.com" | "www.x.com" | "twitter.com" | "www.twitter.com")
        )
        || !value.username().is_empty()
        || value.password().is_some()
        || value.port().is_some()
        || value.path().contains('%')
        || value.path().contains('\\')
        || value.path().contains("//")
    {
        return None;
    }
    let pieces: Vec<_> = value.path().trim_matches('/').split('/').collect();
    let handle = *pieces.first()?;
    if !valid_handle(handle)
        || [
            "home",
            "explore",
            "search",
            "notifications",
            "messages",
            "settings",
            "login",
            "logout",
            "signup",
            "i",
            "intent",
            "compose",
            "tos",
            "privacy",
        ]
        .contains(&handle.to_lowercase().as_str())
    {
        return None;
    }
    let post = pieces.len() == 3 && pieces[1] == "status" && valid_id(pieces[2]);
    if pieces.len() != 1 && !post {
        return None;
    }
    Url::parse(&format!("https://x.com/{}", handle.to_lowercase())).ok()
}
fn external_url(value: &str) -> Option<Url> {
    let url = result_url(value)?;
    if url.port().is_some() {
        return None;
    }
    let host = url.host_str()?;
    if !host.contains('.')
        || host.ends_with('.')
        || host.contains(':')
        || host.contains('[')
        || host.parse::<std::net::IpAddr>().is_ok()
        || ["localhost", "local", "internal", "test", "invalid"]
            .iter()
            .any(|part| host == *part || host.ends_with(&format!(".{part}")))
        || matches!(
            host,
            "x.com" | "www.x.com" | "twitter.com" | "www.twitter.com"
        )
    {
        return None;
    }
    Some(url)
}
fn parse_projection(raw: &str, nonce: &str, profile: &Url) -> Result<Option<CreatorLinks>, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Projection {
        nonce: String,
        state: String,
        profile_url: Option<String>,
        links: Option<Vec<SearchLink>>,
        unresolved_short_links: Option<bool>,
    }
    if raw.len() > 24 * 1024 {
        return Err(LOOKUP_ERROR.into());
    }
    let value: Projection = serde_json::from_str(raw).map_err(|_| LOOKUP_ERROR)?;
    if value.nonce != nonce {
        return Err(LOOKUP_ERROR.into());
    }
    match value.state.as_str() {
        "pending" => return Ok(None),
        "ownerChanged" => return Err(STALE_SEARCH.into()),
        "ready" => {}
        _ => return Err(LOOKUP_ERROR.into()),
    }
    if value.profile_url.as_deref() != Some(profile.as_str()) {
        return Err(LOOKUP_ERROR.into());
    }
    let links = value.links.ok_or(LOOKUP_ERROR)?;
    if links.len() > 8 {
        return Err(LOOKUP_ERROR.into());
    }
    let mut seen = HashSet::new();
    for link in &links {
        if link.label.trim().is_empty()
            || link.label.encode_utf16().count() > 128
            || link.label.chars().any(char::is_control)
            || !external_url(&link.url)
                .is_some_and(|url| url.as_str() == link.url && seen.insert(link.url.clone()))
        {
            return Err(LOOKUP_ERROR.into());
        }
    }
    let message = if links.is_empty() {
        "확인한 프로필 소개·웹사이트에 외부 링크가 없습니다."
    } else if value.unresolved_short_links == Some(true) {
        "프로필에 직접 등록된 링크입니다. 일부 단축 링크는 목적지를 확인하지 못했습니다. 해당 이미지의 원작자나 다른 사이트 게시 여부를 보증하지 않습니다."
    } else {
        "프로필에 직접 등록된 링크입니다. 해당 이미지의 원작자나 다른 사이트 게시 여부를 보증하지 않습니다."
    };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| LOOKUP_ERROR)?
        .as_millis();
    let observed = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
        i64::try_from(timestamp).map_err(|_| LOOKUP_ERROR)?,
    )
    .ok_or(LOOKUP_ERROR)?;
    Ok(Some(CreatorLinks {
        profile_url: profile.to_string(),
        observed_at: observed.to_rfc3339(),
        links,
        message: message.into(),
    }))
}

#[cfg(windows)]
fn block_images(view: &Webview) -> Result<(), String> {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE,
        WebResourceRequestedEventHandler,
    };
    use windows::{core::HSTRING, Win32::System::Com::IStream};
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let result = (|| -> windows::core::Result<()> {
            let core = platform.controller().CoreWebView2()?;
            let environment = platform.environment();
            let mut token = 0;
            core.add_WebResourceRequested(
                &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        // This handler is shared with the media blocker. Only image
                        // requests belong here; neither callback reads credentials.
                        let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE;
                        args.ResourceContext(&mut context)?;
                        if context == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE {
                            let response = environment.CreateWebResourceResponse(
                                None::<&IStream>,
                                204,
                                &HSTRING::from("No Content"),
                                &HSTRING::from("Cache-Control: no-store\r\n"),
                            )?;
                            args.SetResponse(&response)?;
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            core.AddWebResourceRequestedFilter(
                &HSTRING::from("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE,
            )?;
            Ok(())
        })();
        let _ = tx.send(result.is_ok());
    })
    .map_err(|_| LOOKUP_ERROR)?;
    if rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false) {
        Ok(())
    } else {
        Err(LOOKUP_ERROR.into())
    }
}
#[cfg(not(windows))]
fn block_images(_: &Webview) -> Result<(), String> {
    Err(LOOKUP_ERROR.into())
}

pub(super) fn create_unloaded_reader(
    app: &AppHandle,
    session: &XSession,
    revision: u64,
    cancel: &AtomicBool,
    profile: &Url,
) -> Result<(Reader, Option<String>), String> {
    session.current(revision, cancel)?;
    let registry = session.registry(app)?;
    let owner = registry
        .accounts
        .iter()
        .find(|a| a.id == registry.active)
        .map(|a| a.handle.clone());
    let directory = accounts::profile(app, &registry.active)?;
    let label = format!("x-feed-reader-creator-{}", uuid::Uuid::new_v4().simple());
    let owned = app.clone();
    let current = session.clone();
    let name = label.clone();
    let target = profile.clone();
    let wanted = Arc::new(AtomicBool::new(true));
    let live = wanted.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        if !live.load(Ordering::Acquire)
            || current.revision.load(Ordering::Acquire) != revision
            || SHUTTING_DOWN.load(Ordering::Acquire)
        {
            return;
        }
        if owned
            .state::<AppState>()
            .begin_platform_work(|| Ok(()))
            .is_err()
        {
            let _ = tx.send(Err(LOOKUP_ERROR.to_string()));
            return;
        }
        let builder = tauri::webview::WebviewBuilder::new(
            &name,
            WebviewUrl::External("about:blank".parse().unwrap()),
        )
        .focused(false)
        .initialization_script(
            include_str!("x_reader_accessibility.js")
                .replace("__ATSUMI_X_READER_DIAGNOSTICS__", "false"),
        )
        .data_directory(directory)
        .browser_extensions_enabled(false)
        .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
        .on_navigation(move |url| {
            url.as_str() == "about:blank"
                || (url.scheme() == "https"
                    && url.host_str() == Some("x.com")
                    && url.port().is_none()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url
                        .path()
                        .trim_end_matches('/')
                        .eq_ignore_ascii_case(target.path()))
        })
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_download(|_, _| false);
        let built = owned
            .get_window("main")
            .ok_or_else(|| LOOKUP_ERROR.to_string())
            .and_then(|parent| {
                parent
                    .add_child(
                        builder,
                        tauri::LogicalPosition::new(-4096.0, 0.0),
                        tauri::LogicalSize::new(1024.0, 900.0),
                    )
                    .and_then(|view| {
                        view.set_auto_resize(false)?;
                        Ok(view)
                    })
                    .map_err(|_| LOOKUP_ERROR.to_string())
            });
        if !live.load(Ordering::Acquire)
            || current.revision.load(Ordering::Acquire) != revision
            || SHUTTING_DOWN.load(Ordering::Acquire)
        {
            if let Ok(view) = built {
                let _ = view.close();
            }
            let _ = tx.send(Err(STALE_SEARCH.into()));
            return;
        }
        let _ = tx.send(built.map(|_| ()));
    })
    .map_err(|_| LOOKUP_ERROR)?;
    let created = rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| LOOKUP_ERROR.to_string())
        .and_then(|v| v);
    wanted.store(false, Ordering::Release);
    if let Err(error) = created {
        if let Some(view) = app.get_webview(&label) {
            let _ = view.close();
        }
        return Err(error);
    }
    let reader = Reader(app.get_webview(&label).ok_or(LOOKUP_ERROR)?);
    session.current(revision, cancel)?;
    block_reader_media(&reader.0, session.clone(), revision)?;
    session.current(revision, cancel)?;
    block_images(&reader.0)?;
    session.current(revision, cancel)?;
    Ok((reader, owner))
}
fn create_reader(app:&AppHandle,session:&XSession,revision:u64,cancel:&AtomicBool,profile:&Url)->Result<(Reader,Option<String>),String>{
    let (reader,owner)=create_unloaded_reader(app,session,revision,cancel,profile)?;
    session.current(revision,cancel)?;reader.0.navigate(profile.clone()).map_err(|_|LOOKUP_ERROR)?;Ok((reader,owner))
}

fn lookup(
    app: &AppHandle,
    session: &XSession,
    id: &str,
    candidate: Option<usize>,
    link: Option<usize>,
) -> Result<CreatorLinks, String> {
    // One automatic original-publisher group owns one immutable indexed cache.
    // Arbitrary similarity candidates cannot replace the displayed link indexes.
    if !original_only(candidate, link) {
        return Err("선택한 미디어의 게시자 프로필만 확인할 수 있습니다.".into());
    }
    let (source, revision, cancel) = creator_source(session, app, id, None, None)?;
    let profile =
        profile_url(&source).ok_or("이 검색 근거에 확인 가능한 X 게시자 주소가 없습니다.")?;
    session.current(revision, &cancel)?;
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return Err(STALE_SEARCH.into());
    }
    if let Some(result) = cached_result(id, &source, revision, &cancel) {
        return Ok(result);
    }
    let _active = acquire(&ACTIVE, ADMISSION_TTL, || {
        session.current(revision, &cancel)?;
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            return Err(STALE_SEARCH.into());
        }
        Ok(())
    })?;
    let (current, current_revision, current_cancel) = creator_source(session, app, id, None, None)?;
    if current != source || current_revision != revision || !Arc::ptr_eq(&current_cancel, &cancel) {
        return Err(STALE_SEARCH.into());
    }
    // Concurrent duplicate calls share the completed predecessor's result;
    // provider progress does not invalidate this original-post identity.
    if let Some(result) = cached_result(id, &source, revision, &cancel) {
        return Ok(result);
    }
    let sequence = SEQUENCE.fetch_add(1, Ordering::AcqRel) + 1;
    *CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = None;
    let (reader, owner) = create_reader(app, session, revision, &cancel, &profile)?;
    let started = Instant::now();
    let mut stable: Option<String> = None;
    loop {
        session.current(revision, &cancel)?;
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            return Err(STALE_SEARCH.into());
        }
        if started.elapsed() >= LOOKUP_TTL {
            return Err(LOOKUP_ERROR.into());
        }
        let before = reader.0.url().map_err(|_| LOOKUP_ERROR)?;
        if profile_url(&before).as_ref() != Some(&profile) {
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        let nonce = uuid::Uuid::new_v4().to_string();
        let script = include_str!("x_creator_links_extract.js").replace(
            "__ATSUMI_X_CREATOR_REQUEST__",
            &json!({"nonce":nonce,"handle":profile.path().trim_matches('/'),"owner":owner})
                .to_string(),
        );
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        reader
            .0
            .eval_with_callback(script, move |raw| {
                let _ = tx.send(raw);
            })
            .map_err(|_| LOOKUP_ERROR)?;
        let raw = rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| LOOKUP_ERROR)?;
        session.current(revision, &cancel)?;
        if !reader.0.url().is_ok_and(|after| after == before) {
            return Err(STALE_SEARCH.into());
        }
        if let Some(result) = parse_projection(&raw, &nonce, &profile)? {
            // A profile header can hydrate across multiple React commits. Do
            // not report an empty link list from the first name-only frame.
            let signature = serde_json::to_string(&(&result.profile_url, &result.links))
                .map_err(|_| LOOKUP_ERROR)?;
            if stable.as_ref() != Some(&signature) || started.elapsed() < Duration::from_secs(1) {
                stable = Some(signature);
                std::thread::sleep(Duration::from_millis(300));
                continue;
            }
            let (current, current_revision, current_cancel) =
                creator_source(session, app, id, None, None)?;
            if current != source
                || current_revision != revision
                || !Arc::ptr_eq(&current_cancel, &cancel)
                || SEQUENCE.load(Ordering::Acquire) != sequence
            {
                return Err(STALE_SEARCH.into());
            }
            *CACHE
                .get_or_init(Mutex::default)
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(Cached {
                id: id.into(),
                source,
                revision,
                cancel: cancel.clone(),
                result: result.clone(),
                created: Instant::now(),
            });
            return Ok(result);
        }
        stable = None;
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn x_media_search_creator_links(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
    id: String,
    candidate_index: Option<usize>,
    link_index: Option<usize>,
) -> Result<CreatorLinks, String> {
    trusted(&window)?;
    let session = session.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        lookup(&app, &session, &id, candidate_index, link_index)
    })
    .await
    .map_err(|_| LOOKUP_ERROR.to_string())?
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_media_search_open_creator_link(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
    id: String,
    link_index: usize,
) -> Result<(), String> {
    trusted(&window)?;
    let cached = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .filter(|value| value.id == id)
        .ok_or(STALE_SEARCH)?;
    let (source, revision, cancel) = creator_source(&session, &app, &id, None, None)?;
    session.current(revision, &cancel)?;
    if !cache_matches(&cached, &id, &source, revision, &cancel) {
        return Err(STALE_SEARCH.into());
    }
    let url = cached
        .result
        .links
        .get(link_index)
        .and_then(|link| external_url(&link.url))
        .ok_or(LOOKUP_ERROR)?;
    crate::media_search_navigation::open_supported_or_external(&app, url.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_original_lookup_can_replace_the_single_profile_cache() {
        assert!(original_only(None, None));
        for selectors in [(Some(0), None), (None, Some(0)), (Some(0), Some(0))] {
            assert!(!original_only(selectors.0, selectors.1));
        }
    }
    #[test]
    fn cached_links_are_bound_to_job_source_revision_cancel_and_expiry() {
        let cancel = Arc::new(AtomicBool::new(false));
        let source = Url::parse("https://x.com/creator/status/123456789").unwrap();
        let mut cached = Cached {
            id: "job".into(),
            source: source.clone(),
            revision: 4,
            cancel: cancel.clone(),
            created: Instant::now(),
            result: CreatorLinks {
                profile_url: "https://x.com/creator".into(),
                observed_at: "2026-10-24T12:00:00Z".into(),
                links: vec![],
                message: String::new(),
            },
        };
        assert!(cache_matches(&cached, "job", &source, 4, &cancel));
        assert!(!cache_matches(&cached, "other", &source, 4, &cancel));
        assert!(!cache_matches(
            &cached,
            "job",
            &Url::parse("https://x.com/other/status/123456789").unwrap(),
            4,
            &cancel
        ));
        assert!(!cache_matches(&cached, "job", &source, 5, &cancel));
        assert!(!cache_matches(
            &cached,
            "job",
            &source,
            4,
            &Arc::new(AtomicBool::new(false))
        ));
        cancel.store(true, Ordering::Release);
        assert!(!cache_matches(&cached, "job", &source, 4, &cancel));
        cancel.store(false, Ordering::Release);
        cached.created = Instant::now()
            .checked_sub(CACHE_TTL + Duration::from_secs(1))
            .unwrap();
        assert!(!cache_matches(&cached, "job", &source, 4, &cancel));
    }
    #[test]
    fn admission_is_bounded_and_cancelled_calls_cannot_take_the_active_slot() {
        let active = AtomicBool::new(true);
        assert!(acquire(&active, Duration::ZERO, || Ok(())).is_err());
        assert!(active.load(Ordering::Acquire));
        let free = AtomicBool::new(false);
        assert!(acquire(&free, ADMISSION_TTL, || Err("cancelled".into())).is_err());
        assert!(!free.load(Ordering::Acquire));
        let mut checks = 0;
        assert!(acquire(&free, ADMISSION_TTL, || {
            checks += 1;
            if checks == 2 {
                Err("cancelled after claim".into())
            } else {
                Ok(())
            }
        })
        .is_err());
        assert!(!free.load(Ordering::Acquire));
        let guard = acquire(&free, Duration::ZERO, || Ok(())).unwrap();
        assert!(free.load(Ordering::Acquire));
        drop(guard);
        assert!(!free.load(Ordering::Acquire));
    }
    #[test]
    fn cancelling_a_queued_lookup_does_not_release_someone_elses_slot() {
        let active = AtomicBool::new(true);
        let cancel = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(10));
                cancel.store(true, Ordering::Release);
            });
            let result = acquire(&active, Duration::from_secs(1), || {
                if cancel.load(Ordering::Acquire) {
                    Err("cancelled".into())
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err());
            assert!(active.load(Ordering::Acquire));
        });
    }
    #[test]
    fn only_exact_x_sources_can_supply_a_profile() {
        for input in [
            "https://x.com/Creator/status/123456789",
            "https://twitter.com/Creator",
        ] {
            assert_eq!(
                profile_url(&Url::parse(input).unwrap()).unwrap().as_str(),
                "https://x.com/creator"
            );
        }
        for input in [
            "https://x.com/home",
            "https://x.com/i/status/123",
            "https://x.com/Creator/status/no",
            "https://x.com/Creator/followers",
            "https://x.com.evil.test/creator",
            "https://me@x.com/creator",
            "https://x.com:444/creator",
            "https://google.com/goto?url=opaque",
            "https://x.com/%63reator",
        ] {
            assert!(
                profile_url(&Url::parse(input).unwrap()).is_none(),
                "{input}"
            );
        }
    }
    #[test]
    fn registered_links_require_public_web_destinations() {
        for input in ["https://www.pixiv.net/users/123456", "https://t.co/fixture"] {
            assert!(external_url(input).is_some());
        }
        for input in [
            "javascript:alert(1)",
            "file:///tmp/test",
            "http://127.0.0.1/file",
            "https://[::1]/",
            "https://[::ffff:127.0.0.1]/",
            "https://server.local/",
            "https://site.test/",
            "https://user:pass@pixiv.net/",
            "https://pixiv.net:444/",
            "https://x.com/other",
        ] {
            assert!(external_url(input).is_none(), "{input}");
        }
    }
    #[test]
    fn projection_is_bound_to_nonce_profile_and_explicit_links() {
        let profile = Url::parse("https://x.com/creator").unwrap();
        let valid = json!({"nonce":"n","state":"ready","profileUrl":profile.as_str(),"links":[{"label":"pixiv","url":"https://www.pixiv.net/users/123"}],"unresolvedShortLinks":false});
        assert_eq!(
            parse_projection(&valid.to_string(), "n", &profile)
                .unwrap()
                .unwrap()
                .links
                .len(),
            1
        );
        assert!(parse_projection(&valid.to_string(), "wrong", &profile).is_err());
        assert!(parse_projection(
            &valid.to_string(),
            "n",
            &Url::parse("https://x.com/other").unwrap()
        )
        .is_err());
        let mut invalid = valid.clone();
        invalid["links"][0]["url"] = json!("http://127.0.0.1/file");
        assert!(parse_projection(&invalid.to_string(), "n", &profile).is_err());
        invalid = valid.clone();
        invalid["creator"] = json!("guessed author");
        assert!(parse_projection(&invalid.to_string(), "n", &profile).is_err());
    }
}
