use serde::Deserialize;

use crate::source::{
    map_http_status, map_transport_failure, SourceErrorCategory, SourceErrorCode,
    TransportFailureKind,
};

use super::{
    download_full_candidates, galleryinfo_script_url, gg_script_url, index_all_nozomi_url,
    parse_galleryinfo_script, parse_gg_routing, parse_nozomi_ids, parse_nozomi_range,
    webp_full_candidates, webp_thumbnail_candidates, HitomiImageFormat, HitomiTagKind,
    NozomiByteRange, ThumbnailSize, HITOMI_METADATA_ORIGIN, HITOMI_PARSER_VERSION,
    HITOMI_RESOLVER_VERSION, NOZOMI_CONTENT_TYPE,
};

const GALLERYINFO_NORMAL: &str = include_str!("../../../fixtures/hitomi/galleryinfo-normal.js");
const GALLERYINFO_MALFORMED: &str =
    include_str!("../../../fixtures/hitomi/galleryinfo-malformed.js");
const GALLERYINFO_INVALID_DATA: &str =
    include_str!("../../../fixtures/hitomi/galleryinfo-invalid-data.js");
const GG_CURRENT: &str = include_str!("../../../fixtures/hitomi/gg-current.js");
const GG_MALFORMED: &str = include_str!("../../../fixtures/hitomi/gg-malformed.js");
const NOZOMI_RANGE_HEX: &str = include_str!("../../../fixtures/hitomi/nozomi-range.hex");
const HTTP_POLICY: &str = include_str!("../../../fixtures/hitomi/http-policy.json");
const TRANSPORT_POLICY: &str = include_str!("../../../fixtures/hitomi/transport-policy.json");

#[test]
fn parser_and_resolver_contracts_are_explicitly_versioned() {
    assert_eq!(HITOMI_PARSER_VERSION, 1);
    assert_eq!(HITOMI_RESOLVER_VERSION, 1);
}

#[test]
fn parses_galleryinfo_wrapper_into_typed_projections() {
    let metadata = parse_galleryinfo_script(GALLERYINFO_NORMAL).expect("fixture should parse");

    assert_eq!(metadata.id, 424_242);
    assert_eq!(metadata.title, "Fixture } Landscape Collection");
    assert_eq!(metadata.artists, vec!["Example Artist"]);
    assert_eq!(metadata.groups, vec!["Example Group"]);
    assert_eq!(metadata.series, vec!["original"]);
    assert_eq!(metadata.related_gallery_ids, vec![424_240, 424_241]);
    assert_eq!(metadata.pages.len(), 2);
    assert_eq!(metadata.pages[0].source_page, 1);
    assert_eq!(metadata.pages[0].width, Some(1200));
    assert_eq!(metadata.pages[0].height, Some(1600));
    assert_eq!(metadata.pages[0].aspect_ratio(), Some(0.75));
    assert_eq!(metadata.pages[0].has_webp, None);
    assert!(metadata.pages[0].has_avif);
    assert_eq!(metadata.pages[1].has_webp, Some(false));
    assert!(metadata.pages[1].has_avif);
    assert_eq!(metadata.tags[1].kind, HitomiTagKind::Female);

    let summary = metadata.summary();
    assert_eq!(summary.primary_artist.as_deref(), Some("Example Artist"));
    assert_eq!(summary.primary_group.as_deref(), Some("Example Group"));
    assert_eq!(summary.series, vec!["original"]);
    assert_eq!(summary.characters, vec!["Example Character"]);
    assert_eq!(summary.page_count, 2);
    assert_eq!(summary.cover.as_ref().map(|page| page.source_page), Some(1));
    assert_eq!(
        summary.source_url,
        "https://hitomi.la/galleries/fixture-landscape-424242.html"
    );

    let detail = metadata.detail();
    assert_eq!(detail.pages[1].source_page, 2);
    assert_eq!(detail.characters, vec!["Example Character"]);
    assert_eq!(detail.source_revision, summary.source_revision);
    assert_eq!(metadata.page(2).unwrap().name, "002.png");
}

#[test]
fn source_page_lookup_is_one_based_and_typed() {
    let metadata = parse_galleryinfo_script(GALLERYINFO_NORMAL).unwrap();

    let zero = metadata.page(0).unwrap_err();
    assert_eq!(zero.code, SourceErrorCode::Validation);
    assert_eq!(zero.category, SourceErrorCategory::Input);

    let missing = metadata.page(3).unwrap_err();
    assert_eq!(missing.code, SourceErrorCode::NotFound);
    assert_eq!(missing.category, SourceErrorCategory::Missing);
}

