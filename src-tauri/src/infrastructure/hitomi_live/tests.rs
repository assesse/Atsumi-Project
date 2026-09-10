use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    io::Cursor,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use reqwest::Url;

use crate::{
    application::{
        ArtifactStore, AutoFindSource, AutoFindSourceRequest, DownloadSourcePort,
        ExistingPageVerification, GallerySummaryCache, RepositoryError, SearchRepository,
        TagCatalogSource,
    },
    domain::{ArtifactRelativePath, GalleryId, Language, SearchRequest, SearchSort},
    infrastructure::{FilesystemArtifactStore, SqliteRepository},
    source::{
        hitomi::{
            download_full_candidates, galleries_index_file_url, galleries_index_version_url,
            gallery_index_term_key, galleryinfo_script_url, gg_script_url,
            parse_galleryinfo_script, parse_gg_routing, webp_full_candidates,
            webp_thumbnail_candidates, ThumbnailSize, HITOMI_METADATA_ORIGIN,
        },
        SourceContractError, SourceErrorCode,
    },
    thumbnail::{CancellationToken, ThumbnailKey, ThumbnailResolver},
};

use super::{
    decode_download_payload,
    http::{validate_source_url, HttpPayload, HttpRequest, HttpTransport},
    search::{prefixed_nozomi_path, tag_nozomi_path},
    HitomiLiveAdapter, HitomiLiveConfig,
};

#[test]
fn download_decode_accepts_empty_or_octet_stream_only_when_magic_matches() {
    let page = crate::domain::SourcePageNumber::new(1).unwrap();
    for content_type in ["", "application/octet-stream"] {
        let decoded = decode_download_payload(
            HttpPayload {
                status: 200,
                bytes: one_pixel_png(),
                content_type: content_type.into(),
            },
            page,
            "fixture-v1".into(),
            0,
            crate::source::hitomi::HitomiImageFormat::Png,
        )
        .unwrap();
        assert_eq!(decoded.source_format.as_str(), "png");
    }
    let mismatch = decode_download_payload(
        HttpPayload {
            status: 200,
            bytes: one_pixel_png(),
            content_type: "image/png".into(),
        },
        page,
        "fixture-v1".into(),
        0,
        crate::source::hitomi::HitomiImageFormat::Webp,
    )
    .unwrap_err();
    assert_eq!(mismatch.code.as_str(), "image_response_invalid");
}

const GALLERY_SCRIPT: &str = include_str!("../../../fixtures/hitomi/galleryinfo-normal.js");
const GG_SCRIPT: &str = include_str!("../../../fixtures/hitomi/gg-current.js");

#[derive(Default)]
struct FakeTransport {
    responses: Mutex<HashMap<String, VecDeque<Result<HttpPayload, SourceContractError>>>>,
    calls: Mutex<Vec<String>>,
    requests: Mutex<Vec<(String, Option<String>)>>,
}

impl FakeTransport {
    fn respond(&self, url: String, content_type: &str, bytes: Vec<u8>) {
        self.responses
            .lock()
            .unwrap()
            .entry(url)
            .or_default()
            .push_back(Ok(HttpPayload {
                status: 200,
                bytes,
                content_type: content_type.to_owned(),
            }));
    }

    fn fail(&self, url: String, error: SourceContractError) {
        self.responses
            .lock()
            .unwrap()
            .entry(url)
            .or_default()
            .push_back(Err(error));
    }

    fn call_count(&self, url: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.as_str() == url)
            .count()
    }

    fn was_called(&self, url: &str) -> bool {
        self.call_count(url) > 0
    }

    fn ranges_for(&self, url: &str) -> Vec<Option<String>> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(request_url, _)| request_url == url)
            .map(|(_, range)| range.clone())
            .collect()
    }
}

#[test]
fn gallery_summary_fetches_only_main_metadata_and_reuses_the_shared_cache() {
    let transport = Arc::new(FakeTransport::default());
    let main_url = galleryinfo_script_url(7_001).unwrap();
    let related_url = galleryinfo_script_url(7_002).unwrap();
    transport.respond(
        main_url.clone(),
        "text/javascript",
        gallery_script(7_001, "Main summary fixture", "[7002]").into_bytes(),
    );
    transport.fail(
        related_url.clone(),
        crate::source::map_http_status(503, None).unwrap_err(),
    );
    let adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        transport.clone(),
    );
    let gallery_id = GalleryId::new(7_001).unwrap();

    let summary = adapter
        .gallery_summary_get(gallery_id)
        .expect("load main summary without fetching related galleries")
        .expect("main summary exists");
    assert_eq!(summary.title, "Main summary fixture");
    assert_eq!(summary.pages, 2);
    assert_eq!(
        summary.tags,
        vec!["landscape", "female:blue_sky", "daylight"]
    );
    assert_eq!(
        (summary.thumbnail_width, summary.thumbnail_height),
        (1200, 1600)
    );
    assert!(summary.thumbnail_key.is_some());
    assert_eq!(
        adapter.gallery_summary_get(gallery_id).unwrap(),
        Some(summary),
    );
    let snapshot = adapter
        .gallery_snapshot(gallery_id, &CancellationToken::new())
        .expect("download source also reuses the warmed metadata");
    assert_eq!(snapshot.pages.len(), 2);
    assert_eq!(transport.call_count(&main_url), 1);
    assert_eq!(transport.call_count(&related_url), 0);
    assert_eq!(transport.calls.lock().unwrap().len(), 1);
}

#[test]
fn persisted_gallery_summaries_survive_restart_with_empty_tags_and_no_network() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("summary-cache.sqlite");
    let loaded = {
        let repository = Arc::new(SqliteRepository::open(&path).unwrap());
        let transport = Arc::new(FakeTransport::default());
        transport.respond(
            galleryinfo_script_url(7_001).unwrap(),
            "text/javascript",
            gallery_script(7_001, "Persisted tags", "[7002]").into_bytes(),
        );
        transport.respond(
            galleryinfo_script_url(7_003).unwrap(),
            "text/javascript",
            gallery_script(7_003, "Persisted empty tags", "[7002]")
                .replace("\"tags\": [", "\"unused_tags\": [")
                .into_bytes(),
        );
        let adapter =
            HitomiLiveAdapter::with_transport(HitomiLiveConfig::default(), transport.clone())
                .with_summary_cache(repository.clone());
        let loaded = [7_001, 7_003].map(|id| {
            adapter
                .gallery_summary_get(GalleryId::new(id).unwrap())
                .unwrap()
                .unwrap()
        });
        assert!(!loaded[0].tags.is_empty());
        assert!(loaded[1].tags.is_empty());
        assert_eq!(transport.calls.lock().unwrap().len(), 2);
        assert_eq!(
            transport.call_count(&galleryinfo_script_url(7_002).unwrap()),
            0
        );
        repository
            .connection()
            .unwrap()
            .execute(
                "UPDATE gallery_summary_cache SET updated_at = '1999-01-01T00:00:00Z'",
                [],
            )
            .unwrap();
        loaded
    };
    let offline = Arc::new(FakeTransport::default());
    let repository = Arc::new(SqliteRepository::open(&path).unwrap());
    let restarted = HitomiLiveAdapter::with_transport(HitomiLiveConfig::default(), offline.clone())
        .with_summary_cache(repository);
    for summary in loaded {
        assert_eq!(
            restarted.gallery_summary_get(summary.id).unwrap(),
            Some(summary)
        );
    }
    assert!(offline.calls.lock().unwrap().is_empty());
}

