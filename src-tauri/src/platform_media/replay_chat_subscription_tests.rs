use super::*;
#[test]
fn subscription_roundtrip_preserves_time_identity_no_money_and_provider_boundary() {
    for (service, metadata) in [
        (
            91,
            serde_json::json!({"event":"new","displayTier":1,"productMonths":3,"fromVod":false}),
        ),
        (
            93,
            serde_json::json!({"event":"renewal","displayTier":2,"months":17}),
        ),
    ] {
        let raw = serde_json::json!({"kind":"membership","receivedAtMs":1700000001000u64,"messageId":"public-message",
            "nickname":"Public","text":"not a subscription message body","amount":"not money",
            "extra":{"service":service,"soopSubscription":metadata}});
        let events = normalize_public_line("soop", &serde_json::to_vec(&raw).unwrap()).unwrap();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.kind, "membership");
        assert!(event.text.is_empty() && event.amount.is_none());
        assert_eq!(event.source_timestamp_ms, Some(1700000001000));
        assert_eq!(event.message_id.as_ref().unwrap().len(), 64);
        let stored = serde_json::to_vec(event).unwrap();
        let replay = parse_line(&stored, "soop").unwrap();
        assert_eq!(replay.unsupported, 0);
        assert_eq!(
            replay.items[0].item.soop_subscription,
            event.soop_subscription
        );
        assert!(parse_line(&stored, "youtube").unwrap().items[0]
            .item
            .soop_subscription
            .is_none());
        let mut forged = serde_json::to_value(event).unwrap();
        forged["subscriptionAssets"] = serde_json::json!([{"url":"https://private"}]);
        assert!(serde_json::from_value::<NormalizedChatEvent>(forged)
            .unwrap()
            .subscription_assets
            .is_empty());
    }
}
#[test]
fn subscription_invalid_optional_assets_keep_event_and_invalid_schema_keeps_legacy_text() {
    let mut value = serde_json::json!({"format":"atsumi-chat-v1","kind":"membership","author":"Public","text":"legacy",
        "soopSubscription":{"event":"renewal","displayTier":2,"months":17,"imageAssetId":"https://evil"}});
    let parsed = parse_line(&serde_json::to_vec(&value).unwrap(), "soop").unwrap();
    assert_eq!(parsed.unsupported, 1);
    assert_eq!(
        parsed.items[0]
            .item
            .soop_subscription
            .as_ref()
            .unwrap()
            .months,
        Some(17)
    );
    value["soopSubscription"]["months"] = serde_json::json!(-1);
    let parsed = parse_line(&serde_json::to_vec(&value).unwrap(), "soop").unwrap();
    assert_eq!(parsed.unsupported, 1);
    assert!(parsed.items[0].item.soop_subscription.is_none());
    assert_eq!(parsed.items[0].item.text, "legacy");
    value.as_object_mut().unwrap().remove("soopSubscription");
    assert_eq!(
        parse_line(&serde_json::to_vec(&value).unwrap(), "soop")
            .unwrap()
            .unsupported,
        0
    );
    let raw = serde_json::json!({"kind":"membership","extra":{"service":144,"soopSubscription":{"event":"renewal","displayTier":1,"months":1}}});
    assert!(
        normalize_public_line("soop", &serde_json::to_vec(&raw).unwrap())
            .unwrap()
            .is_empty()
    );
}
