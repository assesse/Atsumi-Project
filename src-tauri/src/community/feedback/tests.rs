use super::*;
use crate::community::RequestFailure;
use std::cell::{Cell, RefCell};

const NOW: u64 = 10_000_000;
fn event(time: u64) -> Value {
    json!({"kind":"event","timeUnixMs":time,"session":"11111111-1111-4111-8111-111111111111","version":"2.1.1",
        "level":"ERROR","module":"atsumi_lib::streaming::browser","line":42,"traceId":1,
        "fields":{"diag_stage":"capture_bridge","error_code":"HTTP_TIMEOUT","elapsed_ms":12,"success":false,
        "capture_reason":"codec_changed","capture_mime":"video/mp4;codecs=avc1.64002a,mp4a.40.2","width":1920,"height":1080,
        "track_index":0,"init_bytes":3200}})
}
fn write_lines(directory: &Path, name: &str, lines: &[Value]) {
    use std::io::Write;
    let mut file = File::create(directory.join(name)).unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
}

#[test]
fn feedback_rechecks_each_field_and_drops_private_paths_urls_ids_text_and_tokens() {
    let mut raw = event(NOW);
    for key in [
        "path",
        "url",
        "title",
        "cookie",
        "message",
        "channelId",
        "galleryId",
        "computerName",
        "file",
    ] {
        raw[key] = json!("private-secret");
        raw["fields"][key] = json!("private-secret");
    }
    raw["fields"]["diag_work"] = json!("deadbeefdeadbeef");
    let sanitized = safe_event(&raw, NOW, &mut HashMap::new()).unwrap();
    let text = sanitized.to_string();
    assert!(!text.contains("private-secret"));
    assert!(!text.contains("deadbeef"));
    assert!(!text.contains("11111111"));
    assert_eq!(sanitized["session"], 1);
    assert_eq!(sanitized["fields"]["width"], 1920);
    assert_eq!(sanitized["fields"]["capture_reason"], "codec_changed");
    raw["fields"]["width"] = json!(20000);
    raw["fields"]["track_index"] = json!(4);
    raw["fields"]["init_bytes"] = json!(65537);
    raw["fields"]["capture_mime"] = json!("https://private.example/token");
    raw["fields"]["capture_reason"] = json!("private-secret");
    raw["fields"]["error_code"] = json!("Bearer private-secret");
    raw["module"] = json!("C:/Users/private-secret");
    let sanitized = safe_event(&raw, NOW, &mut HashMap::new()).unwrap();
    for key in [
        "width",
        "track_index",
        "init_bytes",
        "capture_mime",
        "capture_reason",
        "error_code",
    ] {
        assert!(sanitized["fields"].get(key).is_none());
    }
    assert!(sanitized.get("module").is_none());
}

#[test]
fn feedback_prepare_reads_only_four_fixed_ring_files_and_keeps_recent_previous_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let mut previous = event(NOW - 5000);
    previous["session"] = json!("22222222-2222-4222-8222-222222222222");
    write_lines(
        temp.path(),
        "application-0.jsonl",
        &[event(NOW - WINDOW_MS - 1), event(NOW + 1), event(NOW)],
    );
    write_lines(temp.path(), "application-3.jsonl", &[previous]);
    write_lines(temp.path(), "other.jsonl", &[event(NOW)]);
    write_lines(temp.path(), "application-4.jsonl", &[event(NOW)]);
    let prepared = prepare(temp.path(), "chzzk", NOW).unwrap();
    assert_eq!(prepared.event_count, 2);
    assert!(prepared.bytes <= MAX_PAYLOAD);
    assert!(Uuid::parse_str(&prepared.id).is_ok());
    assert_eq!(prepared.payload["events"][0]["timeUnixMs"], NOW - 5000);
    assert_eq!(prepared.payload["events"][1]["timeUnixMs"], NOW);
    assert_ne!(
        prepared.payload["events"][0]["session"],
        prepared.payload["events"][1]["session"]
    );
    // Local preview neither creates an identity file nor contacts a transport.
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 4);
    assert!(prepare(temp.path(), "../private", NOW).is_err());
    let missing = temp.path().join("missing");
    assert_eq!(prepare(&missing, "hitomi", NOW).unwrap().event_count, 0);
    assert!(!missing.exists());
}