#[test]
fn download_resolution_refreshes_persisted_summary_and_never_uses_it_for_pages() {
    let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
    let gallery_id = GalleryId::new(7_001).unwrap();
    let initial = super::search::gallery_summary(
        &parse_galleryinfo_script(&gallery_script(7_001, "Old summary", "[]")).unwrap(),
        SearchSort::Recent,
        0,
    )
    .unwrap();
    repository.gallery_summary_cache_put(&initial).unwrap();
    let offline = Arc::new(FakeTransport::default());
    let restarted = HitomiLiveAdapter::with_transport(HitomiLiveConfig::default(), offline.clone())
        .with_summary_cache(repository.clone());
    assert_eq!(
        restarted.gallery_summary_get(gallery_id).unwrap(),
        Some(initial.clone())
    );
    assert!(restarted
        .gallery_snapshot(gallery_id, &CancellationToken::new())
        .is_err());
    assert_eq!(offline.calls.lock().unwrap().len(), 1);

    let transport = Arc::new(FakeTransport::default());
    let main_url = galleryinfo_script_url(7_001).unwrap();
    transport.respond(
        main_url.clone(),
        "text/javascript",
        gallery_script(7_001, "New download metadata", "[7002]")
            .replace("blue_sky", "updated_tag")
            .into_bytes(),
    );
    let downloader =
        HitomiLiveAdapter::with_transport(HitomiLiveConfig::default(), transport.clone())
            .with_summary_cache(repository.clone());
    assert_eq!(
        downloader.gallery_summary_get(gallery_id).unwrap(),
        Some(initial)
    );
    let snapshot = downloader
        .gallery_snapshot(gallery_id, &CancellationToken::new())
        .unwrap();
    assert_eq!(snapshot.pages.len(), 2);
    let updated = downloader.gallery_summary_get(gallery_id).unwrap().unwrap();
    assert_eq!(updated.title, "New download metadata");
    assert!(updated.tags.contains(&"female:updated_tag".to_owned()));
    assert_eq!(
        repository.gallery_summary_cache_get(gallery_id).unwrap(),
        Some(updated)
    );
    assert_eq!(transport.calls.lock().unwrap().len(), 1);
    assert_eq!(transport.call_count(&main_url), 1);
}

#[test]
fn corrupt_or_incompatible_persisted_gallery_summary_is_refetched_and_repaired() {
    let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
    let gallery_id = GalleryId::new(7_001).unwrap();
    let expected = super::search::gallery_summary(
        &parse_galleryinfo_script(&gallery_script(7_001, "Recovered summary", "[7002]")).unwrap(),
        SearchSort::Recent,
        0,
    )
    .unwrap();
    let valid_json = serde_json::to_string(&expected).unwrap();
    let mismatched_id = valid_json.replace("\"id\":7001", "\"id\":7002");
    for (profile, version, json) in [
        ("hitomi-gallery-summary-v1", 1, "{broken"),
        ("hitomi-gallery-summary-v1", 1, "{}"),
        ("hitomi-gallery-summary-v1", 1, mismatched_id.as_str()),
        ("hitomi-gallery-summary-v1", 2, valid_json.as_str()),
        ("obsolete-profile", 1, valid_json.as_str()),
    ] {
        repository.connection().unwrap().execute(
            "INSERT OR REPLACE INTO gallery_summary_cache (gallery_id, profile, schema_version, summary_json) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![gallery_id.get(), profile, version, json],
        ).unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.respond(
            galleryinfo_script_url(7_001).unwrap(),
            "text/javascript",
            gallery_script(7_001, "Recovered summary", "[7002]").into_bytes(),
        );
        let adapter =
            HitomiLiveAdapter::with_transport(HitomiLiveConfig::default(), transport.clone())
                .with_summary_cache(repository.clone());
        assert_eq!(
            adapter.gallery_summary_get(gallery_id).unwrap(),
            Some(expected.clone())
        );
        assert_eq!(
            adapter.gallery_summary_get(gallery_id).unwrap(),
            Some(expected.clone())
        );
        assert_eq!(
            repository.gallery_summary_cache_get(gallery_id).unwrap(),
            Some(expected.clone())
        );
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
    }
}

#[test]
fn clearing_persisted_gallery_summary_requires_a_new_main_metadata_fetch() {
    let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
    let transport = Arc::new(FakeTransport::default());
    let main_url = galleryinfo_script_url(7_001).unwrap();
    for title in ["Before clear", "After clear"] {
        transport.respond(
            main_url.clone(),
            "text/javascript",
            gallery_script(7_001, title, "[]").into_bytes(),
        );
    }
    let adapter = HitomiLiveAdapter::with_transport(HitomiLiveConfig::default(), transport.clone())
        .with_summary_cache(repository.clone());
    let gallery_id = GalleryId::new(7_001).unwrap();
    assert_eq!(
        adapter
            .gallery_summary_get(gallery_id)
            .unwrap()
            .unwrap()
            .title,
        "Before clear"
    );
    adapter.clear_derived_caches();
    assert!(repository
        .gallery_summary_cache_get(gallery_id)
        .unwrap()
        .is_none());
    assert_eq!(
        adapter
            .gallery_summary_get(gallery_id)
            .unwrap()
            .unwrap()
            .title,
        "After clear"
    );
    assert_eq!(transport.call_count(&main_url), 2);
}