#[test]
fn gallery_revision_is_stable_and_tracks_metadata_changes() {
    let first = parse_galleryinfo_script(GALLERYINFO_NORMAL).unwrap();
    let second = parse_galleryinfo_script(GALLERYINFO_NORMAL).unwrap();
    assert_eq!(first.source_revision, second.source_revision);
    assert!(first
        .source_revision
        .as_str()
        .starts_with("hitomi-gallery-v1:424242:"));
    assert!(first.pages[0]
        .source_revision
        .as_str()
        .starts_with("hitomi-page-v1:"));

    let changed_script = GALLERYINFO_NORMAL.replace(
        "Fixture } Landscape Collection",
        "Fixture } Landscape Collection revised",
    );
    let changed = parse_galleryinfo_script(&changed_script).unwrap();
    assert_ne!(first.source_revision, changed.source_revision);
}

#[test]
fn malformed_wrapper_and_invalid_metadata_have_distinct_codes() {
    let malformed = parse_galleryinfo_script(GALLERYINFO_MALFORMED).unwrap_err();
    assert_eq!(malformed.code, SourceErrorCode::Protocol);
    assert_eq!(malformed.category, SourceErrorCategory::Contract);

    let invalid = parse_galleryinfo_script(GALLERYINFO_INVALID_DATA).unwrap_err();
    assert_eq!(invalid.code, SourceErrorCode::InvalidData);
    assert!(invalid.message.contains("64-character hexadecimal digest"));
}

#[test]
fn parses_current_classic_gg_shape() {
    let routing = parse_gg_routing(GG_CURRENT).expect("current fixture should parse");

    assert_eq!(routing.base_path(), "1786694402/");
    assert_eq!(routing.default_route(), 1);
    assert_eq!(routing.overrides().get(&4062), Some(&0));
    assert_eq!(routing.overrides().get(&2748), Some(&0));
    assert_eq!(routing.overrides().get(&33), Some(&1));

    let metadata = parse_galleryinfo_script(GALLERYINFO_NORMAL).unwrap();
    assert_eq!(routing.route_for_hash(&metadata.pages[0].hash), Ok(0));
    assert_eq!(routing.route_for_hash(&metadata.pages[1].hash), Ok(1));
}

#[test]
fn malformed_gg_shape_is_a_protocol_error() {
    let error = parse_gg_routing(GG_MALFORMED).unwrap_err();
    assert_eq!(error.code, SourceErrorCode::Protocol);
    assert!(error.message.contains("base path"));
}

#[test]
fn generates_deterministic_thumbnail_and_full_webp_candidates() {
    let routing = parse_gg_routing(GG_CURRENT).unwrap();
    let metadata = parse_galleryinfo_script(GALLERYINFO_NORMAL).unwrap();
    let page = &metadata.pages[0];
    let hash = &page.hash;

    let thumbnails = webp_thumbnail_candidates(page, &routing, ThumbnailSize::Large).unwrap();
    assert_eq!(thumbnails.len(), 3);
    assert_eq!(
        thumbnails[0].url,
        format!("https://atn.gold-usergeneratedcontent.net/webpbigtn/f/de/{hash}.webp")
    );
    assert_eq!(
        thumbnails[1].url,
        format!("https://atn.gold-usergeneratedcontent.net/webp/f/de/{hash}.webp")
    );
    assert_eq!(
        thumbnails[2].url,
        format!("https://w1.gold-usergeneratedcontent.net/1786694402/4062/{hash}.webp")
    );
    assert!(thumbnails
        .iter()
        .all(|candidate| candidate.content_type == "image/webp"));

    let full = webp_full_candidates(page, &routing).unwrap();
    assert_eq!(full.len(), 4);
    assert_eq!(full[0].url, thumbnails[2].url);
    assert_eq!(
        full[3].url,
        format!("https://w2.gold-usergeneratedcontent.net/webp/f/de/{hash}.webp")
    );
    let downloadable = download_full_candidates(page, &routing).unwrap();
    let formats = downloadable
        .iter()
        .map(|candidate| candidate.format)
        .collect::<Vec<_>>();
    let first_non_webp = formats
        .iter()
        .position(|format| *format != HitomiImageFormat::Webp)
        .unwrap();
    assert!(formats[..first_non_webp]
        .iter()
        .all(|format| *format == HitomiImageFormat::Webp));
    assert!(formats[first_non_webp..].contains(&HitomiImageFormat::Avif));
    assert!(formats.contains(&HitomiImageFormat::Jpeg));

    assert!(
        webp_thumbnail_candidates(&metadata.pages[1], &routing, ThumbnailSize::Small)
            .unwrap()
            .is_empty()
    );
    assert!(webp_full_candidates(&metadata.pages[1], &routing)
        .unwrap()
        .is_empty());
}

