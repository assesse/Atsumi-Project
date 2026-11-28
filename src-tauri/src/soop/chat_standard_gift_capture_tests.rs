use super::*;

fn frame(service: u16, fields: &[&str]) -> Vec<u8> {
    let body = format!("\x0c{}\x0c", fields.join("\x0c"));
    format!("\x1d\x09{service:04}{:06}00{ZERO_ID}{body}", body.len()).into_bytes()
}

#[test]
fn standard_and_video_expected_ids_are_raw_safe_before_normalization() {
    let context = gifts::assets::Context::new();
    for (service, fields) in [(18, vec!["fixture(1)","sender-private","Public","211","0","","","","1","0"]),
        (33, vec!["room-private","fixture(1)","","sender-private","Public","211","0","","","1","0"]),
        (105, vec!["room-private","fixture(1)","sender-private","Public","7","0","","0","0","","","","ignored","0"])] {
        let bytes = frame(service, &fields); let mut parsed = parse(&bytes).ok().unwrap(); parsed.id = Some("event-fixture");
        let mut rows = Vec::new(); let mut summary = ChatSummary::default(); let mut seen = Seen::default();
        for _ in 0..2 {
            receive_public_assets(&parsed, Instant::now(), &mut |row| {rows.push(row); Ok(())}, &mut summary,
                &mut seen, Some("fixture"), None, None, Some(&context)).ok().unwrap();
        }
        assert_eq!(rows.len(), 1); assert_eq!(summary.messages, 1); assert_eq!(summary.unsupported_rich_events, 0);
        let row = &rows[0]; let request = row.gift_asset.as_ref().unwrap();
        let raw = serde_json::to_value(row).unwrap(); let meta = gifts::from_raw(&raw).unwrap();
        assert_eq!(meta.image_asset_id.as_deref(), Some(request.id()));
        assert!(request.matches(&meta)); assert!(row.text.is_empty());
        let bytes = serde_json::to_vec(row).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        for private in ["sender-private", "room-private", "fixture(1)", "https://", "giftAsset"] { assert!(!text.contains(private)); }
    }
}

#[test]
fn endpoint_reconstruction_refreshes_standard_image_namespace() {
    let metadata = serde_json::json!({"CHANNEL":{"RESULT":1,"BPWD":"N","GRADE":0,"P_MIN_TIER":0,
        "BSTATUS":"BROADING","BJID":"fixture","CHDOMAIN":"chat-76DBFCC2.sooplive.com","CHPT":9000,
        "CHATNO":8709,"FTK":"fixture-ticket","BNO":123}});
    let a = endpoint_from_value("fixture", &metadata).ok().unwrap();
    let b = endpoint_from_value("fixture", &metadata).ok().unwrap();
    let f = ["fixture", "private", "Public", "211", "0", "", "", "", "1", "0"];
    let first = a.gift_assets.from_fields(18, &f, "fixture").unwrap();
    let next = b.gift_assets.from_fields(18, &f, "fixture").unwrap();
    assert_eq!(first.url(), next.url()); assert_ne!(first.id(), next.id());
}