#[test]
fn clearing_persisted_gallery_summary_rejects_inflight_metadata_repopulation() {
    struct BlockingTransport {
        started: std::sync::mpsc::Sender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl HttpTransport for BlockingTransport {
        fn execute(&self, _: HttpRequest) -> Result<HttpPayload, SourceContractError> {
            self.started.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            Ok(HttpPayload {
                status: 200,
                content_type: "text/javascript".into(),
                bytes: gallery_script(7_001, "In-flight old summary", "[]").into_bytes(),
            })
        }
    }
    let (started, waiting) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
    let adapter = Arc::new(
        HitomiLiveAdapter::with_transport(
            HitomiLiveConfig::default(),
            Arc::new(BlockingTransport {
                started,
                release: Mutex::new(released),
            }),
        )
        .with_summary_cache(repository.clone()),
    );
    let worker_adapter = adapter.clone();
    let gallery_id = GalleryId::new(7_001).unwrap();
    let worker = std::thread::spawn(move || worker_adapter.gallery_summary_get(gallery_id));
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    adapter.clear_derived_caches();
    release.send(()).unwrap();
    assert!(worker.join().unwrap().unwrap().is_some());
    assert!(repository
        .gallery_summary_cache_get(gallery_id)
        .unwrap()
        .is_none());
    assert!(adapter
        .metadata_cache
        .lock()
        .unwrap()
        .get_fresh(&7_001, Duration::from_secs(60))
        .is_none());
}

#[test]
fn persisted_gallery_summary_cache_failure_does_not_fail_download_resolution() {
    struct BrokenCache;
    impl GallerySummaryCache for BrokenCache {
        fn gallery_summary_cache_get(
            &self,
            _: GalleryId,
        ) -> Result<Option<crate::domain::GallerySummary>, RepositoryError> {
            Err(RepositoryError::Other("cache read unavailable".into()))
        }
        fn gallery_summary_cache_put(
            &self,
            _: &crate::domain::GallerySummary,
        ) -> Result<(), RepositoryError> {
            Err(RepositoryError::Other("cache write unavailable".into()))
        }
        fn gallery_summary_cache_clear(&self) -> Result<u64, RepositoryError> {
            Err(RepositoryError::Other("cache clear unavailable".into()))
        }
    }
    let transport = Arc::new(FakeTransport::default());
    transport.respond(
        galleryinfo_script_url(7_001).unwrap(),
        "text/javascript",
        gallery_script(7_001, "Cache-independent download", "[]").into_bytes(),
    );
    let adapter = HitomiLiveAdapter::with_transport(HitomiLiveConfig::default(), transport.clone())
        .with_summary_cache(Arc::new(BrokenCache));
    let gallery_id = GalleryId::new(7_001).unwrap();
    assert!(adapter.gallery_summary_get(gallery_id).unwrap().is_some());
    assert_eq!(
        adapter
            .gallery_snapshot(gallery_id, &CancellationToken::new())
            .unwrap()
            .pages
            .len(),
        2
    );
    assert_eq!(transport.calls.lock().unwrap().len(), 1);
}

#[test]
fn detail_keeps_main_page_dimensions_when_related_metadata_is_temporarily_unavailable() {
    let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
    let transport = Arc::new(FakeTransport::default());
    transport.respond(
        galleryinfo_script_url(7_001).unwrap(),
        "text/javascript",
        gallery_script(7_001, "Main detail fixture", "[7002]").into_bytes(),
    );
    transport.fail(
        galleryinfo_script_url(7_002).unwrap(),
        crate::source::map_http_status(503, None).unwrap_err(),
    );
    let adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        transport,
    )
    .with_summary_cache(repository.clone());

    let detail = adapter
        .gallery_detail_get(GalleryId::new(7_001).unwrap())
        .expect("related failure is supplemental")
        .expect("main detail exists");

    assert_eq!(detail.summary.title, "Main detail fixture");
    assert!(!detail.page_dimensions.is_empty());
    assert!(detail.related.is_empty());
    assert_eq!(
        repository
            .gallery_summary_cache_get(GalleryId::new(7_001).unwrap())
            .unwrap(),
        Some(detail.summary)
    );
    assert!(repository
        .gallery_summary_cache_get(GalleryId::new(7_002).unwrap())
        .unwrap()
        .is_none());
}

impl HttpTransport for FakeTransport {
    fn execute(&self, request: HttpRequest) -> Result<HttpPayload, SourceContractError> {
        self.calls.lock().unwrap().push(request.url.clone());
        self.requests
            .lock()
            .unwrap()
            .push((request.url.clone(), request.range.clone()));
        self.responses
            .lock()
            .unwrap()
            .get_mut(&request.url)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| {
                Err(SourceContractError::not_found(
                    format!("fake response for {}", request.url),
                    Some(404),
                ))
            })
    }
}

#[test]
fn source_allowlist_rejects_lookalike_and_plain_http_hosts() {
    assert!(validate_source_url(
        &Url::parse("https://w1.gold-usergeneratedcontent.net/path.webp").unwrap()
    )
    .is_ok());
    assert!(validate_source_url(
        &Url::parse("https://gold-usergeneratedcontent.net.attacker.invalid/path").unwrap()
    )
    .is_err());
    assert!(validate_source_url(
        &Url::parse("http://ltn.gold-usergeneratedcontent.net/index-all.nozomi").unwrap()
    )
    .is_err());
}

#[test]
fn structured_tag_paths_preserve_hitomi_gender_namespace() {
    assert_eq!(
        tag_nozomi_path("female:long_hair").as_deref(),
        Some("n/tag/female%3Along%20hair-all.nozomi")
    );
    assert_eq!(
        tag_nozomi_path("full color").as_deref(),
        Some("n/tag/full%20color-all.nozomi")
    );
    assert_eq!(
        prefixed_nozomi_path("series:rain_archives").as_deref(),
        Some("n/series/rain%20archives-all.nozomi")
    );
    assert_eq!(
        prefixed_nozomi_path("character:mira_lane").as_deref(),
        Some("n/character/mira%20lane-all.nozomi")
    );
    assert_eq!(
        prefixed_nozomi_path("artist:sugoi_hi").as_deref(),
        Some("n/artist/sugoi%20hi-all.nozomi")
    );
    assert_eq!(
        prefixed_nozomi_path("artist:sugoi\\_hi").as_deref(),
        Some("n/artist/sugoi%20hi-all.nozomi")
    );
}