#[test]
fn feedback_preparation_is_bounded_and_ignores_partial_or_oversized_lines() {
    let temp = tempfile::tempdir().unwrap();
    write_lines(
        temp.path(),
        "application-0.jsonl",
        &(0..6000).map(|i| event(NOW - 6000 + i)).collect::<Vec<_>>(),
    );
    std::fs::write(
        temp.path().join("application-1.jsonl"),
        format!("{{partial\n{}\n{}\n", "x".repeat(MAX_LINE + 1), event(NOW)),
    )
    .unwrap();
    let prepared = prepare(temp.path(), "danbooru", NOW).unwrap();
    assert!(prepared.bytes <= MAX_PAYLOAD);
    assert!(prepared.event_count <= MAX_EVENTS);
    assert!(prepared.event_count > 0);
    assert_eq!(prepared.payload["truncated"], true);
    assert_eq!(
        prepared.payload["events"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["timeUnixMs"],
        NOW
    );
}

#[test]
fn feedback_claim_requires_preview_expires_and_serializes_double_clicks_and_retries() {
    let temp = tempfile::tempdir().unwrap();
    let prepared = prepare(temp.path(), "hitomi", NOW).unwrap();
    let id = Uuid::parse_str(&prepared.id).unwrap();
    let mut drafts = DraftStore::default();
    assert!(drafts.claim(id, "설명", NOW).is_err());
    drafts.insert(prepared, "hitomi".into(), NOW).unwrap();
    assert!(matches!(drafts.claim(id, "설명", NOW), Ok(Claim::Send(_))));
    assert!(drafts.claim(id, "설명", NOW).is_err());
    drafts.finish(id, None);
    assert!(drafts.claim(id, "변경한 설명", NOW).is_err());
    assert!(matches!(drafts.claim(id, "설명", NOW), Ok(Claim::Send(_))));
    drafts.finish(
        id,
        Some(FeedbackReceipt {
            receipt_id: id.to_string(),
        }),
    );
    assert!(matches!(
        drafts.claim(id, "설명", NOW),
        Ok(Claim::Complete(_))
    ));
    assert!(drafts.claim(id, "설명", NOW + DRAFT_LIFETIME_MS).is_err());
    assert!(validate_comment(&"가".repeat(1000)).is_ok());
    assert!(validate_comment(&"가".repeat(1001)).is_err());
    assert!(validate_comment("private\0control").is_err());
}

#[test]
fn feedback_keeps_reports_above_the_previous_200_kib_cap() {
    let temp = tempfile::tempdir().unwrap();
    write_lines(
        temp.path(),
        "application-0.jsonl",
        &(0..1000).map(|i| event(NOW - 1000 + i)).collect::<Vec<_>>(),
    );
    let prepared = prepare(temp.path(), "chzzk", NOW).unwrap();
    assert_eq!(prepared.event_count, 1000);
    assert!(prepared.bytes > 200 * 1024);
    assert!(prepared.bytes <= 1024 * 1024);
    assert_eq!(prepared.payload["truncated"], false);
}

#[test]
fn feedback_prioritizes_older_login_and_errors_over_repetitive_health_samples() {
    let temp = tempfile::tempdir().unwrap();
    let mut login = event(NOW - 60_000);
    login["level"] = json!("INFO");
    login["fields"] =
        json!({"diag_stage":"chzzk_account_captcha_response","status":200,"success":true});
    let mut lines = vec![
        login,
        event(NOW - 59_000),
        json!({"kind":"http_summary","timeUnixMs":NOW-58_000,"failures":1}),
    ];
    for i in 0..6000 {
        let mut health = event(NOW - 6000 + i);
        health["level"] = json!("INFO");
        health["fields"] =
            json!({"diag_stage":"health","elapsed_ms":i,"renderer_private_bytes":123456789});
        lines.push(health);
    }
    write_lines(temp.path(), "application-0.jsonl", &lines);
    let prepared = prepare(temp.path(), "chzzk", NOW).unwrap();
    let events = prepared.payload["events"].as_array().unwrap();
    assert_eq!(prepared.payload["truncated"], true);
    assert!(prepared.bytes <= MAX_PAYLOAD);
    assert_eq!(
        events[0]["fields"]["diag_stage"],
        "chzzk_account_captcha_response"
    );
    assert_eq!(events[1]["level"], "ERROR");
    assert_eq!(events[2]["failures"], 1);
    assert!(events
        .windows(2)
        .all(|pair| pair[0]["timeUnixMs"].as_u64() <= pair[1]["timeUnixMs"].as_u64()));
    assert_eq!(events.last().unwrap()["timeUnixMs"], NOW - 1);
}

#[test]
fn feedback_caps_the_number_of_even_tiny_events_at_ten_thousand() {
    let temp = tempfile::tempdir().unwrap();
    let lines = (0..10_100)
        .map(|i| json!({"kind":"event","timeUnixMs":NOW - 10_100 + i}))
        .collect::<Vec<_>>();
    write_lines(temp.path(), "application-0.jsonl", &lines);
    let prepared = prepare(temp.path(), "chzzk", NOW).unwrap();
    assert_eq!(prepared.event_count, 10_000);
    assert_eq!(prepared.payload["truncated"], true);
    assert!(prepared.bytes <= MAX_PAYLOAD);
}

#[test]
fn feedback_drafts_are_bounded_and_identity_is_separate_from_community() {
    let temp = tempfile::tempdir().unwrap();
    let mut drafts = DraftStore::default();
    for i in 0..20 {
        drafts
            .insert(
                prepare(temp.path(), "chzzk", NOW + i).unwrap(),
                "chzzk".into(),
                NOW + i,
            )
            .unwrap();
    }
    assert_eq!(drafts.0.len(), MAX_DRAFTS);
    let path = identity_path(temp.path());
    assert!(path.ends_with(Path::new("feedback-identity").join(PROJECT_FILE)));
    assert_ne!(
        path,
        temp.path().join("community-identity").join(PROJECT_FILE)
    );
}

#[test]
fn completed_lease_cannot_unlock_a_newer_retry() {
    let temp = tempfile::tempdir().unwrap();
    let prepared = prepare(temp.path(), "hitomi", NOW).unwrap();
    let id = Uuid::parse_str(&prepared.id).unwrap();
    {
        let mut store = drafts().lock().unwrap();
        store.insert(prepared, "hitomi".into(), NOW).unwrap();
        assert!(matches!(store.claim(id, "retry", NOW), Ok(Claim::Send(_))));
    }
    let mut previous = SendLease(Some(id));
    previous.finish(None);
    assert!(matches!(
        drafts().lock().unwrap().claim(id, "retry", NOW),
        Ok(Claim::Send(_))
    ));
    drop(previous);
    assert!(drafts().lock().unwrap().claim(id, "retry", NOW).is_err());
    // A worker that exits without completing still releases its own attempt.
    drop(SendLease(Some(id)));
    assert!(matches!(
        drafts().lock().unwrap().claim(id, "retry", NOW),
        Ok(Claim::Send(_))
    ));
    drafts().lock().unwrap().0.remove(&id);
}

struct MemoryIdentity(RefCell<String>);
impl IdentityStore for MemoryIdentity {
    fn load(&self) -> Result<Identity, String> {
        serde_json::from_str(&self.0.borrow()).map_err(|_| "identity".into())
    }
    fn save(&self, value: &Identity) -> Result<(), String> {
        *self.0.borrow_mut() = serde_json::to_string(value).unwrap();
        Ok(())
    }
}
#[derive(Default)]
struct FakeTransport {
    routes: RefCell<Vec<String>>,
    bodies: RefCell<Vec<Value>>,
    fail: Cell<bool>,
    fail_signup_once: Cell<bool>,
}
impl Transport for FakeTransport {
    fn post(&self, route: &str, body: Value, token: Option<&str>) -> Result<Value, RequestFailure> {
        self.routes.borrow_mut().push(route.into());
        self.bodies.borrow_mut().push(body.clone());
        if route.starts_with("auth/") {
            assert!(token.is_none());
            if route == "auth/v1/signup" && self.fail_signup_once.replace(false) {
                return Err(RequestFailure {
                    message: "signup timeout".into(),
                    definitely_rejected: false,
                });
            }
            return Ok(
                json!({"access_token":"private-access","refresh_token":"private-refresh","expires_in":3600,
                "user":{"id":"11111111-1111-4111-8111-111111111111"}}),
            );
        }
        assert_eq!(token, Some("private-access"));
        if self.fail.get() {
            return Err(RequestFailure {
                message: "잠시 후 다시 시도해 주세요.".into(),
                definitely_rejected: false,
            });
        }
        Ok(json!({"receiptId":body["p_id"]}))
    }
}

#[test]
fn feedback_explicit_retry_recovers_an_interrupted_signup_without_resetting_author_keys() {
    let temp = tempfile::tempdir().unwrap();
    let prepared = prepare(temp.path(), "chzzk", NOW).unwrap();
    let id = Uuid::parse_str(&prepared.id).unwrap();
    let mut drafts = DraftStore::default();
    drafts.insert(prepared, "chzzk".into(), NOW).unwrap();
    let Claim::Send(draft) = drafts.claim(id, "network failure", NOW).unwrap() else {
        panic!()
    };
    let store = MemoryIdentity(RefCell::new(
        serde_json::to_string(&Identity::Unissued).unwrap(),
    ));
    let http = FakeTransport::default();
    http.fail_signup_once.set(true);
    assert!(submit(&store, &http, &draft).is_err());
    assert!(matches!(store.load().unwrap(), Identity::Issuing));
    assert_eq!(http.routes.borrow().as_slice(), ["auth/v1/signup"]);
    // The community helper retains its stricter author-identity protection.
    assert!(ensure_session(&store, &http, now_seconds(), true).is_err());
    assert_eq!(http.routes.borrow().len(), 1);
    // The next user-confirmed feedback attempt recovers before any report RPC.
    assert_eq!(
        submit(&store, &http, &draft).unwrap().receipt_id,
        id.to_string()
    );
    assert_eq!(
        http.routes.borrow().as_slice(),
        [
            "auth/v1/signup",
            "auth/v1/signup",
            "rest/v1/rpc/feedback_v1_submit"
        ]
    );
    // A durably saved identity is reused, never replaced on subsequent sends.
    assert!(submit(&store, &http, &draft).is_ok());
    assert_eq!(
        http.routes
            .borrow()
            .iter()
            .filter(|r| *r == "auth/v1/signup")
            .count(),
        2
    );
}

#[test]
fn feedback_capacity_errors_are_understandable_without_echoing_server_details() {
    assert_eq!(
        super::super::safe_server_error(
            400,
            &json!({"message":"Report limit reached. Try later."})
        ),
        "진단 제보 한도에 도달했습니다. 나중에 다시 보내 주세요."
    );
    assert!(
        !super::super::safe_server_error(500, &json!({"message":"private-token-path"}))
            .contains("private-token-path")
    );
}
#[test]
fn feedback_send_retries_same_snapshot_and_id_without_new_identity_or_renderer_payload() {
    let temp = tempfile::tempdir().unwrap();
    let prepared = prepare(temp.path(), "hitomi", NOW).unwrap();
    let id = Uuid::parse_str(&prepared.id).unwrap();
    let mut drafts = DraftStore::default();
    drafts
        .insert(prepared.clone(), "hitomi".into(), NOW)
        .unwrap();
    let store = MemoryIdentity(RefCell::new(
        serde_json::to_string(&Identity::Unissued).unwrap(),
    ));
    let http = FakeTransport::default();
    http.fail.set(true);
    let Claim::Send(first) = drafts.claim(id, "첫 제보", NOW).unwrap() else {
        panic!()
    };
    assert!(submit(&store, &http, &first).is_err());
    drafts.finish(id, None);
    http.fail.set(false);
    let Claim::Send(retry) = drafts.claim(id, "첫 제보", NOW + 1).unwrap() else {
        panic!()
    };
    let receipt = submit(&store, &http, &retry).unwrap();
    assert_eq!(receipt.receipt_id, prepared.id);
    assert_eq!(
        http.routes
            .borrow()
            .iter()
            .filter(|r| r.starts_with("auth/"))
            .count(),
        1
    );
    assert_eq!(http.bodies.borrow()[1], http.bodies.borrow()[2]);
    assert_eq!(http.bodies.borrow()[2]["p_payload"], prepared.payload);
    assert!(!http.bodies.borrow()[2]
        .to_string()
        .contains("private-access"));
    drafts.finish(id, Some(receipt));
    assert!(matches!(
        drafts.claim(id, "첫 제보", NOW + 2),
        Ok(Claim::Complete(_))
    ));
    assert_eq!(http.routes.borrow().len(), 3);
}

#[test]
fn feedback_uses_same_main_origin_gate_and_never_accepts_embedded_remote_webviews() {
    let dev = "http://127.0.0.1:14200".parse().unwrap();
    assert!(crate::frontend_origin::trusted_main(
        "main",
        &dev,
        true,
        Some(&dev)
    ));
    for (label, url) in [
        ("official-browser", "https://chzzk.naver.com"),
        ("main", "https://attacker.test"),
        ("main", "http://127.0.0.1:14201"),
    ] {
        assert!(!crate::frontend_origin::trusted_main(
            label,
            &url.parse().unwrap(),
            true,
            Some(&dev)
        ));
    }
}
