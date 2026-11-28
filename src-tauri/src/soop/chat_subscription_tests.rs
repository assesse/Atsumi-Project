use super::*;
fn new_fields(code: &str) -> Vec<&str> {
    vec![
        "private-room",
        "private-receiver",
        "private-sender",
        "Public",
        code,
        "unused",
        "unused",
        "2",
        "unused",
        "ko_KR",
        "123",
    ]
}
fn renewal_fields(month: &str) -> Vec<&str> {
    vec![
        "private-channel",
        "private-sender",
        "Public",
        month,
        "private-room",
        "ignored-type",
        "999",
        "2",
        "unused",
        "en_US",
        "123",
    ]
}
#[test]
fn ordered_products_preserve_alias_collisions_legacy_and_trial_ceremonies() {
    for (code, months, gift, vod) in [
        ("11", 1, true, false),
        ("12", 1, true, false),
        ("1", 3, false, false),
        ("2", 6, false, false),
        ("3", 12, false, false),
        ("10", 1, true, true),
        ("-1", 1, false, false),
        ("0", 1, false, true),
    ] {
        let p = parse(91, &new_fields(code), None).unwrap();
        assert_eq!(
            (
                p.metadata.product_months,
                p.metadata.from_vod,
                p.metadata.display_tier
            ),
            (Some(months), Some(vod), 1)
        );
        assert_eq!(p.requests[0].url().contains("monthsGift"), gift);
    }
    for (code, ending, tier) in [
        ("100", "gudok_1.png?v=123", 1),
        ("103", "gudok_90days_1.png?v=123", 1),
        ("111", "gudok_1monthsGift_1.png?v=123", 1),
        ("9213", "gudok_90days_1.png?v=123", 2),
    ] {
        let p = parse(91, &new_fields(code), None).unwrap();
        assert!(p.requests[0].url().ends_with(ending));
        assert_eq!(p.metadata.display_tier, tier);
        assert_eq!(p.metadata.months, None);
    }
    assert!(parse(91, &new_fields("99999"), None).is_err());
}
#[test]
fn renewal_display_uses_month_not_accumulated_month_or_type() {
    let fields = renewal_fields("17");
    let p = parse(93, &fields, None).unwrap();
    assert_eq!(
        (
            p.metadata.months,
            p.metadata.product_months,
            p.metadata.from_vod,
            p.metadata.display_tier
        ),
        (Some(17), None, None, 2)
    );
    assert!(p.requests[0].url().ends_with("gudok_17_en.png?v=123"));
    assert!(p.requests[1].url().ends_with("gudok_1_en.png"));
    for tier in ["", "1", "3", "-1", "2.0"] {
        let mut f = fields.clone();
        f[7] = tier;
        let p = parse(93, &f, None).unwrap();
        assert_eq!(p.metadata.display_tier, if tier == "2.0" { 2 } else { 1 });
        assert!(!p.partial);
    }
    for month in ["0", "-1", "1201", "1.5", "4294967296", ""] {
        assert!(parse(93, &renewal_fields(month), None).is_err());
    }
}
#[test]
fn image_language_version_and_opaque_identity_keep_exact_source_rules() {
    for (lang, suffix) in [
        ("", ""),
        ("ko_KR", ""),
        ("en_US", "_en"),
        ("ja_JP", "_en"),
        ("zh_CN", "_en"),
    ] {
        let mut f = new_fields("100");
        f[9] = lang;
        f[10] = "";
        let mut p = parse(91, &f, None).unwrap();
        for request in &p.requests {
            assert!(request.apply(&mut p.metadata));
            assert_eq!(canonical_url(request.url()), Some(request.url()));
        }
        assert!(p.requests[0]
            .url()
            .ends_with(&format!("gudok_1{suffix}.png")));
        assert_eq!(p.metadata.image_asset_id, p.metadata.fallback_asset_id);
        let mut wrong = p.metadata.clone();
        wrong.display_tier = 2;
        assert!(!p.requests[0].apply(&mut wrong));
    }
    for url in [
        "https://evil/subscription_ceremony/m/gudok_1.png",
        "https://static.file.sooplive.com/subscription_ceremony/m/../x.png",
        "https://static.file.sooplive.com/subscription_ceremony/m/gudok_1.png?other=1",
        "https://static.file.sooplive.com/subscription_ceremony/m/gudok_1.png?v=1&url=https://evil",
        "https://static.file.sooplive.com/subscription_ceremony/m/gudok_1201.png",
    ] {
        assert!(canonical_url(url).is_none());
    }
}
#[test]
fn optional_bad_image_and_tier_do_not_erase_public_membership() {
    for (index, bad) in [
        (9, "../ko"),
        (10, "https://evil"),
        (10, "18446744073709551616"),
    ] {
        let mut f = new_fields("103");
        f[index] = bad;
        let p = parse(91, &f, None).unwrap();
        assert!(p.partial);
        assert_eq!(p.nickname, "Public");
        assert_eq!(p.metadata.product_months, Some(3));
        assert_eq!(p.requests.len(), 1);
        assert!(p.requests[0].fallback);
    }
    let mut f = renewal_fields("17");
    f[7] = "NaN";
    let p = parse(93, &f, None).unwrap();
    assert!(p.partial);
    assert_eq!(p.metadata.months, Some(17));
}
#[test]
fn stored_optional_metadata_is_closed_without_network_capabilities() {
    let mut p = parse(91, &new_fields("103"), None).unwrap();
    for request in &p.requests {
        assert!(request.apply(&mut p.metadata));
    }
    let good = serde_json::to_value(&p.metadata).unwrap();
    assert!(!invalid_stored(&good));
    for key in ["imageAssetId", "fallbackAssetId", "tierNickname"] {
        let mut value = good.clone();
        value[key] = Value::String("\nhttps://untrusted".into());
        assert!(invalid_stored(&value));
        let saved = from_stored(&value).unwrap();
        assert_eq!(saved.product_months, Some(3));
    }
    for (key, value) in [
        ("displayTier", serde_json::json!(3)),
        ("months", serde_json::json!(3)),
        ("productMonths", serde_json::json!(2)),
        ("fromVod", serde_json::json!(1)),
    ] {
        let mut stored = good.clone();
        stored[key] = value;
        assert!(from_stored(&stored).is_none());
    }
    assert!(label(&"한".repeat(80)).is_some());
    assert!(label(&"한".repeat(81)).is_none());
    assert!(label("a\nb").is_none());
    assert!(label("   ").is_none());
    let labels = [Some("Saved public label".into()), None];
    assert_eq!(
        parse(91, &new_fields("103"), Some(&labels))
            .unwrap()
            .metadata
            .tier_nickname,
        labels[0]
    );
}