#[test]
fn structured_artist_search_accepts_canonical_and_escaped_underscores() {
    let transport = Arc::new(FakeTransport::default());
    let origin = HITOMI_METADATA_ORIGIN;
    for _ in 0..2 {
        transport.respond(
            format!("{origin}/n/index-korean.nozomi"),
            "application/x-nozomi",
            nozomi(&[1001]),
        );
        transport.respond(
            format!("{origin}/n/artist/sugoi%20hi-all.nozomi"),
            "application/x-nozomi",
            nozomi(&[1001]),
        );
    }
    transport.respond(
        galleryinfo_script_url(1001).unwrap(),
        "text/javascript",
        gallery_script(1001, "Sugoi Hi Fixture", "[]").into_bytes(),
    );
    let adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        transport,
    );

    for text in ["artist:sugoi_hi", "artist:sugoi\\_hi"] {
        let result = adapter
            .search_submit(&SearchRequest {
                text: text.into(),
                include_tags: Vec::new(),
                exclude_tags: Vec::new(),
                languages: vec![Language::Korean],
                sort: SearchSort::Recent,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(result.first_page.items[0].id.get(), 1001);
    }
}

#[test]
fn structured_search_intersects_artist_and_gender_tag_indexes() {
    let transport = Arc::new(FakeTransport::default());
    let origin = HITOMI_METADATA_ORIGIN;
    transport.respond(
        format!("{origin}/n/index-korean.nozomi"),
        "application/x-nozomi",
        nozomi(&[1001, 1002]),
    );
    transport.respond(
        format!("{origin}/n/artist/healthyman-all.nozomi"),
        "application/x-nozomi",
        nozomi(&[1001, 1002]),
    );
    transport.respond(
        format!("{origin}/n/tag/female%3Aahegao-all.nozomi"),
        "application/x-nozomi",
        nozomi(&[1001]),
    );
    transport.respond(
        galleryinfo_script_url(1001).unwrap(),
        "text/javascript",
        gallery_script(1001, "Healthyman Fixture", "[]").into_bytes(),
    );
    let adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        transport,
    );

    let result = adapter
        .search_submit(&SearchRequest {
            text: "artist:healthyman female:ahegao".into(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: vec![Language::Korean],
            sort: SearchSort::Recent,
            page_size: 20,
        })
        .unwrap();

    assert_eq!(
        result
            .first_page
            .items
            .iter()
            .map(|gallery| gallery.id.get())
            .collect::<Vec<_>>(),
        vec![1001]
    );
}

#[test]
fn korean_title_search_uses_galleries_index_ranges_before_fetching_metadata() {
    let transport = Arc::new(FakeTransport::default());
    let origin = HITOMI_METADATA_ORIGIN;
    let version = "2026090801";
    let version_url = galleries_index_version_url();
    let index_url = galleries_index_file_url(version, "index").unwrap();
    let data_url = galleries_index_file_url(version, "data").unwrap();
    let title_data = gallery_index_data(&[1001, 1003]);

    transport.respond(
        format!("{origin}/n/index-korean.nozomi"),
        "application/x-nozomi",
        nozomi(&[1001, 1002, 1003]),
    );
    transport.respond(
        version_url.clone(),
        "text/plain",
        format!("{version}\n").into_bytes(),
    );
    transport.respond(
        index_url.clone(),
        "application/octet-stream",
        gallery_index_node(&[("줄곧", 1_200, title_data.len() as u32)]),
    );
    transport.respond(data_url.clone(), "application/octet-stream", title_data);
    transport.respond(
        galleryinfo_script_url(1003).unwrap(),
        "text/javascript",
        gallery_script(1003, "줄곧 이어지는 기록", "[]").into_bytes(),
    );
    let adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        transport.clone(),
    );

    let result = adapter
        .search_submit(&SearchRequest {
            text: "줄곧".into(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: vec![Language::Korean],
            sort: SearchSort::Recent,
            page_size: 1,
        })
        .unwrap();

    assert_eq!(result.first_page.items[0].id.get(), 1003);
    assert_eq!(transport.ranges_for(&version_url), vec![None]);
    assert_eq!(
        transport.ranges_for(&index_url),
        vec![Some("bytes=0-463".into())]
    );
    assert_eq!(
        transport.ranges_for(&data_url),
        vec![Some("bytes=1200-1211".into())]
    );
    assert_eq!(
        transport.call_count(&galleryinfo_script_url(1002).unwrap()),
        0,
        "a gallery outside the title index result must not be metadata-scanned"
    );
    assert_eq!(
        transport.call_count(&galleryinfo_script_url(1001).unwrap()),
        0,
        "the first page must fetch only the first indexed result"
    );
}

#[test]
fn title_search_intersects_multiple_positive_terms_and_subtracts_negative_terms() {
    let transport = Arc::new(FakeTransport::default());
    let origin = HITOMI_METADATA_ORIGIN;
    let version = "multi-term-v1";
    let index_url = galleries_index_file_url(version, "index").unwrap();
    let data_url = galleries_index_file_url(version, "data").unwrap();
    let first = gallery_index_data(&[10, 20, 30]);
    let second = gallery_index_data(&[20, 30, 40]);
    let excluded = gallery_index_data(&[30]);

    transport.respond(
        format!("{origin}/n/index-korean.nozomi"),
        "application/x-nozomi",
        nozomi(&[10, 20, 30, 40]),
    );
    transport.respond(
        galleries_index_version_url(),
        "text/plain",
        version.as_bytes().to_vec(),
    );
    transport.respond(
        index_url.clone(),
        "application/octet-stream",
        gallery_index_node(&[
            ("푸른", 100, first.len() as u32),
            ("줄곧", 200, second.len() as u32),
            ("비밀", 300, excluded.len() as u32),
        ]),
    );
    transport.respond(data_url.clone(), "application/octet-stream", first);
    transport.respond(data_url.clone(), "application/octet-stream", second);
    transport.respond(data_url.clone(), "application/octet-stream", excluded);
    transport.respond(
        galleryinfo_script_url(20).unwrap(),
        "text/javascript",
        gallery_script(20, "푸른 줄곧 기록", "[]").into_bytes(),
    );
    let adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        transport.clone(),
    );

    let result = adapter
        .search_submit(&SearchRequest {
            text: "푸른 줄곧 -비밀".into(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: vec![Language::Korean],
            sort: SearchSort::Recent,
            page_size: 20,
        })
        .unwrap();

    assert_eq!(
        result
            .first_page
            .items
            .iter()
            .map(|gallery| gallery.id.get())
            .collect::<Vec<_>>(),
        vec![20]
    );
    assert_eq!(
        transport.call_count(&index_url),
        1,
        "all terms in one search must share the fetched root node"
    );
    assert_eq!(
        transport.ranges_for(&data_url),
        vec![
            Some("bytes=100-115".into()),
            Some("bytes=200-215".into()),
            Some("bytes=300-307".into()),
        ]
    );
    for gallery_id in [10, 30, 40] {
        assert_eq!(
            transport.call_count(&galleryinfo_script_url(gallery_id).unwrap()),
            0,
            "non-matching gallery {gallery_id} must not be metadata-scanned"
        );
    }
}

#[test]
fn missing_or_invalid_title_index_never_falls_back_to_metadata_scanning() {
    let origin = HITOMI_METADATA_ORIGIN;
    let version = "missing-v1";
    let version_url = galleries_index_version_url();
    let index_url = galleries_index_file_url(version, "index").unwrap();

    let missing = Arc::new(FakeTransport::default());
    missing.respond(
        format!("{origin}/n/index-korean.nozomi"),
        "application/x-nozomi",
        nozomi(&[1001, 1002]),
    );
    missing.respond(
        version_url.clone(),
        "text/plain",
        version.as_bytes().to_vec(),
    );
    missing.respond(
        index_url.clone(),
        "application/octet-stream",
        gallery_index_node(&[("다른", 100, 8)]),
    );
    let missing_adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        missing.clone(),
    );
    let result = missing_adapter
        .search_submit(&SearchRequest {
            text: "줄곧".into(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: vec![Language::Korean],
            sort: SearchSort::Recent,
            page_size: 20,
        })
        .unwrap();
    assert!(result.first_page.items.is_empty());
    assert_eq!(result.first_page.total_pages, 0);
    for gallery_id in [1001, 1002] {
        assert_eq!(
            missing.call_count(&galleryinfo_script_url(gallery_id).unwrap()),
            0
        );
    }

    let invalid = Arc::new(FakeTransport::default());
    invalid.respond(
        format!("{origin}/n/index-korean.nozomi"),
        "application/x-nozomi",
        nozomi(&[1001]),
    );
    invalid.respond(version_url, "text/plain", version.as_bytes().to_vec());
    invalid.respond(index_url, "application/octet-stream", vec![0; 4]);
    let invalid_adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        invalid.clone(),
    );
    let error = invalid_adapter
        .search_submit(&SearchRequest {
            text: "줄곧".into(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: vec![Language::Korean],
            sort: SearchSort::Recent,
            page_size: 20,
        })
        .unwrap_err();
    assert!(matches!(
        error,
        RepositoryError::Source(error) if error.code == SourceErrorCode::InvalidData
    ));
    assert_eq!(
        invalid.call_count(&galleryinfo_script_url(1001).unwrap()),
        0
    );
}

#[test]
fn exact_seven_digit_search_bypasses_language_index_and_honors_global_tags() {
    let transport = Arc::new(FakeTransport::default());
    let origin = HITOMI_METADATA_ORIGIN;
    transport.respond(
        format!("{origin}/n/tag/landscape-all.nozomi"),
        "application/x-nozomi",
        nozomi(&[4_051_038]),
    );
    transport.respond(
        format!("{origin}/n/tag/female%3Aahegao-all.nozomi"),
        "application/x-nozomi",
        nozomi(&[]),
    );
    transport.respond(
        galleryinfo_script_url(4_051_038).unwrap(),
        "text/javascript",
        gallery_script(4_051_038, "Direct ID Fixture", "[]").into_bytes(),
    );
    let adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        transport.clone(),
    );

    let result = adapter
        .search_submit(&SearchRequest {
            text: "4051038".into(),
            include_tags: vec!["landscape".into()],
            exclude_tags: vec!["female:ahegao".into()],
            languages: vec![Language::Japanese],
            sort: SearchSort::PopularWeek,
            page_size: 20,
        })
        .unwrap();

    assert_eq!(result.first_page.items[0].id.get(), 4_051_038);
    assert!(!transport.was_called(&format!(
        "{}/n/index-japanese.nozomi",
        HITOMI_METADATA_ORIGIN
    )));
    assert!(transport.was_called(&format!("{origin}/n/tag/landscape-all.nozomi")));
    assert!(transport.was_called(&format!("{origin}/n/tag/female%3Aahegao-all.nozomi")));
}

