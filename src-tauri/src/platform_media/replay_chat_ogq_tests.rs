use super::*;
fn raw(text: &str) -> Value {
    serde_json::json!({"kind":"chat","receivedAtMs":1700000001000u64,"messageId":"fixture109","nickname":"Public <b>","text":text,"flags":"32|262144","extra":{"service":109,"color":"255","subscriptionMonth":"7","accSubscriptionMonth":"9","nicknameColorKey":"E12E2E","soopEmoticon":{"kind":"ogq","animated":false}}})
}
#[test]
fn empty_emote_roundtrip_and_index_preserve_public_metadata_and_seek() {
    let rows = normalize_public_line("soop", &serde_json::to_vec(&raw("")).unwrap()).unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert!(row.text.is_empty() && row.amount.is_none() && row.soop_emoticon.is_some());
    let meta = row.soop.as_ref().unwrap();
    assert_eq!(meta.badge, "fan");
    assert_eq!(meta.role, "subscribe");
    assert_eq!(meta.message_color.as_deref(), Some("#FF0000"));
    assert_eq!(meta.nickname_color_key.as_deref(), Some("E12E2E"));
    assert_eq!(meta.subscription_months, Some(7));
    let serialized = serde_json::to_vec(row).unwrap();
    assert!(!String::from_utf8_lossy(&serialized).contains("https://"));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("chat.events.jsonl"),
        [serialized, vec![b'\n']].concat(),
    )
    .unwrap();
    std::fs::write(dir.path().join("chat-timeline.json"),r#"{"timelineQuality":"receive-time-approximate","firstSegmentCommittedAtMs":1700000004000,"firstSegmentDurationMs":4000}"#).unwrap();
    let later = read_page(dir.path(), "soop", 2000, None, 120).unwrap();
    assert_eq!(later.items.len(), 1);
    assert!(later.items[0].text.is_empty());
    assert_eq!(
        later.items[0].soop_emoticon.as_ref().unwrap().kind,
        crate::soop::chat::ogq::Kind::Ogq
    );
    assert!(read_page(dir.path(), "soop", 999, None, 120)
        .unwrap()
        .items
        .is_empty());
    assert_eq!(
        read_page(dir.path(), "soop", 1000, None, 120)
            .unwrap()
            .items
            .len(),
        1
    );
}
#[test]
fn damaged_optional_metadata_keeps_body_badges_and_provider_isolation() {
    let row = normalize_public_line("soop", &serde_json::to_vec(&raw("readable text")).unwrap())
        .unwrap()
        .remove(0);
    let base = serde_json::to_value(row).unwrap();
    for invalid in [
        serde_json::json!({"kind":"other","animated":false}),
        serde_json::json!({"kind":"ogq","animated":"yes"}),
        serde_json::json!({"kind":"ogq","animated":false,"staticImageAssetId":"https://evil"}),
    ] {
        let mut value = base.clone();
        value["soopEmoticon"] = invalid;
        value["soop"]["nicknameColorKey"] = serde_json::json!("url(evil)");
        let bytes = serde_json::to_vec(&value).unwrap();
        let soop = parse_line(&bytes, "soop").unwrap();
        assert_eq!(soop.unsupported, 1);
        assert_eq!(soop.items[0].item.text, "readable text");
        assert_eq!(soop.items[0].item.soop.as_ref().unwrap().badge, "fan");
        assert!(soop.items[0]
            .item
            .soop
            .as_ref()
            .unwrap()
            .nickname_color_key
            .is_none());
        let yt = parse_line(&bytes, "youtube").unwrap();
        assert_eq!(yt.unsupported, 0);
        assert!(yt.items[0].item.soop.is_none() && yt.items[0].item.soop_emoticon.is_none());
    }
}
