use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    io::Cursor,
    sync::{Arc, Mutex},
    time::Duration,
};

use reqwest::Url;

use crate::{
    application::{
        ArtifactStore, AutoFindSource, AutoFindSourceRequest, DownloadSourcePort,
        ExistingPageVerification, RepositoryError, SearchRepository, TagCatalogSource,
    },
    domain::{ArtifactRelativePath, GalleryId, Language, SearchRequest, SearchSort},
    infrastructure::FilesystemArtifactStore,
    source::{
        hitomi::{
            download_full_candidates, galleryinfo_script_url, gg_script_url,
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
}

#[test]
fn detail_keeps_main_page_dimensions_when_related_metadata_is_temporarily_unavailable() {
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
    );

    let detail = adapter
        .gallery_detail_get(GalleryId::new(7_001).unwrap())
        .expect("related failure is supplemental")
        .expect("main detail exists");

    assert_eq!(detail.summary.title, "Main detail fixture");
    assert!(!detail.page_dimensions.is_empty());
    assert!(detail.related.is_empty());
}

impl HttpTransport for FakeTransport {
    fn execute(&self, request: HttpRequest) -> Result<HttpPayload, SourceContractError> {
        self.calls.lock().unwrap().push(request.url.clone());
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
                newer_than_gallery_id: Some(GalleryId::new(100).unwrap()),
                candidate_limit: 1,
            },
            &cancellation,
        )
        .unwrap();

    assert_eq!(plan.candidate_ids, vec![GalleryId::new(300).unwrap()]);
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