#[test]
fn auto_find_filters_nozomi_ids_before_metadata_and_reports_the_bounded_plan() {
    let transport = Arc::new(FakeTransport::default());
    let origin = HITOMI_METADATA_ORIGIN;
    transport.respond(
        format!("{origin}/n/artist/serein-all.nozomi"),
        "application/x-nozomi",
        nozomi(&[90, 200, 300, 400]),
    );
    transport.respond(
        format!("{origin}/n/index-english.nozomi"),
        "application/x-nozomi",
        nozomi(&[90, 200, 300]),
    );
    let selected_url = galleryinfo_script_url(300).unwrap();
    transport.respond(
        selected_url.clone(),
        "text/javascript",
        gallery_script(300, "Newest", "[]").into_bytes(),
    );
    let adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        transport.clone(),
    );
    let cancellation = CancellationToken::new();
    let plan = adapter
        .auto_find_artist_plan(
            &AutoFindSourceRequest {
                artist: "serein".into(),
                languages: vec![Language::English],
                retain_after_gallery_id: None,
                newer_than_gallery_id: Some(GalleryId::new(100).unwrap()),
                candidate_limit: 1,
            },
            &cancellation,
        )
        .unwrap();

    assert_eq!(plan.candidate_ids, vec![GalleryId::new(300).unwrap()]);
    assert_eq!(
        plan.matching_ids,
        vec![
            GalleryId::new(300).unwrap(),
            GalleryId::new(200).unwrap(),
            GalleryId::new(90).unwrap()
        ]
    );
    assert_eq!(
        plan.latest_available_gallery_id,
        Some(GalleryId::new(300).unwrap())
    );
    assert_eq!(plan.eligible_count, 2);
    assert_eq!(
        plan.truncated_reason.as_deref(),
        Some("candidate_limit_after_cutoff")
    );
    assert_eq!(
        transport.call_count(&selected_url),
        0,
        "plan must not fetch metadata"
    );
    assert_eq!(
        adapter
            .auto_find_gallery_summary(GalleryId::new(300).unwrap(), &cancellation)
            .unwrap()
            .unwrap()
            .id
            .get(),
        300
    );
    assert_eq!(transport.call_count(&selected_url), 1);
}

