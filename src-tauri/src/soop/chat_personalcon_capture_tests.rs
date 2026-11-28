use super::*;

fn body(message_type: &str) -> String {
    let mut fields = vec![""; 14];
    fields[0] = "Public text";
    fields[2] = "0";
    fields[3] = message_type;
    fields[5] = "Public";
    fields[6] = "32|262144";
    fields[7] = "17";
    fields[8] = "E04949";
    fields[10] = "17";
    fields[11] = "0";
    format!("\x0c{}\x0c", fields.join("\x0c"))
}
#[test]
fn captured_personalcon_metadata_and_typed_capability_are_separated() {
    let context = personalcon::Context::from_channel(
        &serde_json::json!({"PCON_OBJECT":{"tier1":[{"MONTH":0,"FILENAME":"https://static.file.sooplive.com/spcon/373867c54c3ee5ae5.png?_=1741052109"}]},"TIER1_NICK":"Basic"}),
    );
    let text = body("0");
    let frame = Frame {
        service: 5,
        result: 0,
        id: Some("one"),
        body: &text,
    };
    let mut rows = Vec::new();
    let mut summary = ChatSummary::default();
    receive_public_appearance(
        &frame,
        Instant::now(),
        &mut |row| {
            rows.push(row);
            Ok(())
        },
        &mut summary,
        &mut Seen::default(),
        Some("fixture"),
        None,
        Some(&context),
    )
    .ok()
    .unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    let request = row.personalcon_asset.as_ref().unwrap();
    assert_eq!(
        row.extra.as_ref().unwrap()["personalcon"]["imageAssetId"],
        request.id()
    );
    let json = serde_json::to_string(row).unwrap();
    assert!(!json.contains("https://"));
    assert!(!json.contains("personalconAsset"));
    assert_eq!(summary.unsupported_rich_events, 0);
}
#[test]
fn invalid_personalcon_keeps_body_and_emits_single_honest_coverage_marker() {
    let context = personalcon::Context::from_channel(
        &serde_json::json!({"PCON_OBJECT":[{"MONTH":0,"FILENAME":"https://evil.test/a.png"}]}),
    );
    let text = body("0");
    let frame = Frame {
        service: 5,
        result: 0,
        id: None,
        body: &text,
    };
    let mut rows = Vec::new();
    let mut summary = ChatSummary::default();
    let mut seen = Seen::default();
    for _ in 0..2 {
        receive_public_appearance(
            &frame,
            Instant::now(),
            &mut |row| {
                rows.push(row);
                Ok(())
            },
            &mut summary,
            &mut seen,
            Some("fixture"),
            None,
            Some(&context),
        )
        .ok()
        .unwrap();
    }
    assert_eq!(rows.iter().filter(|r| r.kind == "gap").count(), 1);
    assert_eq!(rows.iter().filter(|r| r.text == "Public text").count(), 2);
    assert!(rows.iter().all(|r| r.personalcon_asset.is_none()));
}
#[test]
fn official_staff_and_police_do_not_mint_subscriber_images() {
    let context = personalcon::Context::from_channel(&serde_json::json!({}));
    for (kind, expected) in [("1", "staff"), ("2", "police"), ("1.0", "staff")] {
        let text = body(kind);
        let frame = Frame {
            service: 5,
            result: 0,
            id: None,
            body: &text,
        };
        let mut rows = Vec::new();
        receive_public_appearance(
            &frame,
            Instant::now(),
            &mut |row| {
                rows.push(row);
                Ok(())
            },
            &mut ChatSummary::default(),
            &mut Seen::default(),
            Some("fixture"),
            None,
            Some(&context),
        )
        .ok()
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].extra.as_ref().unwrap()["messageType"], expected);
        assert!(rows[0].personalcon_asset.is_none());
        assert_eq!(rows[0].text, "Public text");
    }
}
#[test]
fn ordinary_and_ogq_streamer_capture_representative_month_instead_of_accumulated() {
    let context = personalcon::Context::from_channel(
        &serde_json::json!({"PCON_OBJECT":{"tier1":[{"MONTH":0,"FILENAME":"https://static.file.sooplive.com/spcon/aa.png"},{"MONTH":12,"FILENAME":"https://static.file.sooplive.com/spcon/bb.png"}]}}),
    );
    for service in [5, 109] {
        let text = if service == 5 {
            body("0")
                .replace("32|262144", "4|524288")
                .replace("\x0c17\x0c0\x0c", "\x0c17\x0c1\x0c")
        } else {
            let f = [
                "room",
                "Public text",
                "gem_fixture",
                "0",
                "version",
                "sender",
                "Public",
                "4|524288",
                "0",
                "0",
                "0",
                "png",
                "17",
                "E04949",
                "",
                "17",
                "1",
                "0",
            ];
            format!("\x0c{}\x0c", f.join("\x0c"))
        };
        let frame = Frame {
            service,
            result: 0,
            id: None,
            body: &text,
        };
        let mut rows = Vec::new();
        receive_public_appearance(
            &frame,
            Instant::now(),
            &mut |row| {
                rows.push(row);
                Ok(())
            },
            &mut ChatSummary::default(),
            &mut Seen::default(),
            Some("fixture"),
            None,
            Some(&context),
        )
        .ok()
        .unwrap();
        let row = rows.iter().find(|r| r.kind == "chat").unwrap();
        let r = row.personalcon_asset.as_ref().unwrap();
        assert_eq!(r.metadata().months, 1);
        assert_eq!(r.metadata().tier, 1);
        assert!(r.url().ends_with("/aa.png"));
    }
}
#[test]
fn broadcaster_portrait_binds_ephemeral_sender_without_saving_identity_or_url() {
    let context = personalcon::Context::from_channel(&serde_json::json!({}));
    for service in [5, 109] {
        for sender in ["fixture(1)", "different"] {
            let text = if service == 5 {
                body("0")
                    .replace(
                        "Public text\x0c\x0c0",
                        &format!("Public text\x0c{sender}\x0c0"),
                    )
                    .replace("32|262144", "4|0")
            } else {
                let f = [
                    "room",
                    "Public text",
                    "gem_fixture",
                    "0",
                    "version",
                    sender,
                    "Public",
                    "4|0",
                    "0",
                    "0",
                    "0",
                    "png",
                    "17",
                    "E04949",
                    "",
                    "17",
                    "0",
                    "0",
                ];
                format!("\x0c{}\x0c", f.join("\x0c"))
            };
            let frame = Frame {
                service,
                result: 0,
                id: None,
                body: &text,
            };
            let mut rows = Vec::new();
            receive_public_appearance(
                &frame,
                Instant::now(),
                &mut |row| {
                    rows.push(row);
                    Ok(())
                },
                &mut ChatSummary::default(),
                &mut Seen::default(),
                Some("fixture"),
                None,
                Some(&context),
            )
            .ok()
            .unwrap();
            let row = rows.iter().find(|r| r.kind == "chat").unwrap();
            assert_eq!(row.avatar_asset.is_some(), sender == "fixture(1)");
            let raw = serde_json::to_string(row).unwrap();
            assert!(!raw.contains("https://"));
            assert!(!raw.contains(sender));
            assert!(row.personalcon_asset.is_none());
        }
    }
}
