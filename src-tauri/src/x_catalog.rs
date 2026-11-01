//! Automatic work/sale evidence for a current, native-owned search job.
//! Derived links never become caller-provided navigation or author identity.
use super::page_evidence::{Field, PageEvidence};
use super::*;
#[path = "x_catalog_match.rs"]
mod matching;
use matching::{Claim, Context};
use std::{
    collections::HashSet,
    sync::{atomic::AtomicUsize, Condvar},
};
const MAX_PAGES: usize = 24;
const TTL: Duration = Duration::from_secs(600);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static CACHE: OnceLock<Mutex<Option<Arc<Entry>>>> = OnceLock::new();
#[derive(Clone, Debug, PartialEq)]
struct Root {
    url: String,
    label: String,
    via: Option<String>,
    source_status: Option<&'static str>,
    claims: Vec<Claim>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Work {
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title_evidence: Option<Field>,
    url: String,
    root_url: String,
    origin: String,
    observed_at: String,
    codes: Vec<Field>,
    sale: Vec<Field>,
    ambiguous: bool,
    context: Context,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    report_id: String,
    works: Vec<Work>,
    examined: usize,
    unavailable: usize,
    limited: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_status: Option<&'static str>,
}
struct Entry {
    job: String,
    roots: Vec<Root>,
    parent: Arc<AtomicBool>,
    cancel: AtomicBool,
    created: Instant,
    result: Mutex<Option<Result<Report, String>>>,
    changed: Condvar,
}
struct Active;
impl Drop for Active {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(super) fn busy() -> bool {
    ACTIVE.load(Ordering::Acquire) > 0
}
fn same(entry: &Entry, id: &str, roots: &[Root], parent: &Arc<AtomicBool>) -> bool {
    entry.job == id
        && entry.roots == roots
        && Arc::ptr_eq(&entry.parent, parent)
        && !entry.parent.load(Ordering::Acquire)
        && !entry.cancel.load(Ordering::Acquire)
        && entry.created.elapsed() < TTL
}
fn roots(session: &XSession, app: &AppHandle, id: &str) -> Result<(Fence, Vec<Root>), String> {
    let fence = current_fence(id).ok_or(STALE_SEARCH)?;
    if !owned_current(session, app, &fence) {
        return Err(STALE_SEARCH.into());
    }
    let mut roots = Vec::new();
    if let Ok((post, _, _)) = creator_source(session, app, id, None, None) {
        let projected = source_links::registered(session, app, id);
        let status = match projected.as_ref() {
            Some(Ok(value)) if value.limited => "partial",
            Some(Ok(_)) => "complete",
            _ => "unavailable",
        };
        if let Some(Ok(value)) = projected {
            for link in value.links {
                roots.push(Root {
                    url: link.url,
                    label: if link.reply {
                        "게시자 타래 등록 링크"
                    } else {
                        "원 게시물 등록 링크"
                    }
                    .into(),
                    via: Some(link.post_url),
                    source_status: None,
                    claims: Vec::new(),
                });
            }
        }
        roots.push(Root {
            url: post.to_string(),
            label: "원 게시물".into(),
            via: None,
            source_status: Some(status),
            claims: Vec::new(),
        });
    }
    roots.extend(
        creator_links::registered_links(session, app, id)
            .into_iter()
            .map(|link| Root {
                url: link.url,
                label: "게시자 등록 링크".into(),
                via: None,
                source_status: None,
                claims: Vec::new(),
            })
            .collect::<Vec<_>>(),
    );
    {
        let data = controller().lock().unwrap_or_else(|p| p.into_inner());
        let job = data
            .current
            .as_ref()
            .filter(|j| fence.matches(j))
            .ok_or(STALE_SEARCH)?;
        if !matches!(job.snapshot.phase.as_str(), "complete" | "failed") {
            return Err(STALE_SEARCH.into());
        }
        let mut append = |provider: Provider, candidates: &[Candidate]| {
            for (i, candidate) in candidates.iter().enumerate() {
                for link in &candidate.links {
                    if let Some(url) = result_url(&link.url) {
                        roots.push(Root {
                            url: url.to_string(),
                            label: format!(
                                "{} {}",
                                if provider == Provider::Lens {
                                    "Google Lens"
                                } else {
                                    "SauceNAO"
                                },
                                i + 1
                            ),
                            via: None,
                            source_status: None,
                            claims: vec![Claim::new(provider, i, candidate, link)],
                        });
                    }
                }
            }
        };
        if job.snapshot.providers.is_empty() {
            if job.snapshot.phase == "complete" {
                append(job.snapshot.provider, &job.snapshot.candidates);
            }
        } else {
            for provider in job
                .snapshot
                .providers
                .iter()
                .filter(|p| p.phase == "complete")
            {
                append(provider.provider, &provider.candidates);
            }
        }
    }
    matching::merge_roots(&mut roots);
    Ok((fence, roots))
}
#[derive(Clone)]
struct Task {
    url: String,
    root: Root,
    depth: usize,
    trail: Vec<(String, String, &'static str)>,
}
struct Scan {
    queue: VecDeque<Task>,
    seen: HashSet<String>,
    reserved: usize,
    report: Report,
    pages: Vec<(Task, PageEvidence)>,
    failed: HashSet<String>,
}
fn analyze(
    entry: &Entry,
    budget: Duration,
    fetch: &(impl Fn(&str, &AtomicBool, Instant) -> Result<PageEvidence, String> + Sync),
) -> Result<Report, String> {
    let end = Instant::now() + budget;
    let scan = Mutex::new(Scan {
        queue: entry
            .roots
            .iter()
            .take(MAX_PAGES)
            .map(|root| Task {
                url: root.url.clone(),
                root: root.clone(),
                depth: 0,
                trail: Vec::new(),
            })
            .collect(),
        seen: HashSet::new(),
        reserved: 0,
        pages: Vec::new(),
        failed: HashSet::new(),
        report: Report {
            report_id: uuid::Uuid::new_v4().to_string(),
            works: Vec::new(),
            examined: 0,
            unavailable: 0,
            limited: entry.roots.len() > MAX_PAGES,
            source_status: entry.roots.iter().find_map(|root| root.source_status),
        },
    });
    std::thread::scope(|scope| {
        for _ in 0..2 {
            let scan = &scan;
            scope.spawn(move || loop {
                if entry.cancel.load(Ordering::Acquire) || entry.parent.load(Ordering::Acquire) {
                    return;
                }
                let task = {
                    let mut scan = scan.lock().unwrap_or_else(|p| p.into_inner());
                    if Instant::now() >= end || scan.reserved >= MAX_PAGES {
                        scan.report.limited |= !scan.queue.is_empty();
                        return;
                    }
                    let mut next = None;
                    while let Some(item) = scan.queue.pop_front() {
                        if scan.seen.insert(item.url.clone()) {
                            next = Some(item);
                            break;
                        }
                    }
                    let Some(task) = next else {
                        return;
                    };
                    scan.reserved += 1;
                    task
                };
                let fetched = fetch(&task.url, &entry.parent, end);
                let mut scan = scan.lock().unwrap_or_else(|p| p.into_inner());
                let evidence = match fetched {
                    Ok(v) => v,
                    Err(_) => {
                        scan.report.unavailable += 1;
                        scan.failed.insert(task.url.clone());
                        continue;
                    }
                };
                if entry.cancel.load(Ordering::Acquire) || entry.parent.load(Ordering::Acquire) {
                    return;
                }
                scan.report.examined += 1;
                scan.report.limited |= evidence.catalog.limited || evidence.limited;
                if task.depth < 2 {
                    for url in evidence.catalog.links.iter().take(2) {
                        if scan.queue.len() >= MAX_PAGES {
                            scan.report.limited = true;
                            break;
                        }
                        if !scan.seen.contains(url) {
                            let relation = evidence
                                .catalog
                                .relations
                                .iter()
                                .find(|(target, _)| target == url)
                                .map(|(_, kind)| *kind)
                                .unwrap_or("reference");
                            let mut trail = task.trail.clone();
                            trail.push((evidence.url.clone(), url.clone(), relation));
                            scan.queue.push_back(Task {
                                url: url.clone(),
                                root: task.root.clone(),
                                depth: task.depth + 1,
                                trail,
                            });
                        }
                    }
                } else {
                    scan.report.limited |= !evidence.catalog.links.is_empty();
                }
                scan.pages.push((task, evidence));
            });
        }
    });
    if entry.cancel.load(Ordering::Acquire) || entry.parent.load(Ordering::Acquire) {
        return Err(STALE_SEARCH.into());
    }
    let mut scan = scan.into_inner().unwrap_or_else(|p| p.into_inner());
    scan.report.works = matching::reconcile(&entry.roots, &scan.pages, &scan.failed);
    Ok(scan.report)
}
fn read(id: &str, roots: Vec<Root>, parent: Arc<AtomicBool>) -> Result<Report, String> {
    ACTIVE.fetch_add(1, Ordering::AcqRel);
    let _active = Active;
    let (entry, created) = {
        let mut cache = CACHE
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = cache.as_ref().filter(|e| same(e, id, &roots, &parent)) {
            (entry.clone(), false)
        } else {
            if let Some(old) = cache.as_ref() {
                old.cancel.store(true, Ordering::Release);
                old.changed.notify_all();
            }
            let entry = Arc::new(Entry {
                job: id.into(),
                roots,
                parent,
                cancel: AtomicBool::new(false),
                created: Instant::now(),
                result: Mutex::new(None),
                changed: Condvar::new(),
            });
            *cache = Some(entry.clone());
            (entry, true)
        }
    };
    if created {
        let report = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            analyze(
                &entry,
                Duration::from_secs(32),
                &page_evidence::read_result_until,
            )
        }))
        .unwrap_or_else(|_| Err("작품·판매 정보를 읽지 못했습니다.".into()));
        *entry.result.lock().unwrap_or_else(|p| p.into_inner()) = Some(report);
        entry.changed.notify_all();
    }
    let mut result = entry.result.lock().unwrap_or_else(|p| p.into_inner());
    loop {
        if entry.cancel.load(Ordering::Acquire) || entry.parent.load(Ordering::Acquire) {
            return Err(STALE_SEARCH.into());
        }
        if let Some(result) = result.as_ref() {
            return result.clone();
        }
        if entry.created.elapsed() > Duration::from_secs(45) {
            return Err("작품·판매 정보 확인 시간이 초과되었습니다.".into());
        }
        result = entry
            .changed
            .wait_timeout(result, Duration::from_millis(100))
            .unwrap_or_else(|p| p.into_inner())
            .0;
    }
}
#[tauri::command(rename_all = "camelCase")]
pub async fn x_media_search_work_evidence(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
    id: String,
) -> Result<Report, String> {
    trusted(&window)?;
    maintain(&session, &app, &id);
    let (fence, _) = roots(&session, &app, &id)?;
    ACTIVE.fetch_add(1, Ordering::AcqRel);
    let _active = Active;
    let parent = fence.cancel.clone();
    let work_id = id.clone();
    let owned = app.clone();
    let host = session.inner().clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        let _ = source_links::lookup(&host, &owned, &work_id);
        let (_, seeds) = roots(&host, &owned, &work_id)?;
        read(&work_id, seeds, parent)
    })
    .await
    .map_err(|_| "작품·판매 정보를 확인하지 못했습니다.".to_string())??;
    if !owned_current(&session, &app, &fence) {
        return Err(STALE_SEARCH.into());
    }
    Ok(report)
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_media_search_open_work_evidence(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
    id: String,
    report_id: String,
    index: usize,
    origin: Option<bool>,
) -> Result<(), String> {
    trusted(&window)?;
    maintain(&session, &app, &id);
    let (fence, seeds) = roots(&session, &app, &id)?;
    let entry = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .filter(|e| same(e, &id, &seeds, &fence.cancel))
        .ok_or(STALE_SEARCH)?;
    let target = {
        let guard = entry.result.lock().unwrap_or_else(|p| p.into_inner());
        let report = guard
            .as_ref()
            .and_then(|v| v.as_ref().ok())
            .filter(|v| v.report_id == report_id)
            .ok_or(STALE_SEARCH)?;
        let work = report.works.get(index).ok_or(INVALID)?;
        if origin == Some(true) {
            let target = Url::parse(&work.root_url).map_err(|_| INVALID)?;
            source_link_parse::source(&target).ok_or(INVALID)?;
            work.root_url.clone()
        } else {
            work.url.clone()
        }
    };
    if !owned_current(&session, &app, &fence) {
        return Err(STALE_SEARCH.into());
    }
    crate::media_search_navigation::open_supported_or_external(&app, &target)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(roots: Vec<Root>) -> Entry {
        Entry {
            job: "job".into(),
            roots,
            parent: Arc::new(AtomicBool::new(false)),
            cancel: AtomicBool::new(false),
            created: Instant::now(),
            result: Mutex::new(None),
            changed: Condvar::new(),
        }
    }
    fn root(url: &str) -> Root {
        Root {
            url: url.into(),
            label: "Published source".into(),
            via: None,
            source_status: None,
            claims: Vec::new(),
        }
    }
    fn claim_root(url: &str, title: &str, provider: Provider) -> Root {
        let mut root = root(url);
        let link = SearchLink {
            url: url.into(),
            label: "Source".into(),
        };
        root.claims.push(Claim::new(
            provider,
            0,
            &Candidate {
                title: title.into(),
                creator: None,
                similarity: Some(88.0),
                links: vec![link.clone()],
                thumbnail_data: None,
                source_metadata: vec![],
            },
            &link,
        ));
        root
    }
    fn page(url: &str, html: &str) -> PageEvidence {
        let mut p = PageEvidence::empty(url);
        p.catalog = super::super::catalog_parse::parse(url, html);
        p
    }
    #[test]
    fn unavailable_candidate_and_corroboration_survive_readable_unrelated_upload() {
        let e = entry(vec![
            claim_root(
                "https://catalog.example/film",
                "Aurora Garden Story",
                Provider::Saucenao,
            ),
            claim_root(
                "https://mirror.example/video",
                "Aurora Garden Story ep 2 HD",
                Provider::Lens,
            ),
            claim_root(
                "https://upload.example/video",
                "Unrelated uploader heading",
                Provider::Lens,
            ),
        ]);
        let r=analyze(&e,Duration::from_secs(2),&|url,_,_| {
            if url.contains("catalog.") {return Err("unavailable".into());}
            let title=if url.contains("mirror."){ "Aurora Garden Story ep 2 HD" } else {"Unrelated uploader heading"};
            Ok(page(url,&format!(r#"<script type="application/ld+json">{{"@type":"VideoObject","name":"{title}"}}</script>"#)))
        }).unwrap();
        assert_eq!(r.works.len(), 3);
        assert_eq!(r.works[0].title.as_deref(), Some("Aurora Garden Story"));
        assert_eq!(r.works[0].context.page_status, "unavailable");
        assert_eq!(r.works[0].context.status, "supported");
        let unrelated = r.works.iter().find(|w| w.url.contains("upload.")).unwrap();
        assert_ne!(unrelated.context.group, r.works[0].context.group);
        assert_eq!(unrelated.context.kind, "upload");
        assert_eq!(unrelated.context.association, "unverified");
        assert!(r
            .works
            .iter()
            .all(|w| w.codes.is_empty() && w.sale.is_empty()));
    }
    #[test]
    fn same_url_keeps_both_provider_claims_including_similarity() {
        let mut roots = vec![
            claim_root(
                "https://example.com/book",
                "Example Book",
                Provider::Saucenao,
            ),
            claim_root("https://example.com/book", "Example Book", Provider::Lens),
        ];
        matching::merge_roots(&mut roots);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].claims.len(), 2);
        let r = analyze(&entry(roots), Duration::from_secs(1), &|_, _, _| {
            Err("offline".into())
        })
        .unwrap();
        assert_eq!(r.works.len(), 1);
        assert_eq!(r.works[0].context.status, "supported");
        let serialized = serde_json::to_value(&r).unwrap();
        assert_eq!(
            serialized["works"][0]["context"]["claims"][0]["similarity"],
            88.0
        );
    }
    #[test]
    fn redirect_aliases_merge_all_claims_without_stale_unread_seed() {
        let roots = vec![
            claim_root("https://example.com/old", "Exact Book", Provider::Saucenao),
            claim_root("https://example.com/current", "Exact Book", Provider::Lens),
        ];
        let r=analyze(&entry(roots),Duration::from_secs(2),&|url,_,_|{let mut p=page(url,r#"<script type="application/ld+json">{"@type":"Book","name":"Exact Book","isbn":"9781718504448"}</script>"#);p.url="https://example.com/current".into();Ok(p)}).unwrap();
        assert_eq!(r.works.len(), 1);
        assert_eq!(r.works[0].context.claims.len(), 2);
        assert_eq!(r.works[0].context.page_status, "read");
    }
    #[test]
    fn publisher_link_preserves_its_own_title_when_a_search_title_disagrees() {
        let mut source = claim_root(
            "https://publisher.example/book",
            "Noisy search caption",
            Provider::Lens,
        );
        source.via = Some("https://x.com/publisher/status/123".into());
        let r=analyze(&entry(vec![source]),Duration::from_secs(2),&|url,_,_|Ok(page(url,r#"<script type="application/ld+json">{"@type":"Book","name":"Exact Publisher Book","isbn":"9781718504448"}</script>"#))).unwrap();
        assert_eq!(r.works[0].title.as_deref(), Some("Exact Publisher Book"));
        assert_eq!(r.works[0].context.association, "source_link");
        assert_eq!(r.works[0].context.claims[0].title, "Noisy search caption");
        assert!(r.works[0]
            .context
            .reasons
            .iter()
            .any(|v| v.contains("다릅니다")));
    }
    #[test]
    fn different_editions_and_seller_local_skus_never_merge_sale_facts() {
        let roots = vec![
            claim_root(
                "https://one.example/book",
                "Example Complete Book",
                Provider::Saucenao,
            ),
            claim_root(
                "https://two.example/book",
                "Example Complete Book",
                Provider::Lens,
            ),
        ];
        let r=analyze(&entry(roots),Duration::from_secs(2),&|url,_,_|{
            let isbn=if url.contains("one."){ "9781718500440" }else{"9781718504448"};
            Ok(page(url,&format!(r#"<script type="application/ld+json">{{"@type":"Book","name":"Example Complete Book","isbn":"{isbn}","sku":"00042","offers":{{"price":"10","priceCurrency":"USD"}}}}</script>"#)))
        }).unwrap();
        assert_ne!(r.works[0].context.group, r.works[1].context.group);
        assert!(r.works.iter().all(|w| w.context.status == "conflict"));
    }
    #[test]
    fn explicit_purchase_and_same_isbn_associate_but_keep_exact_per_page_prices() {
        let e = entry(vec![claim_root(
            "https://publisher.example/book",
            "Example Complete Book",
            Provider::Saucenao,
        )]);
        let r=analyze(&e,Duration::from_secs(2),&|url,_,_|{
            let retail=url.contains("retailer.");
            Ok(page(url,&format!(r#"<script type="application/ld+json">{{"@type":"Book","name":"Example Complete Book","isbn":"9781718504448","offers":{{"price":"{}","priceCurrency":"USD"}}}}</script><main><a href="https://retailer.example/book">Buy</a></main>"#,if retail {"15"} else {"20"})))
        }).unwrap();
        assert_eq!(r.works.len(), 2);
        assert_eq!(r.works[0].context.group, r.works[1].context.group);
        assert!(r
            .works
            .iter()
            .all(|w| w.context.association == "identifier"));
        assert_eq!(r.works[0].sale[0].value, "20");
        assert_eq!(r.works[1].sale[0].value, "15");
    }
    #[test]
    fn related_work_and_conflicting_product_title_cannot_become_original_sale_information() {
        let roots = vec![
            claim_root(
                "https://example.com/video",
                "Aurora Garden Story",
                Provider::Saucenao,
            ),
            claim_root(
                "https://wrong.example/product",
                "Aurora Garden Story",
                Provider::Lens,
            ),
        ];
        let r=analyze(&entry(roots),Duration::from_secs(2),&|url,_,_|Ok(page(url,if url.ends_with("video"){
            r#"<script type="application/ld+json">{"@type":"VideoObject","name":"Aurora Garden Story","isBasedOn":"https://example.com/book"}</script>"#
        }else{r#"<script type="application/ld+json">{"@type":"Book","name":"Different Unrelated Book","isbn":"9781718504448","offers":{"price":"29"}}</script>"#}))).unwrap();
        let video = r.works.iter().find(|w| w.url.ends_with("video")).unwrap();
        let book = r.works.iter().find(|w| w.url.ends_with("book")).unwrap();
        let wrong = r.works.iter().find(|w| w.url.contains("wrong.")).unwrap();
        assert_ne!(video.context.group, book.context.group);
        assert_eq!(book.context.association, "unverified");
        assert_eq!(wrong.context.status, "conflict");
        assert_eq!(
            wrong.context.page_title.as_deref(),
            Some("Different Unrelated Book")
        );
        assert_eq!(wrong.title.as_deref(), Some("Aurora Garden Story"));
    }
    #[test]
    fn follows_only_explicit_bounded_trails_and_keeps_the_root_and_page_codes() {
        let entry = entry(vec![root("https://example.com/source")]);
        let count = AtomicUsize::new(0);
        let report=analyze(&entry,Duration::from_secs(2),&|url,_,_|{count.fetch_add(1,Ordering::AcqRel);let mut p=PageEvidence::empty(url);p.catalog=super::super::catalog_parse::parse(url,if url.ends_with("source"){r#"<main><h1>Post</h1><a href="/retail">購入</a></main>"#}else{r#"<main><h1>Actual work</h1><dl><dt>品番</dt><dd>CODE-0042</dd></dl><a href="/source">Source</a></main>"#});Ok(p)}).unwrap();
        assert_eq!(count.load(Ordering::Acquire), 2);
        assert_eq!(report.works.len(), 1);
        assert_eq!(report.works[0].root_url, "https://example.com/source");
        assert_eq!(report.works[0].codes[0].value, "CODE-0042");
    }
    #[test]
    fn automatic_scan_keeps_an_explicit_primary_work_title_without_a_code_or_offer() {
        let e = entry(vec![root("https://example.com/film")]);
        let report = analyze(&e, Duration::from_secs(2), &|url, _, _| {
            let mut page = PageEvidence::empty(url);
            page.catalog = super::super::catalog_parse::parse(
                url,
                r#"<script type="application/ld+json">{"@type":"Movie","url":"https://example.com/film","name":"A Literal Film Title"}</script><main><h1>Other page heading</h1></main>"#,
            );
            Ok(page)
        }).unwrap();
        assert_eq!(
            report.works.len(),
            1,
            "a declared work title is useful even without a product code"
        );
        assert_eq!(
            report.works[0].title.as_deref(),
            Some("A Literal Film Title")
        );
        assert!(report.works[0].codes.is_empty());
        assert!(report.works[0].sale.is_empty());
        assert_eq!(
            report.works[0].title_evidence.as_ref().unwrap().source,
            "json-ld[0].name"
        );
    }
    #[test]
    fn automatic_title_only_scan_keeps_a_published_trail_and_ignores_unproved_titles() {
        let e = entry(vec![root("https://example.com/source")]);
        let report = analyze(&e, Duration::from_secs(2), &|url, _, _| {
            let mut page = PageEvidence::empty(url);
            page.catalog = super::super::catalog_parse::parse(url, if url.ends_with("source") {
                r#"<main><h1>Ordinary post heading</h1><a href="/work">Source</a></main>"#
            } else {
                r#"<main itemscope itemtype="https://schema.org/Book"><span itemprop="name">A Literal Book Title</span></main>"#
            });
            Ok(page)
        }).unwrap();
        assert_eq!(report.examined, 2);
        assert_eq!(report.works.len(), 1);
        assert_eq!(report.works[0].url, "https://example.com/work");
        assert_eq!(report.works[0].root_url, "https://example.com/source");
        assert_eq!(
            report.works[0].title.as_deref(),
            Some("A Literal Book Title")
        );
        assert_eq!(
            report.works[0].title_evidence.as_ref().unwrap().source,
            "microdata:name"
        );
    }
    #[test]
    fn failure_is_not_reported_as_verified_code_absence_and_total_work_is_bounded() {
        let entry = entry(
            (0..80)
                .map(|n| root(&format!("https://example.com/{n}")))
                .collect(),
        );
        let count = AtomicUsize::new(0);
        let r = analyze(&entry, Duration::from_secs(2), &|_, _, _| {
            count.fetch_add(1, Ordering::AcqRel);
            Err("unavailable".into())
        })
        .unwrap();
        assert_eq!(r.examined, 0);
        assert_eq!(r.unavailable, 24);
        assert!(r.limited);
        assert_eq!(count.load(Ordering::Acquire), 24);
    }
    #[test]
    fn cache_ownership_changes_revoke_results_and_late_readers() {
        let e = entry(vec![root("https://example.com/")]);
        assert!(same(&e, "job", &e.roots, &e.parent));
        assert!(!same(
            &e,
            "job",
            &e.roots,
            &Arc::new(AtomicBool::new(false))
        ));
        assert!(!same(&e, "other", &e.roots, &e.parent));
        e.cancel.store(true, Ordering::Release);
        assert!(!same(&e, "job", &e.roots, &e.parent));
        let count = AtomicUsize::new(0);
        assert!(analyze(&e, Duration::from_secs(1), &|_, _, _| {
            count.fetch_add(1, Ordering::AcqRel);
            Err("must not fetch".into())
        })
        .is_err());
        assert_eq!(count.load(Ordering::Acquire), 0);
    }
    #[test]
    fn exhausted_scan_budget_cannot_start_another_redirect_or_page_read() {
        let e = entry(vec![root("https://example.com/source")]);
        let count = AtomicUsize::new(0);
        let r = analyze(&e, Duration::ZERO, &|_, _, _| {
            count.fetch_add(1, Ordering::AcqRel);
            Err("must not read".into())
        })
        .unwrap();
        assert!(r.limited);
        assert_eq!(r.examined, 0);
        assert_eq!(count.load(Ordering::Acquire), 0);
    }
}