#[test]
fn search_and_thumbnail_share_the_same_metadata_cache_without_live_network() {
    let transport = Arc::new(FakeTransport::default());
    let nozomi = 424_242_u32.to_be_bytes().to_vec();
    transport.respond(
        format!("{HITOMI_METADATA_ORIGIN}/n/index-english.nozomi"),
        "application/x-nozomi",
        nozomi.clone(),
    );
    transport.respond(
        format!("{HITOMI_METADATA_ORIGIN}/n/tag/landscape-all.nozomi"),
        "application/x-nozomi",
        nozomi,
    );
    let gallery_url = galleryinfo_script_url(424_242).unwrap();
    transport.respond(
        gallery_url.clone(),
        "text/javascript",
        GALLERY_SCRIPT.as_bytes().to_vec(),
    );
    transport.respond(
        gg_script_url(),
        "text/javascript",
        GG_SCRIPT.as_bytes().to_vec(),
    );
    let metadata = parse_galleryinfo_script(GALLERY_SCRIPT).unwrap();
    let routing = parse_gg_routing(GG_SCRIPT).unwrap();
    let candidate = webp_thumbnail_candidates(
        metadata.pages.first().unwrap(),
        &routing,
        ThumbnailSize::Large,
    )
    .unwrap()
    .remove(0);
    transport.respond(candidate.url, "image/png", one_pixel_png());

    let config = HitomiLiveConfig {
        request_start_interval: Duration::ZERO,
        ..HitomiLiveConfig::default()
    };
    let adapter = HitomiLiveAdapter::with_transport(config, transport.clone());
    let submission = adapter
        .search_submit(&SearchRequest {
            text: String::new(),
            include_tags: vec!["landscape".to_owned()],
            exclude_tags: Vec::new(),
            languages: vec![Language::English],
            sort: SearchSort::Recent,
            page_size: 20,
        })
        .unwrap();
    assert_eq!(submission.first_page.items.len(), 1);
    assert_eq!(submission.first_page.items[0].series, vec!["original"]);
    assert_eq!(
        submission.first_page.items[0].characters,
        vec!["Example Character"]
    );

    let thumbnail = adapter
        .resolve(
            &ThumbnailKey::gallery_cover(424_242).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!((thumbnail.width, thumbnail.height), (1, 1));
    assert_eq!(thumbnail.content_type, "image/png");
    assert_eq!(transport.call_count(&gallery_url), 1);
}

#[test]
fn thumbnail_falls_back_to_avif_when_webp_derivatives_are_missing() {
    let transport = Arc::new(FakeTransport::default());
    let metadata = parse_galleryinfo_script(GALLERY_SCRIPT).unwrap();
    let routing = parse_gg_routing(GG_SCRIPT).unwrap();
    let avif = download_full_candidates(metadata.pages.first().unwrap(), &routing)
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.format == crate::source::hitomi::HitomiImageFormat::Avif)
        .expect("fixture has an AVIF fallback");
    transport.respond(
        galleryinfo_script_url(424_242).unwrap(),
        "text/javascript",
        GALLERY_SCRIPT.as_bytes().to_vec(),
    );
    transport.respond(
        gg_script_url(),
        "text/javascript",
        GG_SCRIPT.as_bytes().to_vec(),
    );
    // The fake payload is PNG so this test stays independent of the AVIF codec;
    // it verifies resolver candidate fallback after every WebP endpoint misses.
    transport.respond(avif.url.clone(), "image/png", one_pixel_png());
    let adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        transport.clone(),
    );

    let resolved = adapter
        .resolve(
            &ThumbnailKey::gallery_page(424_242, 1).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap();

    assert_eq!(resolved.content_type, "image/png");
    assert!(transport.was_called(&avif.url));
}

#[test]
fn gallery_page_thumbnail_uses_alternate_full_webp_after_derivatives_miss() {
    let transport = Arc::new(FakeTransport::default());
    let metadata = parse_galleryinfo_script(GALLERY_SCRIPT).unwrap();
    let routing = parse_gg_routing(GG_SCRIPT).unwrap();
    let page = metadata.pages.first().unwrap();
    let derivatives = webp_thumbnail_candidates(page, &routing, ThumbnailSize::Large).unwrap();
    let full_webp = webp_full_candidates(page, &routing).unwrap();
    let alternate = full_webp
        .iter()
        .rev()
        .find(|candidate| {
            derivatives
                .iter()
                .all(|derivative| derivative.url != candidate.url)
        })
        .expect("fixture has an alternate w1/w2 WebP route");

    transport.respond(
        galleryinfo_script_url(424_242).unwrap(),
        "text/javascript",
        GALLERY_SCRIPT.as_bytes().to_vec(),
    );
    transport.respond(
        gg_script_url(),
        "text/javascript",
        GG_SCRIPT.as_bytes().to_vec(),
    );
    // Every preceding derivative/full route is an implicit deterministic 404
    // in FakeTransport. Only the final alternate endpoint succeeds.
    transport.respond(
        alternate.url.clone(),
        "image/png",
        fallback_source_page_png(),
    );
    let adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        transport.clone(),
    );

    let resolved = adapter
        .resolve(
            &ThumbnailKey::gallery_page(424_242, 1).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap();

    assert_eq!(resolved.content_type, "image/webp");
    assert_eq!((resolved.width, resolved.height), (768, 384));
    assert_eq!(
        image::guess_format(&resolved.bytes).unwrap(),
        image::ImageFormat::WebP
    );
    assert!(transport.was_called(&alternate.url));
    assert!(derivatives
        .iter()
        .all(|candidate| transport.was_called(&candidate.url)));
    for candidate in full_webp
        .iter()
        .filter(|candidate| derivatives.iter().any(|item| item.url == candidate.url))
    {
        assert_eq!(
            transport.call_count(&candidate.url),
            1,
            "overlapping primary WebP routes must be deduplicated"
        );
    }
}

#[test]
fn gallery_cover_does_not_use_alternate_full_webp_routes() {
    let transport = Arc::new(FakeTransport::default());
    let metadata = parse_galleryinfo_script(GALLERY_SCRIPT).unwrap();
    let routing = parse_gg_routing(GG_SCRIPT).unwrap();
    let page = metadata.pages.first().unwrap();
    let derivatives = webp_thumbnail_candidates(page, &routing, ThumbnailSize::Large).unwrap();
    let alternate = webp_full_candidates(page, &routing)
        .unwrap()
        .into_iter()
        .find(|candidate| {
            derivatives
                .iter()
                .all(|derivative| derivative.url != candidate.url)
        })
        .expect("fixture has an alternate w1/w2 WebP route");

    transport.respond(
        galleryinfo_script_url(424_242).unwrap(),
        "text/javascript",
        GALLERY_SCRIPT.as_bytes().to_vec(),
    );
    transport.respond(
        gg_script_url(),
        "text/javascript",
        GG_SCRIPT.as_bytes().to_vec(),
    );
    transport.respond(alternate.url.clone(), "image/png", one_pixel_png());
    let adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        transport.clone(),
    );

    adapter
        .resolve(
            &ThumbnailKey::gallery_cover(424_242).unwrap(),
            &CancellationToken::new(),
        )
        .expect_err("cover must not widen into full WebP endpoints");

    assert!(!transport.was_called(&alternate.url));
}