#[test]
fn parses_big_endian_nozomi_ids_and_validates_requested_range() {
    let bytes = decode_hex_fixture(NOZOMI_RANGE_HEX);
    assert_eq!(
        parse_nozomi_ids(&bytes).unwrap(),
        vec![1, 42, 2_147_483_647, 4_294_967_295]
    );

    let requested = NozomiByteRange::new(2, 3).unwrap();
    assert_eq!(requested.start, 8);
    assert_eq!(requested.end_inclusive, 19);
    assert_eq!(requested.header_value(), "bytes=8-19");
    assert_eq!(
        parse_nozomi_range(&bytes[..12], requested).unwrap(),
        vec![1, 42, 2_147_483_647]
    );

    let truncated = parse_nozomi_ids(&bytes[..bytes.len() - 1]).unwrap_err();
    assert_eq!(truncated.code, SourceErrorCode::InvalidData);
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpPolicyCase {
    case: String,
    status: u16,
    retry_after_seconds: Option<u64>,
    expected: String,
    retryable: bool,
}

#[test]
fn applies_saved_http_status_policy_for_404_429_and_503() {
    let cases: Vec<HttpPolicyCase> = serde_json::from_str(HTTP_POLICY).unwrap();
    for case in cases {
        let outcome = map_http_status(case.status, case.retry_after_seconds);
        if case.expected == "ok" {
            outcome.unwrap_or_else(|error| panic!("{}: {error}", case.case));
            continue;
        }
        let error = outcome.unwrap_err();
        assert_eq!(error.code.as_str(), case.expected, "{}", case.case);
        assert_eq!(error.retryable, case.retryable, "{}", case.case);
        assert_eq!(error.http_status, Some(case.status), "{}", case.case);
        assert_eq!(
            error.retry_after_seconds, case.retry_after_seconds,
            "{}",
            case.case
        );
    }
}

#[derive(Debug, Deserialize)]
struct TransportPolicyCase {
    case: String,
    kind: String,
    expected: String,
    retryable: bool,
}

#[test]
fn applies_saved_timeout_and_transport_policy() {
    let cases: Vec<TransportPolicyCase> = serde_json::from_str(TRANSPORT_POLICY).unwrap();
    for case in cases {
        let kind = match case.kind.as_str() {
            "timeout" => TransportFailureKind::Timeout,
            "dns" => TransportFailureKind::Dns,
            "connection" => TransportFailureKind::Connection,
            other => panic!("unknown fixture transport kind: {other}"),
        };
        let error = map_transport_failure(kind, &case.case);
        assert_eq!(error.code.as_str(), case.expected, "{}", case.case);
        assert_eq!(error.retryable, case.retryable, "{}", case.case);
        assert_eq!(error.http_status, None);
    }
}

#[test]
fn endpoint_helpers_are_allowlisted_and_validate_gallery_ids() {
    assert_eq!(
        galleryinfo_script_url(424_242).unwrap(),
        format!("{HITOMI_METADATA_ORIGIN}/galleries/424242.js")
    );
    assert_eq!(gg_script_url(), format!("{HITOMI_METADATA_ORIGIN}/gg.js"));
    assert_eq!(
        index_all_nozomi_url(),
        format!("{HITOMI_METADATA_ORIGIN}/index-all.nozomi")
    );
    assert_eq!(NOZOMI_CONTENT_TYPE, "application/x-nozomi");
    assert_eq!(
        galleryinfo_script_url(0).unwrap_err().code,
        SourceErrorCode::Validation
    );
}

fn decode_hex_fixture(fixture: &str) -> Vec<u8> {
    fixture
        .split_ascii_whitespace()
        .flat_map(|word| {
            assert_eq!(word.len() % 2, 0);
            (0..word.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&word[index..index + 2], 16).unwrap())
                .collect::<Vec<_>>()
        })
        .collect()
}
