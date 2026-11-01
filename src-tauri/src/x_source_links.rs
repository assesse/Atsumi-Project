//! Read only links the original selected publisher actually posted. Official
//! page requests stay source-owned; no private API call or credential export.
#[path = "x_source_link_observer.rs"]
mod observer;
use super::source_link_parse::{self, Projection};
use super::*;
use std::sync::{atomic::AtomicUsize, Condvar};
const ERROR: &str = "게시물·답글의 등록 링크를 확인하지 못했습니다.";
const TTL: Duration = Duration::from_secs(600);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static CACHE: OnceLock<Mutex<Option<Arc<Entry>>>> = OnceLock::new();
struct Entry {
    id: String,
    source: Url,
    revision: u64,
    parent: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    created: Instant,
    result: Mutex<Option<Result<Projection, String>>>,
    changed: Condvar,
}
struct Active;
impl Drop for Active {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Lease(Arc<AtomicBool>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
pub(super) fn busy() -> bool {
    ACTIVE.load(Ordering::Acquire) > 0
}
fn matches(entry: &Entry, id: &str, source: &Url, revision: u64, parent: &Arc<AtomicBool>) -> bool {
    entry.id == id
        && entry.source == *source
        && entry.revision == revision
        && Arc::ptr_eq(&entry.parent, parent)
        && !parent.load(Ordering::Acquire)
        && entry.created.elapsed() < TTL
}
pub(super) fn registered(
    session: &XSession,
    app: &AppHandle,
    id: &str,
) -> Option<Result<Projection, String>> {
    let (source, revision, parent) = creator_source(session, app, id, None, None).ok()?;
    let entry = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .filter(|e| matches(e, id, &source, revision, &parent))
        .cloned()?;
    let result = entry
        .result
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    result
}
fn current(session: &XSession, app: &AppHandle, entry: &Entry) -> Result<(), String> {
    session.current(entry.revision, &entry.parent)?;
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return Err(STALE_SEARCH.into());
    }
    let (source, revision, parent) = creator_source(session, app, &entry.id, None, None)?;
    if !matches(entry, &entry.id, &source, revision, &parent) {
        return Err(STALE_SEARCH.into());
    }
    Ok(())
}
pub(super) fn lookup(session: &XSession, app: &AppHandle, id: &str) -> Result<Projection, String> {
    ACTIVE.fetch_add(1, Ordering::AcqRel);
    let _active = Active;
    let (source, revision, parent) = creator_source(session, app, id, None, None)?;
    source_link_parse::source(&source).ok_or(ERROR)?;
    let (entry, created) = {
        let mut cache = CACHE
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = cache
            .as_ref()
            .filter(|e| matches(e, id, &source, revision, &parent))
        {
            (entry.clone(), false)
        } else {
            if let Some(old) = cache.as_ref() {
                old.stop.store(true, Ordering::Release);
                old.changed.notify_all();
            }
            let entry = Arc::new(Entry {
                id: id.into(),
                source,
                revision,
                parent,
                stop: Arc::new(AtomicBool::new(false)),
                created: Instant::now(),
                result: Mutex::new(None),
                changed: Condvar::new(),
            });
            *cache = Some(entry.clone());
            (entry, true)
        }
    };
    if created {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(session, app, &entry)))
                .unwrap_or_else(|_| Err(ERROR.into()));
        *entry.result.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
        entry.changed.notify_all();
    }
    let mut result = entry.result.lock().unwrap_or_else(|p| p.into_inner());
    loop {
        current(session, app, &entry)?;
        if let Some(result) = result.as_ref() {
            return result.clone();
        }
        if entry.created.elapsed() > Duration::from_secs(25) {
            return Err(ERROR.into());
        }
        result = entry
            .changed
            .wait_timeout(result, Duration::from_millis(100))
            .unwrap_or_else(|p| p.into_inner())
            .0;
    }
}
fn owner_probe(view: &Webview, owner: &str, entry: &Entry) -> Result<bool, String> {
    let script = format!(
        r#"(()=>{{const expected=new URL({source}),current=new URL(location.href);if(current.origin!==expected.origin||current.pathname.toLowerCase()!==expected.pathname.toLowerCase()||current.search||current.hash)return false;const link=document.querySelector('[data-testid="AppTabBar_Profile_Link"]');const account=document.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]');if(document.querySelector('input[type="password"],input[autocomplete="one-time-code"],[data-testid="LoginForm_Login_Button"]'))return false;try{{const url=new URL(link?.getAttribute('href')||'',location.origin);return !!account&&url.origin==='https://x.com'&&url.pathname.toLowerCase().replace(/\/$/,'')==={owner};}}catch{{return false;}}}})()"#,
        source = serde_json::to_string(entry.source.as_str()).unwrap(),
        owner = serde_json::to_string(&format!("/{}", owner.to_ascii_lowercase())).unwrap()
    );
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(script, move |value| {
        let _ = tx.send(value);
    })
    .map_err(|_| ERROR)?;
    let raw = rx.recv_timeout(Duration::from_secs(1)).map_err(|_| ERROR)?;
    Ok(serde_json::from_str::<bool>(&raw).unwrap_or(false))
}
fn read(session: &XSession, app: &AppHandle, entry: &Entry) -> Result<Projection, String> {
    current(session, app, entry)?;
    let _lease = Lease(entry.stop.clone());
    let end = entry.created + Duration::from_secs(18);
    let (reader, owner) = creator_links::create_unloaded_reader(
        app,
        session,
        entry.revision,
        &entry.parent,
        &entry.source,
    )?;
    let owner = owner.ok_or(ERROR)?;
    let payload = Arc::new(Mutex::new(None));
    observer::observe(
        &reader.0,
        entry.source.clone(),
        entry.stop.clone(),
        payload.clone(),
    )?;
    current(session, app, entry)?;
    reader.0.navigate(entry.source.clone()).map_err(|_| ERROR)?;
    loop {
        current(session, app, entry)?;
        if entry.stop.load(Ordering::Acquire) || Instant::now() >= end {
            return Err(ERROR.into());
        }
        let projection = payload.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(projection) = projection {
            if owner_probe(&reader.0, &owner, entry)? {
                current(session, app, entry)?;
                return Ok(projection);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_reader_cache_retains_only_its_exact_source_revision_and_cancel_identity() {
        let parent = Arc::new(AtomicBool::new(false));
        let source: Url = "https://x.com/author/status/100001".parse().unwrap();
        let entry = Entry {
            id: "job".into(),
            source: source.clone(),
            revision: 4,
            parent: parent.clone(),
            stop: Arc::new(AtomicBool::new(true)),
            created: Instant::now(),
            result: Mutex::new(Some(Ok(Projection {
                links: Vec::new(),
                limited: false,
            }))),
            changed: Condvar::new(),
        };
        assert!(matches(&entry, "job", &source, 4, &parent));
        assert!(!matches(&entry, "other", &source, 4, &parent));
        assert!(!matches(&entry, "job", &source, 5, &parent));
        assert!(!matches(
            &entry,
            "job",
            &"https://x.com/other/status/100001".parse().unwrap(),
            4,
            &parent
        ));
        assert!(!matches(
            &entry,
            "job",
            &source,
            4,
            &Arc::new(AtomicBool::new(false))
        ));
        parent.store(true, Ordering::Release);
        assert!(!matches(&entry, "job", &source, 4, &parent));
    }
}