#[test]
fn live_search_contract_covers_paging_filters_popular_and_related_without_network() {
    let transport = Arc::new(FakeTransport::default());
    let origin = HITOMI_METADATA_ORIGIN;
    transport.respond(
        format!("{origin}/n/index-english.nozomi"),
        "application/x-nozomi",
        nozomi(&[1001, 1002, 1003]),
    );
    transport.respond(
        format!("{origin}/n/index-english.nozomi"),
        "application/x-nozomi",
        nozomi(&[1001, 1002, 1003]),
    );
    transport.respond(
        format!("{origin}/n/tag/landscape-all.nozomi"),
        "application/x-nozomi",
        nozomi(&[1002, 1003]),
    );
    transport.respond(
        format!("{origin}/n/tag/female%3Ablue%20sky-all.nozomi"),
        "application/x-nozomi",
        nozomi(&[1002]),
    );
    let title_index_version = "fixture-v1";
    let title_data = gallery_index_data(&[1003]);
    transport.respond(
        galleries_index_version_url(),
        "text/plain",
        title_index_version.as_bytes().to_vec(),
    );
    transport.respond(
        galleries_index_file_url(title_index_version, "index").unwrap(),
        "application/octet-stream",
        gallery_index_node(&[("sunlit", 80, title_data.len() as u32)]),
    );
    transport.respond(
        galleries_index_file_url(title_index_version, "data").unwrap(),
        "application/octet-stream",
        title_data,
    );
    transport.respond(
        format!("{origin}/n/popular/week-english.nozomi"),
        "application/x-nozomi",
        nozomi(&[1001, 1003]),
    );

    for (id, title, related) in [
        (1001, "Quiet Night Fixture", "[]"),
        (1002, "Excluded Blue Fixture", "[]"),
        (1003, "Sunlit Archive Fixture", "[1002, 1999]"),
    ] {
        transport.respond(
            galleryinfo_script_url(id).unwrap(),
            "text/javascript",
            gallery_script(id, title, related).into_bytes(),
        );
    }

    let adapter = HitomiLiveAdapter::with_transport(
        HitomiLiveConfig {
            request_start_interval: Duration::ZERO,
            ..HitomiLiveConfig::default()
        },
        transport.clone(),
    );

    let recent = adapter
        .search_submit(&SearchRequest {
            text: String::new(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: vec![Language::English],
            sort: SearchSort::Recent,
            page_size: 1,
        })
        .unwrap();
    assert_eq!(recent.first_page.total_pages, 3);
    assert_eq!(recent.first_page.items[0].id.get(), 1003);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let cancelled_error = adapter
        .search_page_get_cancellable(&recent.query_id, 2, &cancelled)
        .unwrap_err();
    assert!(matches!(
        cancelled_error,
        RepositoryError::Source(error) if error.code == SourceErrorCode::Cancelled
    ));
    assert_eq!(
        transport.call_count(&galleryinfo_script_url(1002).unwrap()),
        0,
        "a cancelled page request must not fetch metadata"
    );
    let second = adapter
        .search_page_get(&recent.query_id, 2)
        .unwrap()
        .expect("cached query exists");
    assert_eq!(second.items[0].id.get(), 1002);

    let filtered = adapter
        .search_submit(&SearchRequest {
            text: "Sunlit".into(),
            include_tags: vec!["landscape".into()],
            exclude_tags: vec!["female:blue_sky".into()],
            languages: vec![Language::English],
            sort: SearchSort::Recent,
            page_size: 20,
        })
        .unwrap();
    assert_eq!(
        filtered
            .first_page
            .items
            .iter()
            .map(|gallery| gallery.id.get())
            .collect::<Vec<_>>(),
        vec![1003]
    );
    assert!(!transport.was_called(&format!("{origin}/n/index-korean.nozomi")));

    let popular = adapter
        .search_submit(&SearchRequest {
            text: String::new(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: vec![Language::English],
            sort: SearchSort::PopularWeek,
            page_size: 20,
        })
        .unwrap();
    assert_eq!(
        popular
            .first_page
            .items
            .iter()
            .map(|gallery| gallery.id.get())
            .collect::<Vec<_>>(),
        vec![1001, 1003]
    );
    assert!(popular.first_page.items[0].popularity > popular.first_page.items[1].popularity);

    let detail = adapter
        .gallery_detail_get(crate::domain::GalleryId::new(1003).unwrap())
        .unwrap()
        .expect("detail exists");
    assert_eq!(detail.summary.title, "Sunlit Archive Fixture");
    assert_eq!(detail.summary.series, vec!["original"]);
    assert_eq!(detail.summary.characters, vec!["Example Character"]);
    assert_eq!(
        detail
            .related
            .iter()
            .map(|gallery| gallery.id.get())
            .collect::<Vec<_>>(),
        vec![1002]
    );
}

fn nozomi(ids: &[u32]) -> Vec<u8> {
    ids.iter().flat_map(|id| id.to_be_bytes()).collect()
}

fn gallery_index_data(ids: &[u32]) -> Vec<u8> {
    let mut data = Vec::with_capacity(4 + ids.len() * 4);
    data.extend_from_slice(&(ids.len() as u32).to_be_bytes());
    data.extend(ids.iter().flat_map(|id| id.to_be_bytes()));
    data
}

fn gallery_index_node(entries: &[(&str, u64, u32)]) -> Vec<u8> {
    let mut entries = entries
        .iter()
        .map(|(term, offset, length)| (gallery_index_term_key(term), *offset, *length))
        .collect::<Vec<_>>();
    entries.sort_by_key(|(key, _, _)| *key);
    let mut node = Vec::new();
    node.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (key, _, _) in &entries {
        node.extend_from_slice(&(key.len() as u32).to_be_bytes());
        node.extend_from_slice(key);
    }
    node.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (_, offset, length) in entries {
        node.extend_from_slice(&offset.to_be_bytes());
        node.extend_from_slice(&length.to_be_bytes());
    }
    for _ in 0..17 {
        node.extend_from_slice(&0_u64.to_be_bytes());
    }
    assert!(node.len() <= 464);
    node.resize(464, 0);
    node
}

fn gallery_script(id: u64, title: &str, related: &str) -> String {
    GALLERY_SCRIPT
        .replace("\"id\": \"424242\"", &format!("\"id\": \"{id}\""))
        .replace("Fixture } Landscape Collection", title)
        .replace("[424240, \"424241\", 424240]", related)
}

#[test]
#[ignore = "opt-in live Floating Detail metadata regression smoke"]
fn live_floating_detail_metadata_for_reported_galleries() {
    assert_eq!(
        std::env::var("ATSUMI_ALLOW_LIVE_SMOKE").as_deref(),
        Ok("1"),
        "live network access requires ATSUMI_ALLOW_LIVE_SMOKE=1"
    );
    let adapter = HitomiLiveAdapter::new(HitomiLiveConfig {
        request_start_interval: Duration::ZERO,
        ..HitomiLiveConfig::default()
    })
    .expect("construct live adapter");

    for id in [4_133_977, 4_136_275, 4_137_316] {
        let detail = adapter
            .gallery_detail_get(GalleryId::new(id).unwrap())
            .unwrap_or_else(|error| panic!("gallery {id} detail failed: {error}"))
            .unwrap_or_else(|| panic!("gallery {id} detail was missing"));
        assert_eq!(detail.summary.id.get(), id);
        assert_eq!(detail.summary.pages as usize, detail.page_dimensions.len());
        assert!(!detail.page_dimensions.is_empty());
    }
}

#[test]
#[ignore = "opt-in live Korean galleriesindex search smoke"]
fn live_korean_title_index_search_smoke() {
    assert_eq!(
        std::env::var("ATSUMI_ALLOW_LIVE_SMOKE").as_deref(),
        Ok("1"),
        "live network access requires ATSUMI_ALLOW_LIVE_SMOKE=1"
    );
    let adapter = HitomiLiveAdapter::new(HitomiLiveConfig {
        request_start_interval: Duration::ZERO,
        max_retries: 0,
        ..HitomiLiveConfig::default()
    })
    .expect("construct live adapter");
    let started = Instant::now();
    let result = adapter
        .search_submit(&SearchRequest {
            text: "줄곧".into(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: vec![Language::Korean],
            sort: SearchSort::Recent,
            page_size: 5,
        })
        .expect("query the live galleries title index with a Hangul token");
    assert!(result.query_id.starts_with("hitomi-"));
    assert!(result.first_page.items.len() <= 5);
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "indexed Hangul title search unexpectedly behaved like a metadata scan"
    );
}

#[test]
#[ignore = "opt-in live gallery 4113714 full download pipeline smoke"]
fn live_gallery_4113714_download_pipeline() {
    assert_eq!(
        std::env::var("ATSUMI_ALLOW_LIVE_SMOKE").as_deref(),
        Ok("1"),
        "live network access requires ATSUMI_ALLOW_LIVE_SMOKE=1"
    );
    let adapter = HitomiLiveAdapter::new(HitomiLiveConfig {
        max_candidate_ids: 3,
        query_cache_capacity: 1,
        related_gallery_limit: 1,
        ..HitomiLiveConfig::default()
    })
    .expect("construct live adapter");
    let gallery_id = GalleryId::new(4_113_714).expect("fixed live gallery id");
    let cancellation = CancellationToken::new();
    let snapshot = adapter
        .gallery_snapshot(gallery_id, &cancellation)
        .unwrap_or_else(|error| {
            panic!(
                "live gallery 4113714 metadata failed: {}",
                error.code.as_str()
            )
        });
    assert_eq!(
        snapshot.pages.len(),
        18,
        "gallery 4113714 page count changed"
    );
    let temporary = tempfile::tempdir().expect("temporary live download root");
    let store = FilesystemArtifactStore::new();
    let relative = ArtifactRelativePath::new("live-4113714").unwrap();
    let layout = store
        .prepare_layout(temporary.path(), &relative, false)
        .expect("prepare isolated live artifact");
    let mut selected_format_counts = BTreeMap::<&'static str, u32>::new();
    let mut selected_total_bytes = 0_u64;
    let mut verified_pages = 0_u32;

    for source_page in &snapshot.pages {
        let payload = adapter
            .download_page(gallery_id, source_page.source_page_number, &cancellation)
            .unwrap_or_else(|error| {
                for diagnostic in &error.candidate_diagnostics {
                    eprintln!(
                        "sourcePage={} format={} status={:?} contentType={:?} bytes={:?} errorCode={:?} retryable={}",
                        source_page.source_page_number.get(),
                        diagnostic.format,
                        diagnostic.http_status,
                        diagnostic.content_type,
                        diagnostic.bytes_received,
                        diagnostic.error_code.map(|code| code.as_str()),
                        diagnostic.retryable,
                    );
                }
                panic!(
                    "live gallery 4113714 page {} download failed: {}",
                    source_page.source_page_number.get(),
                    error.code.as_str()
                );
            });
        for diagnostic in &payload.candidate_diagnostics {
            eprintln!(
                "sourcePage={} format={} status={:?} contentType={:?} bytes={:?} errorCode={:?} retryable={}",
                source_page.source_page_number.get(),
                diagnostic.format,
                diagnostic.http_status,
                diagnostic.content_type,
                diagnostic.bytes_received,
                diagnostic.error_code.map(|code| code.as_str()),
                diagnostic.retryable,
            );
        }
        assert!(
            payload.source_page_number == source_page.source_page_number,
            "source page identity mismatch at page {}",
            source_page.source_page_number.get()
        );
        assert!(
            payload.source_revision == source_page.source_revision,
            "source revision mismatch at page {}",
            source_page.source_page_number.get()
        );
        selected_total_bytes = selected_total_bytes
            .checked_add(u64::try_from(payload.bytes.len()).expect("selected page size fits u64"))
            .expect("selected live byte total fits u64");
        *selected_format_counts
            .entry(payload.source_format.as_str())
            .or_default() += 1;
        let stored = store
            .store_page(&layout, &payload, &cancellation)
            .expect("store verified live WebP");
        assert!(matches!(
            store
                .verify_existing_page(
                    &layout,
                    source_page.source_page_number,
                    &source_page.source_revision,
                    Some(&stored),
                )
                .expect("verify stored live page"),
            ExistingPageVerification::Verified(_)
        ));
        verified_pages += 1;
    }

    eprintln!(
        "verifiedPages={} selectedFormatCounts={:?} selectedTotalBytes={}",
        verified_pages, selected_format_counts, selected_total_bytes,
    );
    assert_eq!(verified_pages, 18);
}

#[test]
#[ignore = "opt-in live Hitomi tag, artist, and group catalog smoke"]
fn live_tag_catalog_refresh_parses_all_allowlisted_pages() {
    assert_eq!(
        std::env::var("ATSUMI_ALLOW_LIVE_SMOKE").as_deref(),
        Ok("1"),
        "live network access requires ATSUMI_ALLOW_LIVE_SMOKE=1"
    );
    let adapter =
        HitomiLiveAdapter::new(HitomiLiveConfig::default()).expect("construct live adapter");
    let entries = adapter
        .tag_catalog_fetch_all()
        .expect("fetch and parse all allowlisted catalog pages");
    assert!(entries.len() >= 1_000);
    assert!(entries
        .iter()
        .any(|entry| entry.canonical_token == "female:big_balls"));
    assert!(entries
        .iter()
        .any(|entry| entry.canonical_token == "female:ball_sucking"));
    assert!(entries
        .iter()
        .any(|entry| entry.canonical_token.starts_with("artist:")));
    assert!(entries
        .iter()
        .any(|entry| entry.canonical_token.starts_with("group:")));
}

fn one_pixel_png() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(1, 1)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn fallback_source_page_png() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(1_536, 768)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}
