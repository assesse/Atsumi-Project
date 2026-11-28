use super::*;
use serde_json::{json, Value};
use std::{collections::VecDeque, sync::atomic::AtomicUsize};
use upload::{Request, Response, Transport};

#[cfg(all(windows, debug_assertions))]
#[path = "native_private_upload_qa.rs"]
mod native_private_upload_qa;

#[cfg(all(windows, debug_assertions))]
#[path = "native_private_owner_probe_qa.rs"]
mod native_private_owner_probe_qa;

#[cfg(all(windows, debug_assertions))]
#[path = "native_private_upload_preflight_qa.rs"]
mod native_private_upload_preflight_qa;

#[cfg(all(windows, debug_assertions))]
#[path = "native_soop_private_upload_qa.rs"]
mod native_soop_private_upload_qa;

#[cfg(all(windows, debug_assertions))]
#[path = "native_chzzk_upload_qa.rs"]
mod native_chzzk_upload_qa;

#[cfg(all(windows, debug_assertions))]
#[path = "native_soop_gift_upload_qa.rs"]
mod native_soop_gift_upload_qa;

#[cfg(all(windows, debug_assertions))]
#[path = "native_live_readiness_qa.rs"]
mod native_live_readiness_qa;

#[path = "automatic_tests.rs"]
mod automatic_tests;

#[path = "replay_tests.rs"]
mod replay_tests;

const OWNER: &str = "UCaaaaaaaaaaaaaaaaaaaaaa";
const OTHER: &str = "UCbbbbbbbbbbbbbbbbbbbbbb";
const VIDEO: &str = "abcdefghijk";
const SESSION: &str = "https://www.googleapis.com/upload/youtube/v3/videos?uploadType=resumable&upload_id=FIXTURE_SESSION_SECRET";
const ACCESS: &str = "FIXTURE_ACCESS_SECRET";
const REFRESH: &str = "FIXTURE_REFRESH_SECRET";
const CLIENT_SECRET: &str = "FIXTURE_CLIENT_SECRET";

#[derive(Default)]
struct Mock {
    responses: Mutex<VecDeque<Result<Response, CloudError>>>,
    requests: Mutex<Vec<Request>>,
}
impl Mock {
    fn new(responses: Vec<Result<Response, CloudError>>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
        })
    }
    fn push(&self, response: Result<Response, CloudError>) {
        self.responses.lock().unwrap().push_back(response);
    }
}
impl Transport for Mock {
    fn send(&self, request: Request) -> Result<Response, CloudError> {
        self.requests.lock().unwrap().push(request);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected fixture HTTP request")
    }
}
fn response(status: u16, value: Value) -> Result<Response, CloudError> {
    Ok(Response {
        status,
        headers: Vec::new(),
        body: serde_json::to_vec(&value).unwrap(),
    })
}
fn opened() -> Result<Response, CloudError> {
    Ok(Response {
        status: 200,
        headers: vec![("location".into(), SESSION.into())],
        body: Vec::new(),
    })
}
fn offset(end: Option<u64>) -> Result<Response, CloudError> {
    Ok(Response {
        status: 308,
        headers: end
            .map(|end| vec![("range".into(), format!("bytes=0-{end}"))])
            .unwrap_or_default(),
        body: Vec::new(),
    })
}
fn complete() -> Result<Response, CloudError> {
    complete_for(VIDEO)
}
fn complete_for(id: &str) -> Result<Response, CloudError> {
    response(200, json!({"id":id}))
}
fn facts(owner: &str, privacy: &str, duration: &str, state: &str) -> Result<Response, CloudError> {
    facts_for(VIDEO, owner, privacy, duration, state)
}
fn facts_for(
    id: &str,
    owner: &str,
    privacy: &str,
    duration: &str,
    state: &str,
) -> Result<Response, CloudError> {
    response(
        200,
        json!({"items":[{"id":id,"snippet":{"channelId":owner},"status":{"privacyStatus":privacy,"uploadStatus":if state == "succeeded" {"processed"} else {"uploaded"}},"contentDetails":{"duration":duration},"processingDetails":{"processingStatus":state}}]}),
    )
}
fn status_facts(
    id: &str,
    owner: &str,
    privacy: &str,
    duration: &str,
    upload: Option<&str>,
    processing: Option<&str>,
) -> Result<Response, CloudError> {
    let mut item = json!({
        "id": id,
        "snippet": {"channelId": owner},
        "status": {"privacyStatus": privacy},
        "contentDetails": {"duration": duration}
    });
    if let Some(upload) = upload {
        item["status"]["uploadStatus"] = json!(upload);
    }
    if let Some(processing) = processing {
        item["processingDetails"] = json!({"processingStatus": processing});
    }
    response(200, json!({"items": [item]}))
}
fn client() -> oauth::ClientConfig {
    oauth::ClientConfig {
        client_id: "123-fixture.apps.googleusercontent.com".into(),
        client_secret: CLIENT_SECRET.into(),
    }
}
fn grant() -> oauth::Grant {
    oauth::Grant {
        access_token: ACCESS.into(),
        refresh_token: REFRESH.into(),
        expires_at: now() + 3_600_000,
        channel_id: OWNER.into(),
        channel_name: "Fixture channel".into(),
    }
}
fn reference() -> CloudSourceRef {
    CloudSourceRef {
        provider: "soop".into(),
        recording_id: "fixture-recording".into(),
        part_index: None,
        plan_id: None,
        unit_id: None,
    }
}
fn fixture_source(path: PathBuf) -> SourceResolver {
    Arc::new(move |source, _| {
        Ok(CloudSource {
            file: File::open(&path).map_err(|_| "fixture missing")?,
            source: source.clone(),
            title: "Fixture recording".into(),
            duration_seconds: 44.0,
            media_offset_seconds: 0.0,
            lease: Box::new(()),
        })
    })
}
fn make_service(root: &Path, transport: Arc<dyn Transport>) -> Arc<CloudService> {
    make_service_with_source(root, transport, fixture_source(root.join("source.mp4")))
}
fn make_service_with_source(
    root: &Path,
    transport: Arc<dyn Transport>,
    resolver: SourceResolver,
) -> Arc<CloudService> {
    let service = CloudService::with_transport(
        root.join("app"),
        resolver,
        Arc::new(|_| Err("must not open real browser".into())),
        transport,
    )
    .unwrap();
    let mut inner = service.lock().unwrap();
    let mut candidate = inner.catalog.clone();
    candidate.client = Some(client());
    candidate.grant = Some(grant());
    service.publish(&mut inner, candidate).unwrap();
    drop(inner);
    service
}
fn install_job(service: &CloudService) -> Job {
    let mut inner = service.lock().unwrap();
    let at = now();
    let job = Job {
        view: CloudJobSnapshot {
            id: uuid::Uuid::new_v4().to_string(),
            source: reference(),
            title: "Fixture".into(),
            state: "queued".into(),
            visibility: CloudVisibility::Unlisted,
            recording_plan: false,
            uploaded_bytes: 0,
            total_bytes: 0,
            video_id: None,
            replay_cohort_id: None,
            duration_seconds: 0.0,
            media_offset_seconds: 0.0,
            error: None,
            retry_at: None,
            preparation: None,
            created_at: at,
            updated_at: at,
        },
        channel_id: OWNER.into(),
        source_sha256: None,
        session_url: None,
        attempts: 0,
        processing_started_at: None,
        automatic: None,
    };
    let mut candidate = inner.catalog.clone();
    candidate.jobs.push(job.clone());
    service.publish(&mut inner, candidate).unwrap();
    job
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("source.mp4"),
        b"neutral owned recording fixture bytes",
    )
    .unwrap();
    root
}
fn wait_done(service: &CloudService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !service.active_ids().is_empty() || {
        let inner = service.lock().unwrap();
        inner.persistence_error.is_none()
            && inner.worker_running
            && inner.catalog.jobs.iter().any(|job| {
                job.view.state == "queued"
                    || matches!(job.view.state.as_str(), "processing" | "retry_wait")
                        && job.view.retry_at.is_none_or(|at| at <= now())
            })
    } {
        assert!(Instant::now() < deadline, "fixture worker did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn oauth_pkce_matches_rfc7636_vector() {
    assert_eq!(
        oauth::pkce("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}
#[test]
fn desktop_client_import_is_exact_and_does_not_accept_web_or_custom_endpoints() {
    let good = json!({"installed":{"client_id":client().client_id,"client_secret":CLIENT_SECRET,"auth_uri":"https://accounts.google.com/o/oauth2/auth","token_uri":"https://oauth2.googleapis.com/token"}});
    assert!(oauth::ClientConfig::parse(&serde_json::to_vec(&good).unwrap()).is_ok());
    for bad in [
        json!({"web":good["installed"]}),
        json!({"installed":{"client_id":client().client_id,"client_secret":CLIENT_SECRET,"token_uri":"https://evil.example/token"}}),
        json!({"installed":{"client_id":"foreign.example","client_secret":CLIENT_SECRET}}),
    ] {
        let error = oauth::ClientConfig::parse(&serde_json::to_vec(&bad).unwrap())
            .err()
            .unwrap();
        assert_eq!(error.code, "CLOUD_CLIENT_FILE");
        assert!(!error.to_string().contains(CLIENT_SECRET));
    }
}
#[test]
fn oauth_callback_requires_exact_loopback_path_state_and_single_code() {
    let make = |path: &str, host: &str| format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n");
    let good = make(
        "/oauth2/callback?code=fixture-code&state=fixture-state",
        "127.0.0.1:32123",
    );
    assert_eq!(
        oauth::parse_callback(&good, 32123, "fixture-state").unwrap(),
        Some("fixture-code".into())
    );
    for bad in [
        make(
            "/oauth2/callback?code=fixture-code&state=wrong",
            "127.0.0.1:32123",
        ),
        make(
            "/oauth2/callback?code=one&code=two&state=fixture-state",
            "127.0.0.1:32123",
        ),
        make(
            "/oauth2/callback?code=one&state=fixture-state&state=fixture-state",
            "127.0.0.1:32123",
        ),
        make("/other?code=one&state=fixture-state", "127.0.0.1:32123"),
        make(
            "/oauth2/callback?code=one&state=fixture-state",
            "localhost:32123",
        ),
        good.replace("GET ", "POST "),
        good.replace("\r\n\r\n", "\r\nContent-Length: 5\r\n\r\nhello"),
        good.replace("\r\n\r\n", "\r\nHost: 127.0.0.1:32123\r\n\r\n"),
    ] {
        assert!(oauth::parse_callback(&bad, 32123, "fixture-state").is_err());
    }
    assert_eq!(
        oauth::parse_callback(
            &make(
                "/oauth2/callback?error=access_denied&state=fixture-state",
                "127.0.0.1:32123"
            ),
            32123,
            "fixture-state"
        )
        .unwrap_err()
        .code,
        "CLOUD_AUTH_DENIED"
    );
}
#[test]
fn resumable_urls_do_not_allow_credential_forwarding_to_arbitrary_hosts() {
    assert!(upload::valid_session_url(SESSION));
    for bad in [
        SESSION.replace("https:", "http:"),
        SESSION.replace("www.googleapis.com", "www.googleapis.com.evil.example"),
        SESSION.replace("www.googleapis.com", "user@www.googleapis.com"),
        SESSION.replace("www.googleapis.com", "www.googleapis.com:444"),
        format!("{SESSION}#secret"),
        format!("{SESSION}&upload_id=another"),
        "https://www.googleapis.com/youtube/v3/videos?upload_id=x".into(),
    ] {
        assert!(!upload::valid_session_url(&bad));
    }
}
#[test]
fn acknowledgments_are_server_positions_not_attempted_bytes() {
    assert_eq!(
        upload::progress(offset(None).unwrap(), 10).unwrap(),
        upload::Progress::Offset(0)
    );
    assert_eq!(
        upload::progress(offset(Some(4)).unwrap(), 10).unwrap(),
        upload::Progress::Offset(5)
    );
    assert_eq!(
        upload::progress(complete().unwrap(), 10).unwrap(),
        upload::Progress::Complete(VIDEO.into())
    );
    assert!(upload::progress(offset(Some(10)).unwrap(), 10).is_err());
    for range in [
        "bytes=2-5",
        "bytes=0--1",
        "bytes=0-18446744073709551615",
        "bytes=0-5,6-8",
    ] {
        assert!(upload::progress(
            Response {
                status: 308,
                headers: vec![("range".into(), range.into())],
                body: Vec::new()
            },
            10
        )
        .is_err());
    }
}
#[test]
fn upload_metadata_explicitly_sets_unlisted_and_chunks_use_exact_range() {
    let mock = Mock::new(vec![opened(), offset(Some(2))]);
    let session = upload::create_session(mock.as_ref(), ACCESS, "Fixture", 3, "reference", CloudVisibility::Unlisted).unwrap();
    assert_eq!(
        upload::send_chunk(mock.as_ref(), ACCESS, &session, 0, 3, vec![1, 2, 3]).unwrap(),
        upload::Progress::Offset(3)
    );
    let requests = mock.requests.lock().unwrap();
    let metadata: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(metadata["status"]["privacyStatus"], "unlisted");
    let upload_url = reqwest::Url::parse(&requests[0].url).unwrap();
    assert_eq!(upload_url.query_pairs().filter(|(key, _)| key == "notifySubscribers")
        .map(|(_, value)| value.into_owned()).collect::<Vec<_>>(), ["false"]);
    assert!(requests[1]
        .headers
        .contains(&("Content-Range".into(), "bytes 0-2/3".into())));
    assert!(upload::send_chunk(mock.as_ref(), ACCESS, &session, 0, 1000, vec![0; 3]).is_err());
}
#[test]
fn persisted_visibility_controls_metadata_and_remote_readiness_without_public_fallback() {
    for expected in [CloudVisibility::Private, CloudVisibility::Unlisted] {
        let mock = Mock::new(vec![opened()]);
        upload::create_session(mock.as_ref(), ACCESS, "Fixture", 3, "reference", expected)
            .unwrap();
        let requests = mock.requests.lock().unwrap();
        let metadata: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(metadata["status"]["privacyStatus"], expected.as_str());
        drop(requests);
        for remote in ["private", "unlisted", "public"] {
            let mock = Mock::new(vec![facts(OWNER, remote, "PT44S", "succeeded")]);
            let result = upload::verify_remote(mock.as_ref(), ACCESS, VIDEO, OWNER, 44.0, expected);
            if remote == expected.as_str() {
                assert!(result.unwrap());
            } else {
                assert_eq!(result.unwrap_err().code, "CLOUD_PRIVACY_MISMATCH");
            }
        }
    }
    assert!(serde_json::from_str::<CloudVisibility>("\"public\"").is_err());
    assert!(serde_json::from_str::<CloudVisibility>("\"unknown\"").is_err());
}

#[cfg(windows)]
#[test]
fn legacy_private_job_resumes_with_its_original_visibility_after_catalog_recovery() {
    let root = fixture();
    let service = make_service(root.path(), Mock::new(vec![]));
    let job = install_job(&service);
    let mut legacy = serde_json::to_value(service.lock().unwrap().catalog.clone()).unwrap();
    legacy["jobs"][0]["view"].as_object_mut().unwrap().remove("visibility");
    let decoded: Catalog = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(decoded.jobs[0].view.visibility, CloudVisibility::Private);
    assert!(decoded.valid());
    service.publish(&mut service.lock().unwrap(), decoded).unwrap();
    service.shutdown_and_wait().unwrap();
    drop(service);
    let mock = Mock::new(vec![opened(), offset(None), complete(), facts(OWNER, "private", "PT44S", "succeeded")]);
    let resumed = make_service(root.path(), mock.clone());
    assert_eq!(resumed.snapshot().unwrap().jobs[0].visibility, CloudVisibility::Private);
    resumed.resume(&job.view.id).unwrap();
    wait_until(|| resumed.snapshot().unwrap().jobs[0].state == "ready");
    wait_done(&resumed);
    assert_eq!(resumed.vault.load().unwrap().jobs[0].view.visibility, CloudVisibility::Private);
    let requests = mock.requests.lock().unwrap();
    let metadata: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(metadata["status"]["privacyStatus"], "private");
    drop(requests);
    resumed.shutdown_and_wait().unwrap();
    legacy["jobs"][0]["view"]["visibility"] = json!("public");
    assert!(serde_json::from_value::<Catalog>(legacy).is_err());
}
#[test]
fn remote_processing_owner_privacy_and_duration_are_distinct_from_playback() {
    for (owner, privacy, duration, state, expected) in [
        (OWNER, "private", "PT44S", "succeeded", None),
        (
            OTHER,
            "private",
            "PT44S",
            "succeeded",
            Some("CLOUD_OWNER_MISMATCH"),
        ),
        (
            OWNER,
            "unlisted",
            "PT44S",
            "succeeded",
            Some("CLOUD_PRIVACY_MISMATCH"),
        ),
        (
            OWNER,
            "public",
            "PT44S",
            "succeeded",
            Some("CLOUD_PRIVACY_MISMATCH"),
        ),
        (
            OWNER,
            "private",
            "PT30S",
            "succeeded",
            Some("CLOUD_DURATION_MISMATCH"),
        ),
        (
            OWNER,
            "private",
            "PT44S",
            "failed",
            Some("CLOUD_PROCESSING_FAILED"),
        ),
    ] {
        let mock = Mock::new(vec![facts(owner, privacy, duration, state)]);
        let result = upload::verify_remote(mock.as_ref(), ACCESS, VIDEO, OWNER, 44.0, CloudVisibility::Private);
        match expected {
            None => assert!(result.unwrap()),
            Some(code) => assert_eq!(result.unwrap_err().code, code),
        }
    }
    let mock = Mock::new(vec![
        facts(OWNER, "private", "PT44S", "processing"),
        response(200, json!({"items":[]})),
    ]);
    assert!(!upload::verify_remote(mock.as_ref(), ACCESS, VIDEO, OWNER, 44.0, CloudVisibility::Private).unwrap());
    assert!(!upload::verify_remote(mock.as_ref(), ACCESS, VIDEO, OWNER, 44.0, CloudVisibility::Private).unwrap());
}
#[test]
fn terminated_processing_status_requires_processed_upload_and_never_masks_failure() {
    for (upload, processing, ready) in [
        (Some("processed"), Some("succeeded"), true),
        (Some("processed"), Some("terminated"), true),
        (Some("uploaded"), Some("terminated"), false),
        (Some("uploaded"), Some("succeeded"), false),
        (Some("processed"), Some("processing"), false),
        (Some("uploaded"), Some("processing"), false),
        (Some("processed"), Some("unknown"), false),
        (Some("processed"), None, false),
        (Some("unknown"), Some("terminated"), false),
        (None, Some("terminated"), false),
        (None, None, false),
    ] {
        let mock = Mock::new(vec![status_facts(
            VIDEO, OWNER, "private", "PT44S", upload, processing,
        )]);
        assert_eq!(
            upload::verify_remote(mock.as_ref(), ACCESS, VIDEO, OWNER, 44.0, CloudVisibility::Private).unwrap(),
            ready,
            "upload={upload:?}, processing={processing:?}"
        );
    }
    for upload in ["failed", "rejected", "deleted"] {
        for processing in ["succeeded", "terminated", "processing", "unknown"] {
            let mock = Mock::new(vec![status_facts(
                VIDEO,
                OWNER,
                "private",
                "PT44S",
                Some(upload),
                Some(processing),
            )]);
            let error =
                upload::verify_remote(mock.as_ref(), ACCESS, VIDEO, OWNER, 44.0, CloudVisibility::Private).unwrap_err();
            assert_eq!(error.code, "CLOUD_PROCESSING_FAILED");
            assert!(!error.retryable);
        }
    }
    for upload in [Some("processed"), Some("uploaded"), Some("unknown"), None] {
        let mock = Mock::new(vec![status_facts(
            VIDEO,
            OWNER,
            "private",
            "PT44S",
            upload,
            Some("failed"),
        )]);
        assert_eq!(
            upload::verify_remote(mock.as_ref(), ACCESS, VIDEO, OWNER, 44.0, CloudVisibility::Private)
                .unwrap_err()
                .code,
            "CLOUD_PROCESSING_FAILED"
        );
    }
}

#[test]
fn terminated_processing_preserves_video_owner_privacy_and_duration_checks() {
    for (id, owner, privacy, duration, code) in [
        (
            "lmnopqrstuv",
            OWNER,
            "private",
            "PT44S",
            "CLOUD_OWNER_MISMATCH",
        ),
        (VIDEO, OTHER, "private", "PT44S", "CLOUD_OWNER_MISMATCH"),
        (VIDEO, OWNER, "public", "PT44S", "CLOUD_PRIVACY_MISMATCH"),
        (VIDEO, OWNER, "unlisted", "PT44S", "CLOUD_PRIVACY_MISMATCH"),
        (VIDEO, OWNER, "private", "PT30S", "CLOUD_DURATION_MISMATCH"),
    ] {
        let mock = Mock::new(vec![status_facts(
            id,
            owner,
            privacy,
            duration,
            Some("processed"),
            Some("terminated"),
        )]);
        assert_eq!(
            upload::verify_remote(mock.as_ref(), ACCESS, VIDEO, OWNER, 44.0, CloudVisibility::Private)
                .unwrap_err()
                .code,
            code
        );
    }
    let mock = Mock::new(vec![status_facts(
        VIDEO,
        OWNER,
        "private",
        "invalid",
        Some("processed"),
        Some("terminated"),
    )]);
    assert!(upload::verify_remote(mock.as_ref(), ACCESS, VIDEO, OWNER, 44.0, CloudVisibility::Private).is_err());
}

#[test]
fn duration_parser_handles_hours_minutes_fractional_seconds_without_ambiguity() {
    assert_eq!(upload::iso_duration("PT11H59M59S"), Some(43199.0));
    assert_eq!(upload::iso_duration("PT44.5S"), Some(44.5));
    for bad in [
        "44", "PT", "PT0S", "PT1S1H", "PT1H1H", "PTNaNS", "PT13H", "PT-2S", "PT1.2.3S",
    ] {
        assert!(upload::iso_duration(bad).is_none());
    }
}
#[test]
fn remote_error_bodies_never_leak_tokens_urls_or_arbitrary_message_text() {
    for status in [400, 401, 403, 429, 500] {
        let value = response(status,json!({"error":{"message":format!("{ACCESS} {SESSION}"),"errors":[{"reason":"arbitrary secret reason"}]}})).unwrap();
        let error = upload::http_error(&value);
        let json = serde_json::to_string(&error).unwrap();
        assert!(!json.contains(ACCESS));
        assert!(!json.contains("upload_id"));
        assert!(!json.contains("arbitrary"));
    }
    assert_eq!(
        upload::http_error(
            &response(
                403,
                json!({"error":{"errors":[{"reason":"uploadLimitExceeded"}]}})
            )
            .unwrap()
        )
        .code,
        "CLOUD_QUOTA"
    );
}

#[cfg(windows)]
#[test]
fn actual_dpapi_storage_is_ciphertext_and_public_snapshot_has_no_secrets() {
    let root = fixture();
    let service = make_service(root.path(), Mock::new(vec![]));
    let job = install_job(&service);
    service
        .update_job(&job.view.id, |job| job.session_url = Some(SESSION.into()))
        .unwrap();
    let text = serde_json::to_string(&service.snapshot().unwrap()).unwrap();
    let bytes = std::fs::read(root.path().join("app/cloud-recording/state.bin")).unwrap();
    for secret in [ACCESS, REFRESH, CLIENT_SECRET, "FIXTURE_SESSION_SECRET"] {
        assert!(!text.contains(secret));
        assert!(!bytes.windows(secret.len()).any(|b| b == secret.as_bytes()));
    }
    assert_eq!(
        service.vault.load().unwrap().jobs[0].session_url.as_deref(),
        Some(SESSION)
    );
    service.shutdown_and_wait().unwrap();
}
#[cfg(windows)]
#[test]
fn queue_worker_uploads_one_fixture_and_preserves_original_bytes() {
    let root = fixture();
    let original = std::fs::read(root.path().join("source.mp4")).unwrap();
    let mock = Mock::new(vec![
        opened(),
        offset(None),
        complete(),
        facts(OWNER, "unlisted", "PT44S", "succeeded"),
    ]);
    let service = make_service(root.path(), mock.clone());
    let snapshot = service.enqueue_with_visibility(reference(), CloudVisibility::Unlisted).unwrap();
    assert_eq!(snapshot.jobs.len(), 1);
    wait_done(&service);
    let snapshot = service.snapshot().unwrap();
    assert_eq!(snapshot.jobs[0].state, "ready");
    assert_eq!(snapshot.jobs[0].uploaded_bytes, original.len() as u64);
    assert_eq!(snapshot.jobs[0].video_id.as_deref(), Some(VIDEO));
    assert_eq!(
        std::fs::read(root.path().join("source.mp4")).unwrap(),
        original
    );
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("playbackVerified"));
    service.enqueue_with_visibility(reference(), CloudVisibility::Unlisted).unwrap();
    assert_eq!(service.snapshot().unwrap().jobs.len(), 1);
    service.shutdown_and_wait().unwrap();
    assert_eq!(mock.requests.lock().unwrap().len(), 4);
}
#[cfg(windows)]
#[test]
fn new_upload_defaults_private_and_explicit_unlisted_preserves_receipts_and_originals() {
    for visibility in [CloudVisibility::Private, CloudVisibility::Unlisted] {
        let root = fixture();
        let original = std::fs::read(root.path().join("source.mp4")).unwrap();
        let mock = Mock::new(vec![opened(), offset(None), complete(), facts(OWNER, visibility.as_str(), "PT44S", "succeeded")]);
        let service = make_service(root.path(), mock.clone());
        if visibility == CloudVisibility::Private { service.enqueue(reference()).unwrap(); }
        else { service.enqueue_with_visibility(reference(), visibility).unwrap(); }
        wait_until(|| service.snapshot().unwrap().jobs[0].state == "ready");
        wait_done(&service);
        let job = service.snapshot().unwrap().jobs.remove(0);
        assert_eq!(job.visibility, visibility);
        let metadata: Value = serde_json::from_slice(&mock.requests.lock().unwrap()[0].body).unwrap();
        assert_eq!(metadata["status"]["privacyStatus"], visibility.as_str());
        // A later click is not a remote visibility update or duplicate video.
        service.enqueue_with_visibility(reference(), if visibility == CloudVisibility::Private { CloudVisibility::Unlisted } else { CloudVisibility::Private }).unwrap();
        assert_eq!(service.snapshot().unwrap().jobs.len(), 1);
        assert_eq!(service.vault.load().unwrap().jobs[0].view.visibility, visibility);
        assert_eq!(mock.requests.lock().unwrap().len(), 4);
        assert_eq!(std::fs::read(root.path().join("source.mp4")).unwrap(), original);
        service.shutdown_and_wait().unwrap();
    }
}

#[cfg(windows)]
#[test]
fn planned_recording_default_private_is_inherited_by_every_unit() {
    let root = fixture();
    let mock = Mock::new(vec![opened(), offset(None), complete(), facts(OWNER,"private","PT44S","succeeded"),
        opened(), offset(None), complete_for("lmnopqrstuv"), facts_for("lmnopqrstuv",OWNER,"private","PT44S","succeeded")]);
    let service = make_planning_service(root.path(), mock.clone(), Arc::new(|source, _| Ok(planned_units(source))));
    service.enqueue_recording(reference()).unwrap();
    wait_until(|| { let jobs = service.snapshot().unwrap().jobs; jobs.len() == 2 && jobs.iter().all(|job| job.state == "ready") });
    wait_done(&service);
    assert!(service.vault.load().unwrap().jobs.iter().all(|job| job.view.visibility == CloudVisibility::Private));
    service.shutdown_and_wait().unwrap();
}
#[cfg(windows)]
#[test]
fn lost_final_response_restarts_by_querying_same_session_not_reuploading() {
    let root = fixture();
    let first = Mock::new(vec![
        opened(),
        offset(None),
        Err(CloudError::new(
            "CLOUD_NETWORK",
            "fixture interrupted",
            true,
        )),
    ]);
    let service = make_service(root.path(), first);
    let job = install_job(&service);
    assert_eq!(
        service
            .run_job(job, &Arc::new(AtomicBool::new(false)))
            .unwrap_err()
            .code,
        "CLOUD_NETWORK"
    );
    let persisted = service.vault.load().unwrap().jobs[0].clone();
    assert_eq!(persisted.view.uploaded_bytes, 0);
    assert_eq!(persisted.session_url.as_deref(), Some(SESSION));
    service.shutdown_and_wait().unwrap();
    drop(service);
    let second = Mock::new(vec![
        complete(),
        facts(OWNER, "unlisted", "PT44S", "succeeded"),
    ]);
    let resumed = make_service(root.path(), second.clone());
    assert_eq!(resumed.snapshot().unwrap().jobs[0].state, "paused");
    let job = resumed.lock().unwrap().catalog.jobs[0].clone();
    resumed
        .run_job(job, &Arc::new(AtomicBool::new(false)))
        .unwrap();
    let requests = second.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, reqwest::Method::PUT);
    assert_eq!(requests[0].url, SESSION);
    assert!(requests[0].body.is_empty());
    drop(requests);
    assert_eq!(resumed.snapshot().unwrap().jobs[0].state, "ready");
    resumed.shutdown_and_wait().unwrap();
}
#[cfg(windows)]
#[test]
fn expired_session_never_blindly_creates_a_duplicate_upload() {
    let root = fixture();
    let mock = Mock::new(vec![response(404, json!({}))]);
    let service = make_service(root.path(), mock.clone());
    let job = install_job(&service);
    service
        .update_job(&job.view.id, |j| j.session_url = Some(SESSION.into()))
        .unwrap();
    let job = service.lock().unwrap().catalog.jobs[0].clone();
    let error = service
        .run_job(job, &Arc::new(AtomicBool::new(false)))
        .unwrap_err();
    assert_eq!(error.code, "CLOUD_UPLOAD_SESSION_EXPIRED");
    assert!(!error.retryable);
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
    assert_eq!(
        mock.requests.lock().unwrap()[0].method,
        reqwest::Method::PUT
    );
    service.shutdown_and_wait().unwrap();
}
#[cfg(windows)]
#[test]
fn changed_source_is_rejected_before_sending_any_more_bytes() {
    let root = fixture();
    let mock = Mock::new(vec![
        opened(),
        offset(None),
        Err(CloudError::new("CLOUD_NETWORK", "fixture", true)),
    ]);
    let service = make_service(root.path(), mock.clone());
    let job = install_job(&service);
    service
        .run_job(job, &Arc::new(AtomicBool::new(false)))
        .unwrap_err();
    std::fs::write(root.path().join("source.mp4"), b"changed fixture bytes").unwrap();
    let count = mock.requests.lock().unwrap().len();
    let job = service.lock().unwrap().catalog.jobs[0].clone();
    assert_eq!(
        service
            .run_job(job, &Arc::new(AtomicBool::new(false)))
            .unwrap_err()
            .code,
        "CLOUD_SOURCE_CHANGED"
    );
    assert_eq!(mock.requests.lock().unwrap().len(), count);
    service.shutdown_and_wait().unwrap();
}
#[cfg(windows)]
#[test]
fn enqueue_persistence_failure_has_no_phantom_queued_job_or_active_work() {
    let root = fixture();
    let mock = Mock::new(vec![]);
    let service = make_service(root.path(), mock.clone());
    service.vault.fail_once();
    assert_eq!(
        service.enqueue_with_visibility(reference(), CloudVisibility::Unlisted).unwrap_err().code,
        "CLOUD_STORAGE"
    );
    assert!(service.snapshot().unwrap().jobs.is_empty());
    assert!(service.active_ids().is_empty());
    assert!(mock.requests.lock().unwrap().is_empty());
    assert_eq!(
        service.enqueue_with_visibility(reference(), CloudVisibility::Unlisted).unwrap_err().code,
        "CLOUD_STORAGE"
    );
    assert!(service.vault.load().unwrap().jobs.is_empty());
    assert_eq!(
        service.shutdown_and_wait().unwrap_err().code,
        "CLOUD_STORAGE"
    );
}
#[cfg(windows)]
#[test]
fn ready_persistence_failure_never_exposes_ready_or_changes_durable_state() {
    let root = fixture();
    let service = make_service(root.path(), Mock::new(vec![]));
    let job = install_job(&service);
    service
        .update_job(&job.view.id, |job| {
            job.view.state = "processing".into();
            job.view.total_bytes = 1;
            job.view.uploaded_bytes = 1;
            job.view.video_id = Some(VIDEO.into());
            job.view.duration_seconds = 44.0;
            job.source_sha256 = Some("a".repeat(64));
        })
        .unwrap();
    service.vault.fail_once();
    assert_eq!(
        service
            .update_job(&job.view.id, |job| job.view.state = "ready".into())
            .unwrap_err()
            .code,
        "CLOUD_STORAGE"
    );
    assert_eq!(service.snapshot().unwrap().jobs[0].state, "failed");
    assert_eq!(
        service.vault.load().unwrap().jobs[0].view.state,
        "processing"
    );
    assert!(service.active_ids().is_empty());
    service.shutdown_and_wait().unwrap_err();
}
#[cfg(windows)]
#[test]
fn refresh_persistence_failure_does_not_publish_or_use_new_access_token() {
    let root = fixture();
    let mock = Mock::new(vec![response(
        200,
        json!({"access_token":"NEW_FIXTURE_ACCESS","token_type":"Bearer","expires_in":3600}),
    )]);
    let service = make_service(root.path(), mock);
    service.vault.fail_once();
    assert_eq!(
        service.token(OWNER, true).unwrap_err().code,
        "CLOUD_STORAGE"
    );
    assert_eq!(
        service
            .lock()
            .unwrap()
            .catalog
            .grant
            .as_ref()
            .unwrap()
            .access_token,
        ACCESS
    );
    assert_eq!(
        service.vault.load().unwrap().grant.unwrap().access_token,
        ACCESS
    );
    service.shutdown_and_wait().unwrap_err();
}
#[cfg(windows)]
#[test]
fn reconnecting_another_channel_cannot_resume_existing_job() {
    let root = fixture();
    let service = make_service(root.path(), Mock::new(vec![]));
    let job = install_job(&service);
    {
        let mut inner = service.lock().unwrap();
        let mut candidate = inner.catalog.clone();
        candidate.grant.as_mut().unwrap().channel_id = OTHER.into();
        service.publish(&mut inner, candidate).unwrap();
    }
    assert_eq!(
        service.resume(&job.view.id).unwrap_err().code,
        "CLOUD_ACCOUNT_MISMATCH"
    );
    service.shutdown_and_wait().unwrap();
}
#[cfg(windows)]
#[test]
fn source_lease_lives_through_upload_attempt_and_is_released_on_failure() {
    struct Lease(Arc<AtomicUsize>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let root = fixture();
    let dropped = Arc::new(AtomicUsize::new(0));
    let expected = dropped.clone();
    let path = root.path().join("source.mp4");
    let resolver: SourceResolver = Arc::new(move |reference, _| {
        Ok(CloudSource {
            file: File::open(&path).unwrap(),
            source: reference.clone(),
            title: "fixture".into(),
            duration_seconds: 44.0,
            media_offset_seconds: 0.0,
            lease: Box::new(Lease(expected.clone())),
        })
    });
    let service = CloudService::with_transport(
        root.path().join("app"),
        resolver,
        Arc::new(|_| Err("unused".into())),
        Mock::new(vec![Err(CloudError::new("CLOUD_NETWORK", "fixture", true))]),
    )
    .unwrap();
    {
        let mut inner = service.lock().unwrap();
        let mut candidate = inner.catalog.clone();
        candidate.client = Some(client());
        candidate.grant = Some(grant());
        service.publish(&mut inner, candidate).unwrap();
    }
    let job = install_job(&service);
    service
        .run_job(job, &Arc::new(AtomicBool::new(false)))
        .unwrap_err();
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    service.shutdown_and_wait().unwrap();
}
#[cfg(windows)]
#[test]
fn authorization_cancel_retains_work_until_thread_finishes() {
    let root = fixture();
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let in_opener = entered.clone();
    let out_opener = release.clone();
    let opener: BrowserOpener = Arc::new(move |_| {
        in_opener.store(true, Ordering::SeqCst);
        while !out_opener.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Err("fixture refused".into())
    });
    let service = CloudService::with_transport(
        root.path().join("app"),
        fixture_source(root.path().join("source.mp4")),
        opener,
        Mock::new(vec![]),
    )
    .unwrap();
    {
        let mut inner = service.lock().unwrap();
        let mut candidate = inner.catalog.clone();
        candidate.client = Some(client());
        service.publish(&mut inner, candidate).unwrap();
    }
    service.authorize().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !entered.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    service.disconnect().unwrap();
    assert!(service.active_ids().contains(&"cloud:authorization".into()));
    release.store(true, Ordering::SeqCst);
    wait_done(&service);
    assert!(!service.snapshot().unwrap().account.connected);
    service.shutdown_and_wait().unwrap();
}
#[test]
fn impossible_ready_catalog_and_cross_path_source_are_rejected() {
    let bad = CloudSourceRef {
        provider: "soop".into(),
        recording_id: "../../other".into(),
        part_index: None,
        plan_id: None,
        unit_id: None,
    };
    assert!(!bad.valid());
    let mut catalog = Catalog::default();
    catalog.grant = Some(grant());
    assert!(!catalog.valid());
    catalog.client = Some(client());
    assert!(catalog.valid());
}

#[cfg(windows)]
#[test]
fn invalid_refresh_grant_requires_reconnection_without_changing_source_or_web_profile() {
    let root = fixture();
    let original = std::fs::read(root.path().join("source.mp4")).unwrap();
    let mock = Mock::new(vec![response(
        400,
        json!({"error":"invalid_grant","error_description":REFRESH}),
    )]);
    let service = make_service(root.path(), mock);
    install_job(&service);
    let retrying = install_job(&service);
    service
        .update_job(&retrying.view.id, |job| {
            job.view.state = "retry_wait".into();
            job.view.retry_at = Some(now() + 60_000);
        })
        .unwrap();
    let error = service.token(OWNER, true).unwrap_err();
    assert_eq!(error.code, "CLOUD_AUTH_REQUIRED");
    assert!(error.message.contains("같은 YouTube 계정"));
    assert!(!service.snapshot().unwrap().account.connected);
    assert!(service.vault.load().unwrap().grant.is_none());
    assert!(service
        .snapshot()
        .unwrap()
        .jobs
        .iter()
        .all(|job| job.state == "waiting_auth" && job.retry_at.is_none()));
    assert!(service
        .vault
        .load()
        .unwrap()
        .jobs
        .iter()
        .all(|job| job.view.state == "waiting_auth"));
    assert!(
        service.active_ids().is_empty(),
        "authorization wait is not running work"
    );
    assert_eq!(
        std::fs::read(root.path().join("source.mp4")).unwrap(),
        original
    );
    assert!(!serde_json::to_string(&service.snapshot().unwrap())
        .unwrap()
        .contains(REFRESH));
    service.shutdown_and_wait().unwrap();
}

#[test]
fn refresh_auth_errors_distinguish_client_setup_and_account_access_without_provider_text() {
    for (reason, help) in [("invalid_client", "데스크톱 앱 설정 파일"), ("unauthorized_client", "데스크톱 앱 설정 파일"), ("access_denied", "허용 계정")] {
        let mock = Mock::new(vec![response(400, json!({"error":reason,"error_description":REFRESH}))]);
        let error = oauth::refresh(mock.as_ref(), &client(), &grant()).err().unwrap();
        assert_eq!(error.code, "CLOUD_AUTH_REQUIRED");
        assert!(error.message.contains(help));
        assert!(!error.message.contains(REFRESH));
    }
}

#[cfg(windows)]
#[test]
fn expired_upload_account_waits_before_opening_or_preparing_a_recording() {
    for planned in [false, true] {
        let root = fixture();
        let mock = Mock::new(vec![response(400, json!({"error":"invalid_grant"}))]);
        let source_calls = Arc::new(AtomicUsize::new(0));
        let service = if planned {
            let calls = source_calls.clone();
            make_planning_service(root.path(), mock.clone(), Arc::new(move |source, _| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(planned_units(source))
            }))
        } else {
            make_service_with_source(root.path(), mock.clone(), rejecting_source(source_calls.clone()))
        };
        {
            let mut inner = service.lock().unwrap();
            let mut candidate = inner.catalog.clone();
            candidate.grant.as_mut().unwrap().expires_at = 0;
            service.publish(&mut inner, candidate).unwrap();
        }
        let started = if planned {
            service.enqueue_recording(reference())
        } else {
            service.enqueue(reference())
        }.unwrap();
        wait_until(|| !service.lock().unwrap().worker_running);
        let saved = service.vault.load().unwrap();
        assert!(saved.grant.is_none());
        assert_eq!(saved.jobs.len(), 1);
        let job = &saved.jobs[0];
        assert_eq!(job.view.id, started.jobs[0].id);
        assert_eq!(job.view.state, "waiting_auth");
        assert_eq!(job.view.visibility, CloudVisibility::Private);
        assert_eq!(job.view.recording_plan, planned);
        assert!(job.source_sha256.is_none() && job.session_url.is_none());
        assert_eq!(source_calls.load(Ordering::SeqCst), 0);
        {
            let requests = mock.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].url, "https://oauth2.googleapis.com/token");
        }
        service.shutdown_and_wait().unwrap();
    }
}

#[cfg(windows)]
#[test]
fn disconnect_preserves_late_upload_acknowledgements_and_same_job_resumes() {
    for acknowledged in [0usize, 4] {
        let root = fixture();
        let original = std::fs::read(root.path().join("source.mp4")).unwrap();
        let mut replies = vec![opened()];
        if acknowledged > 0 {
            replies.push(offset(None));
            replies.push(offset(Some(3)));
        }
        replies.push(offset(acknowledged.checked_sub(1).map(|last| last as u64)));
        replies.push(complete());
        replies.push(facts(OWNER, "private", "PT44S", "succeeded"));
        let mock = Mock::new(replies);
        let entered = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let service = make_service(root.path(), Arc::new(GatedTransport {
            mock: mock.clone(),
            gate: if acknowledged == 0 {
                |request| request.method == reqwest::Method::POST
            } else {
                |request| request.method == reqwest::Method::PUT && !request.body.is_empty()
            },
            entered: entered.clone(),
            release: release.clone(),
        }));
        let id = service.enqueue(reference()).unwrap().jobs[0].id.clone();
        wait_until(|| entered.load(Ordering::SeqCst));
        service.disconnect().unwrap();
        release.store(true, Ordering::SeqCst);
        wait_until(|| !service.lock().unwrap().worker_running);
        let held = service.vault.load().unwrap();
        assert_eq!(held.jobs.len(), 1);
        let job = &held.jobs[0];
        assert_eq!(job.view.id, id);
        assert_eq!(job.view.state, "waiting_auth");
        assert_eq!(job.view.error.as_ref().unwrap().code, "CLOUD_AUTH_REQUIRED");
        assert_eq!(job.session_url.as_deref(), Some(SESSION));
        assert_eq!(job.view.uploaded_bytes, acknowledged as u64);
        assert!(job.view.video_id.is_none());
        service.shutdown_and_wait().unwrap();
        drop(service);

        let resumed = make_service(root.path(), mock.clone());
        resumed.resume(&id).unwrap();
        wait_done(&resumed);
        let snapshot = resumed.snapshot().unwrap();
        assert_eq!(snapshot.jobs.len(), 1);
        assert_eq!(snapshot.jobs[0].id, id);
        assert_eq!(snapshot.jobs[0].state, "ready");
        assert_eq!(snapshot.jobs[0].visibility, CloudVisibility::Private);
        assert_eq!(snapshot.jobs[0].video_id.as_deref(), Some(VIDEO));
        {
            let requests = mock.requests.lock().unwrap();
            assert_eq!(requests.iter().filter(|r| r.method == reqwest::Method::POST).count(), 1);
            let last_chunk = requests.iter().rev().find(|r| r.method == reqwest::Method::PUT && !r.body.is_empty()).unwrap();
            assert_eq!(last_chunk.body.as_slice(), &original[acknowledged..]);
        }
        assert_eq!(std::fs::read(root.path().join("source.mp4")).unwrap(), original);
        resumed.shutdown_and_wait().unwrap();
    }
}

#[cfg(windows)]
#[test]
fn normal_shutdown_joins_inflight_worker_and_preserves_received_final_ack() {
    struct BlockingFinal {
        mock: Arc<Mock>,
        entered: Arc<AtomicBool>,
        release: Arc<AtomicBool>,
    }
    impl Transport for BlockingFinal {
        fn send(&self, request: Request) -> Result<Response, CloudError> {
            if request.method == reqwest::Method::PUT && !request.body.is_empty() {
                self.entered.store(true, Ordering::SeqCst);
                let deadline = Instant::now() + Duration::from_secs(10);
                while !self.release.load(Ordering::SeqCst) {
                    if Instant::now() >= deadline {
                        return Err(CloudError::new("CLOUD_NETWORK", "fixture timeout", true));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            self.mock.send(request)
        }
    }
    let root = fixture();
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let transport = Arc::new(BlockingFinal {
        mock: Mock::new(vec![opened(), offset(None), complete()]),
        entered: entered.clone(),
        release: release.clone(),
    });
    let service = make_service(root.path(), transport);
    service.enqueue_with_visibility(reference(), CloudVisibility::Unlisted).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !entered.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let closing = service.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let finished_thread = finished.clone();
    let shutdown = std::thread::spawn(move || {
        let result = closing.shutdown_and_wait();
        finished_thread.store(true, Ordering::SeqCst);
        result
    });
    while !service.lock().unwrap().closing {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!finished.load(Ordering::SeqCst));
    assert!(!service.active_ids().is_empty());
    release.store(true, Ordering::SeqCst);
    shutdown.join().unwrap().unwrap();
    assert!(service.active_ids().is_empty());
    assert_eq!(service.snapshot().unwrap().jobs[0].state, "paused");
    assert_eq!(
        service.snapshot().unwrap().jobs[0].video_id.as_deref(),
        Some(VIDEO)
    );
    let id = service.snapshot().unwrap().jobs[0].id.clone();
    drop(service);
    let mock = Mock::new(vec![facts(OWNER, "unlisted", "PT44S", "succeeded")]);
    let resumed = make_service(root.path(), mock.clone());
    resumed.resume(&id).unwrap();
    wait_done(&resumed);
    assert_eq!(resumed.snapshot().unwrap().jobs[0].state, "ready");
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, reqwest::Method::GET);
    assert!(requests[0].body.is_empty());
    drop(requests);
    resumed.shutdown_and_wait().unwrap();
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "fixture condition was not reached"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn install_processing_receipt(service: &CloudService, video: &str, started: u64) -> Job {
    let job = install_job(service);
    service
        .update_job(&job.view.id, |current| {
            current.view.source.recording_id = format!("fixture-{}", current.view.id);
            current.view.state = "processing".into();
            current.view.video_id = Some(video.into());
            current.view.total_bytes = 1;
            current.view.uploaded_bytes = 1;
            current.view.duration_seconds = 44.0;
            current.source_sha256 = Some("a".repeat(64));
            current.processing_started_at = Some(started);
        })
        .unwrap();
    service
        .lock()
        .unwrap()
        .catalog
        .jobs
        .iter()
        .find(|current| current.view.id == job.view.id)
        .unwrap()
        .clone()
}
fn rejecting_source(calls: Arc<AtomicUsize>) -> SourceResolver {
    Arc::new(move |_, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        Err("source must not be reopened for remote processing".into())
    })
}
struct GatedTransport {
    mock: Arc<Mock>,
    gate: fn(&Request) -> bool,
    entered: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
}
impl Transport for GatedTransport {
    fn send(&self, request: Request) -> Result<Response, CloudError> {
        if (self.gate)(&request) && !self.entered.swap(true, Ordering::SeqCst) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !self.release.load(Ordering::SeqCst) {
                if Instant::now() >= deadline {
                    return Err(CloudError::new("CLOUD_NETWORK", "fixture timeout", true));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        self.mock.send(request)
    }
}

#[cfg(windows)]
#[test]
fn processing_first_video_yields_worker_to_second_upload_and_keeps_durable_due_time() {
    const SECOND: &str = "lmnopqrstuv";
    let root = fixture();
    let mock = Mock::new(vec![
        opened(),
        offset(None),
        complete(),
        facts(OWNER, "unlisted", "PT44S", "processing"),
        opened(),
        offset(None),
        complete_for(SECOND),
        facts_for(SECOND, OWNER, "unlisted", "PT44S", "succeeded"),
    ]);
    let calls = Arc::new(AtomicUsize::new(0));
    let resolver_calls = calls.clone();
    let fixture_resolver = fixture_source(root.path().join("source.mp4"));
    let service = make_service_with_source(
        root.path(),
        mock.clone(),
        Arc::new(move |source, context| {
            resolver_calls.fetch_add(1, Ordering::SeqCst);
            fixture_resolver(source, context)
        }),
    );
    let first = install_job(&service);
    let second = install_job(&service);
    service
        .update_job(&second.view.id, |job| {
            job.view.source.recording_id = "fixture-second".into()
        })
        .unwrap();
    service.start_worker().unwrap();
    wait_until(|| {
        service
            .snapshot()
            .unwrap()
            .jobs
            .iter()
            .any(|job| job.id == second.view.id && job.state == "ready")
    });
    wait_done(&service);
    let durable = service.vault.load().unwrap();
    assert_eq!(durable.jobs[0].view.state, "processing");
    assert!(durable.jobs[0].view.retry_at.is_some_and(|at| at > now()));
    assert!(durable.jobs[0].processing_started_at.is_some());
    assert_eq!(durable.jobs[0].view.video_id.as_deref(), Some(VIDEO));
    assert!(
        service.active_ids().is_empty(),
        "remote due-wait is not active local work"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    mock.push(facts(OWNER, "unlisted", "PT44S", "succeeded"));
    service
        .update_job(&first.view.id, |job| job.view.retry_at = Some(0))
        .unwrap();
    service.wake.notify_all();
    wait_until(|| service.snapshot().unwrap().jobs[0].state == "ready");
    wait_done(&service);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "processing check must not reopen either source"
    );
    assert_eq!(
        mock.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.method == reqwest::Method::POST)
            .count(),
        2
    );
    service.shutdown_and_wait().unwrap();
}

#[cfg(windows)]
#[test]
fn restarted_processing_receipt_checks_remote_without_source_even_after_deadline() {
    let root = fixture();
    let service = make_service(root.path(), Mock::new(vec![]));
    let started = now() - PROCESSING_WAIT_LIMIT_MS - 1;
    let job = install_processing_receipt(&service, VIDEO, started);
    service.shutdown_and_wait().unwrap();
    drop(service);
    let calls = Arc::new(AtomicUsize::new(0));
    let mock = Mock::new(vec![facts(OWNER, "unlisted", "PT44S", "succeeded")]);
    let resumed =
        make_service_with_source(root.path(), mock.clone(), rejecting_source(calls.clone()));
    assert_eq!(resumed.snapshot().unwrap().jobs[0].state, "paused");
    assert_eq!(resumed.snapshot().unwrap().jobs[0].visibility, CloudVisibility::Unlisted);
    assert_eq!(
        resumed.vault.load().unwrap().jobs[0].processing_started_at,
        Some(started)
    );
    resumed.resume(&job.view.id).unwrap();
    wait_until(|| resumed.snapshot().unwrap().jobs[0].state == "ready");
    wait_done(&resumed);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
    assert_eq!(
        mock.requests.lock().unwrap()[0].method,
        reqwest::Method::GET
    );
    assert_eq!(
        resumed.vault.load().unwrap().jobs[0].processing_started_at,
        Some(started)
    );
    resumed.shutdown_and_wait().unwrap();
}

#[cfg(windows)]
#[test]
fn terminated_processing_restart_checks_receipt_without_source_or_reupload_after_deadline() {
    let root = fixture();
    let service = make_service(root.path(), Mock::new(vec![]));
    let started = now() - PROCESSING_WAIT_LIMIT_MS - 1;
    let job = install_processing_receipt(&service, VIDEO, started);
    service.shutdown_and_wait().unwrap();
    drop(service);
    let calls = Arc::new(AtomicUsize::new(0));
    let mock = Mock::new(vec![status_facts(
        VIDEO,
        OWNER,
        "unlisted",
        "PT44S",
        Some("processed"),
        Some("terminated"),
    )]);
    let resumed =
        make_service_with_source(root.path(), mock.clone(), rejecting_source(calls.clone()));
    assert_eq!(resumed.snapshot().unwrap().jobs[0].state, "paused");
    resumed.resume(&job.view.id).unwrap();
    wait_until(|| resumed.snapshot().unwrap().jobs[0].state == "ready");
    wait_done(&resumed);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, reqwest::Method::GET);
    drop(requests);
    let durable = resumed.vault.load().unwrap();
    assert_eq!(durable.jobs.len(), 1);
    assert_eq!(durable.jobs[0].view.id, job.view.id);
    assert_eq!(durable.jobs[0].view.state, "ready");
    assert_eq!(durable.jobs[0].view.video_id.as_deref(), Some(VIDEO));
    assert_eq!(durable.jobs[0].processing_started_at, Some(started));
    assert_eq!(durable.jobs[0].source_sha256, job.source_sha256);
    assert!(durable.jobs[0].has_upload_receipt());
    resumed.shutdown_and_wait().unwrap();
}

#[cfg(windows)]
#[test]
fn pending_processing_deadline_requires_explicit_recheck_without_reupload() {
    let root = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let mock = Mock::new(vec![facts(OWNER, "unlisted", "PT44S", "processing")]);
    let service =
        make_service_with_source(root.path(), mock.clone(), rejecting_source(calls.clone()));
    let started = now() - PROCESSING_WAIT_LIMIT_MS - 1;
    let job = install_processing_receipt(&service, VIDEO, started);
    service.start_worker().unwrap();
    wait_until(|| service.snapshot().unwrap().jobs[0].state == "failed");
    wait_done(&service);
    let failed = service.snapshot().unwrap().jobs.remove(0);
    assert_eq!(failed.error.unwrap().code, "CLOUD_PROCESSING_WAIT");
    assert!(failed.retry_at.is_none());
    assert_eq!(
        service.vault.load().unwrap().jobs[0].processing_started_at,
        Some(started)
    );
    mock.push(facts(OWNER, "unlisted", "PT44S", "processing"));
    service.resume(&job.view.id).unwrap();
    wait_until(|| service.snapshot().unwrap().jobs[0].retry_at.is_some());
    wait_done(&service);
    let durable = service.vault.load().unwrap();
    assert_eq!(durable.jobs[0].view.state, "processing");
    assert!(durable.jobs[0].processing_started_at.unwrap() > started);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(mock
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|request| request.method == reqwest::Method::GET));
    service.shutdown_and_wait().unwrap();
}

#[cfg(windows)]
#[test]
fn processing_auth_loss_waits_for_account_without_reopening_or_ghost_work() {
    let root = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let mock = Mock::new(vec![response(400, json!({"error":"invalid_grant"}))]);
    let service =
        make_service_with_source(root.path(), mock.clone(), rejecting_source(calls.clone()));
    install_processing_receipt(&service, VIDEO, now());
    install_job(&service);
    {
        let mut inner = service.lock().unwrap();
        let mut candidate = inner.catalog.clone();
        candidate.grant.as_mut().unwrap().expires_at = 0;
        service.publish(&mut inner, candidate).unwrap();
    }
    service.start_worker().unwrap();
    wait_until(|| !service.snapshot().unwrap().account.connected);
    wait_done(&service);
    assert!(service
        .snapshot()
        .unwrap()
        .jobs
        .iter()
        .all(|job| job.state == "waiting_auth" && job.retry_at.is_none()));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
    assert!(service.active_ids().is_empty());
    service.shutdown_and_wait().unwrap();
}

#[cfg(windows)]
#[test]
fn in_flight_processing_check_stays_active_until_pause_response_drains() {
    let root = fixture();
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let mock = Mock::new(vec![facts(OWNER, "unlisted", "PT44S", "succeeded")]);
    let service = make_service_with_source(
        root.path(),
        Arc::new(GatedTransport {
            mock,
            gate: |request| request.method == reqwest::Method::GET,
            entered: entered.clone(),
            release: release.clone(),
        }),
        rejecting_source(Arc::new(AtomicUsize::new(0))),
    );
    let job = install_processing_receipt(&service, VIDEO, now());
    service.start_worker().unwrap();
    wait_until(|| entered.load(Ordering::SeqCst));
    assert!(service
        .active_ids()
        .contains(&format!("cloud:{}", job.view.id)));
    service.pause(&job.view.id).unwrap();
    assert_eq!(service.snapshot().unwrap().jobs[0].state, "paused");
    assert!(!service.active_ids().is_empty());
    release.store(true, Ordering::SeqCst);
    wait_done(&service);
    assert_eq!(
        service.snapshot().unwrap().jobs[0].state,
        "paused",
        "late remote success must not override pause"
    );
    service.shutdown_and_wait().unwrap();
}

#[cfg(windows)]
#[test]
fn processing_schedule_save_failure_does_not_publish_a_due_or_ready_state() {
    let root = fixture();
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let mock = Mock::new(vec![facts(OWNER, "unlisted", "PT44S", "processing")]);
    let service = make_service_with_source(
        root.path(),
        Arc::new(GatedTransport {
            mock,
            gate: |request| request.method == reqwest::Method::GET,
            entered: entered.clone(),
            release: release.clone(),
        }),
        rejecting_source(Arc::new(AtomicUsize::new(0))),
    );
    install_processing_receipt(&service, VIDEO, now());
    service.start_worker().unwrap();
    wait_until(|| entered.load(Ordering::SeqCst));
    service.vault.fail_once();
    release.store(true, Ordering::SeqCst);
    wait_until(|| service.lock().unwrap().persistence_error.is_some());
    wait_done(&service);
    let snapshot = service.snapshot().unwrap();
    assert_eq!(snapshot.jobs[0].state, "failed");
    assert_eq!(
        snapshot.jobs[0].error.as_ref().unwrap().code,
        "CLOUD_STORAGE"
    );
    assert!(snapshot.jobs[0].retry_at.is_none());
    let durable = service.vault.load().unwrap();
    assert_eq!(durable.jobs[0].view.state, "processing");
    assert!(durable.jobs[0].view.retry_at.is_none());
    assert_eq!(durable.jobs[0].view.video_id.as_deref(), Some(VIDEO));
    service.shutdown_and_wait().unwrap_err();
}

#[cfg(windows)]
#[test]
fn final_receipt_save_failure_stops_queue_without_remote_verification_or_fake_id() {
    let root = fixture();
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let mock = Mock::new(vec![opened(), offset(None), complete()]);
    let service = make_service(
        root.path(),
        Arc::new(GatedTransport {
            mock: mock.clone(),
            gate: |request| request.method == reqwest::Method::PUT && !request.body.is_empty(),
            entered: entered.clone(),
            release: release.clone(),
        }),
    );
    service.enqueue_with_visibility(reference(), CloudVisibility::Unlisted).unwrap();
    wait_until(|| entered.load(Ordering::SeqCst));
    service.vault.fail_once();
    release.store(true, Ordering::SeqCst);
    wait_until(|| service.lock().unwrap().persistence_error.is_some());
    wait_done(&service);
    assert_eq!(service.snapshot().unwrap().jobs[0].state, "failed");
    assert!(service.snapshot().unwrap().jobs[0].video_id.is_none());
    let durable = service.vault.load().unwrap();
    assert!(durable.jobs[0].view.video_id.is_none());
    assert_eq!(durable.jobs[0].session_url.as_deref(), Some(SESSION));
    assert!(mock
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|request| request.method != reqwest::Method::GET));
    service.shutdown_and_wait().unwrap_err();
}

#[test]
fn planned_source_identity_requires_a_complete_uuid_pair_without_capture_part() {
    let mut source = reference();
    source.plan_id = Some(uuid::Uuid::new_v4().to_string());
    assert!(!source.valid());
    source.unit_id = Some(uuid::Uuid::new_v4().to_string());
    assert!(source.valid());
    source.part_index = Some(0);
    assert!(!source.valid());
    source.part_index = None;
    source.unit_id = Some("../not-a-unit".into());
    assert!(!source.valid());
}

#[cfg(windows)]
#[test]
fn preparation_updates_are_fenced_to_the_current_active_attempt_and_stop_after_pause() {
    let root = fixture();
    let service = make_service(root.path(), Mock::new(vec![]));
    let job = install_job(&service);
    let current_cancel = Arc::new(AtomicBool::new(false));
    {
        let mut inner = service.lock().unwrap();
        inner.active = Some((job.view.id.clone(), current_cancel.clone()));
        let mut candidate = inner.catalog.clone();
        candidate.jobs[0].view.state = "validating".into();
        service.publish(&mut inner, candidate).unwrap();
    }
    let progress = SourcePreparationProgress {
        phase: "preparing".into(),
        completed_bytes: Some(1),
        total_bytes: Some(2),
        completed_seconds: None,
        total_seconds: None,
    };
    service.update_preparation(
        &job.view.id,
        &Arc::new(AtomicBool::new(false)),
        progress.clone(),
    );
    assert!(service.snapshot().unwrap().jobs[0].preparation.is_none());
    service.update_preparation(&job.view.id, &current_cancel, progress.clone());
    assert_eq!(
        service.snapshot().unwrap().jobs[0].preparation,
        Some(progress.clone())
    );
    service.pause(&job.view.id).unwrap();
    let changed = SourcePreparationProgress {
        phase: "verifying".into(),
        ..progress.clone()
    };
    service.update_preparation(&job.view.id, &current_cancel, changed);
    assert_eq!(
        service.snapshot().unwrap().jobs[0].preparation,
        Some(progress)
    );
    service.lock().unwrap().active = None;
    service.shutdown_and_wait().unwrap();
}

#[cfg(windows)]
#[test]
fn recording_plan_enqueue_is_all_or_nothing_on_invalid_units_and_persistence_failure() {
    let root = fixture();
    let mock = Mock::new(vec![]);
    let service = make_service(root.path(), mock.clone());
    let mut first = reference();
    first.plan_id = Some(uuid::Uuid::new_v4().to_string());
    first.unit_id = Some(uuid::Uuid::new_v4().to_string());
    let mut second = first.clone();
    second.unit_id = Some(uuid::Uuid::new_v4().to_string());
    let mut other_recording = second.clone();
    other_recording.recording_id = "different-recording".into();
    assert!(service
        .enqueue_many(vec![first.clone(), other_recording])
        .is_err());
    assert!(service
        .enqueue_many(vec![first.clone(), first.clone()])
        .is_err());
    assert!(service.snapshot().unwrap().jobs.is_empty());
    service.vault.fail_once();
    assert_eq!(
        service.enqueue_many(vec![first, second]).unwrap_err().code,
        "CLOUD_STORAGE"
    );
    assert!(service.snapshot().unwrap().jobs.is_empty());
    assert!(service.vault.load().unwrap().jobs.is_empty());
    assert!(mock.requests.lock().unwrap().is_empty());
    assert!(service.active_ids().is_empty());
    service.shutdown_and_wait().unwrap_err();
}

#[cfg(windows)]
#[test]
fn bounded_scheduler_alternates_due_processing_with_queued_uploads() {
    const SECOND: &str = "lmnopqrstuv";
    let root = fixture();
    let mock = Mock::new(vec![
        // The first check is pending. Its next due time will be forced while the
        // first upload is blocked, proving a second upload does not starve it.
        facts(OWNER, "unlisted", "PT44S", "processing"),
        opened(),
        offset(None),
        complete_for(SECOND),
        facts_for(SECOND, OWNER, "unlisted", "PT44S", "succeeded"),
        facts(OWNER, "unlisted", "PT44S", "succeeded"),
        opened(),
        offset(None),
        complete_for(SECOND),
        facts_for(SECOND, OWNER, "unlisted", "PT44S", "succeeded"),
    ]);
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let service = make_service(
        root.path(),
        Arc::new(GatedTransport {
            mock: mock.clone(),
            gate: |request| request.method == reqwest::Method::POST,
            entered: entered.clone(),
            release: release.clone(),
        }),
    );
    let processing = install_processing_receipt(&service, VIDEO, now());
    let first_upload = install_job(&service);
    let second_upload = install_job(&service);
    service
        .update_job(&second_upload.view.id, |job| {
            job.view.source.recording_id = "fixture-second".into()
        })
        .unwrap();
    service.start_worker().unwrap();
    wait_until(|| entered.load(Ordering::SeqCst));
    service
        .update_job(&processing.view.id, |job| job.view.retry_at = Some(0))
        .unwrap();
    release.store(true, Ordering::SeqCst);
    wait_until(|| {
        service
            .snapshot()
            .unwrap()
            .jobs
            .iter()
            .all(|job| job.state == "ready")
    });
    wait_done(&service);
    assert_eq!(service.snapshot().unwrap().jobs.len(), 3);
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 10);
    assert_eq!(
        requests[5].method,
        reqwest::Method::GET,
        "the pending video's second check precedes the second queued upload"
    );
    assert!(requests[5].url.contains(VIDEO));
    assert_eq!(requests[6].method, reqwest::Method::POST);
    drop(requests);
    assert_ne!(first_upload.view.id, second_upload.view.id);
    service.shutdown_and_wait().unwrap();
}

fn planned_units(source: &CloudSourceRef) -> Vec<CloudSourceRef> {
    let plan = uuid::Uuid::new_v4().to_string();
    (0..2)
        .map(|_| CloudSourceRef {
            provider: source.provider.clone(),
            recording_id: source.recording_id.clone(),
            part_index: None,
            plan_id: Some(plan.clone()),
            unit_id: Some(uuid::Uuid::new_v4().to_string()),
        })
        .collect()
}
fn make_planning_service(
    root: &Path,
    transport: Arc<dyn Transport>,
    planner: PlanResolver,
) -> Arc<CloudService> {
    let service = CloudService::with_transport_and_planner(
        root.join("app"),
        fixture_source(root.join("source.mp4")),
        Arc::new(|_| Err("must not open a real browser".into())),
        transport,
        Some(planner),
    )
    .unwrap();
    {
        let mut inner = service.lock().unwrap();
        let mut candidate = inner.catalog.clone();
        candidate.client = Some(client());
        candidate.grant = Some(grant());
        service.publish(&mut inner, candidate).unwrap();
    }
    service
}

#[cfg(windows)]
#[test]
fn recording_preparation_is_tracked_then_atomically_expands_without_replanning_units() {
    const SECOND: &str = "lmnopqrstuv";
    let root = fixture();
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let plan_entered = entered.clone();
    let plan_release = release.clone();
    let plan_calls = calls.clone();
    let planner: PlanResolver = Arc::new(move |source, context| {
        plan_calls.fetch_add(1, Ordering::SeqCst);
        (context.progress)(SourcePreparationProgress {
            phase: "preparing".into(),
            completed_bytes: None,
            total_bytes: None,
            completed_seconds: Some(1.0),
            total_seconds: Some(2.0),
        });
        plan_entered.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !plan_release.load(Ordering::SeqCst) {
            if context.cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Err("cancelled fixture preparation".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(planned_units(source))
    });
    let mock = Mock::new(vec![
        opened(),
        offset(None),
        complete(),
        facts(OWNER, "unlisted", "PT44S", "succeeded"),
        opened(),
        offset(None),
        complete_for(SECOND),
        facts_for(SECOND, OWNER, "unlisted", "PT44S", "succeeded"),
    ]);
    let service = make_planning_service(root.path(), mock.clone(), planner);
    service.enqueue_recording_with_visibility(reference(), CloudVisibility::Unlisted).unwrap();
    wait_until(|| entered.load(Ordering::SeqCst));
    let snapshot = service.snapshot().unwrap();
    assert_eq!(snapshot.jobs.len(), 1);
    let placeholder = &snapshot.jobs[0];
    assert!(placeholder.recording_plan);
    assert_eq!(placeholder.visibility, CloudVisibility::Unlisted);
    assert_eq!(placeholder.state, "validating");
    assert_eq!(placeholder.preparation.as_ref().unwrap().phase, "preparing");
    assert!(!service.active_ids().is_empty());
    assert_eq!(
        service.enqueue_recording_with_visibility(reference(), CloudVisibility::Unlisted).unwrap().jobs.len(),
        1
    );
    assert!(mock.requests.lock().unwrap().is_empty());
    release.store(true, Ordering::SeqCst);
    wait_until(|| {
        let snapshot = service.snapshot().unwrap();
        snapshot.jobs.len() == 2 && snapshot.jobs.iter().all(|job| job.state == "ready")
    });
    wait_done(&service);
    let jobs = service.snapshot().unwrap().jobs;
    assert!(jobs.iter().all(|job| !job.recording_plan
        && job.visibility == CloudVisibility::Unlisted
        && job.source.plan_id.is_some()
        && job.source.unit_id.is_some()
        && job.id != placeholder.id
        && job.created_at == placeholder.created_at));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "expanded unit jobs must not invoke PlanResolver again"
    );
    assert_eq!(
        service.enqueue_recording_with_visibility(reference(), CloudVisibility::Unlisted).unwrap().jobs.len(),
        2,
        "a completed plan is not recreated"
    );
    service.shutdown_and_wait().unwrap();
}

#[cfg(windows)]
#[test]
fn preparing_plan_shutdown_is_cancelled_and_joined_without_uploading_partial_units() {
    let root = fixture();
    let entered = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let plan_entered = entered.clone();
    let plan_stopped = stopped.clone();
    let planner: PlanResolver = Arc::new(move |_, context| {
        plan_entered.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !context.cancel.load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                return Err("fixture timed out".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        plan_stopped.store(true, Ordering::SeqCst);
        Err("fixture preparation cancelled".into())
    });
    let mock = Mock::new(vec![]);
    let service = make_planning_service(root.path(), mock.clone(), planner);
    service.enqueue_recording_with_visibility(reference(), CloudVisibility::Unlisted).unwrap();
    wait_until(|| entered.load(Ordering::SeqCst));
    assert!(!service.active_ids().is_empty());
    service.shutdown_and_wait().unwrap();
    assert!(stopped.load(Ordering::SeqCst));
    assert!(service.active_ids().is_empty());
    let jobs = service.vault.load().unwrap().jobs;
    assert_eq!(jobs.len(), 1);
    assert!(jobs[0].view.recording_plan);
    assert_eq!(jobs[0].view.state, "paused");
    assert!(mock.requests.lock().unwrap().is_empty());
}

#[cfg(windows)]
#[test]
fn failed_atomic_plan_expansion_preserves_only_recoverable_placeholder() {
    let root = fixture();
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let plan_entered = entered.clone();
    let plan_release = release.clone();
    let planner: PlanResolver = Arc::new(move |source, context| {
        plan_entered.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !plan_release.load(Ordering::SeqCst) {
            if context.cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Err("fixture cancelled".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(planned_units(source))
    });
    let mock = Mock::new(vec![]);
    let service = make_planning_service(root.path(), mock.clone(), planner);
    service.enqueue_recording_with_visibility(reference(), CloudVisibility::Unlisted).unwrap();
    wait_until(|| entered.load(Ordering::SeqCst));
    service.vault.fail_once();
    release.store(true, Ordering::SeqCst);
    wait_until(|| service.lock().unwrap().persistence_error.is_some());
    wait_done(&service);
    let durable = service.vault.load().unwrap();
    assert_eq!(durable.jobs.len(), 1);
    assert!(durable.jobs[0].view.recording_plan);
    assert!(durable.jobs[0].view.source.plan_id.is_none());
    assert_eq!(service.snapshot().unwrap().jobs[0].state, "failed");
    assert!(mock.requests.lock().unwrap().is_empty());
    service.shutdown_and_wait().unwrap_err();
}

#[test]
fn source_failure_adapter_retries_only_known_native_categories_and_never_exposes_raw_details() {
    for (prefix, retryable) in [
        ("CLOUD_SOURCE_SPACE:", true),
        ("CLOUD_SOURCE_IO:", true),
        ("CLOUD_SOURCE_BUSY:", true),
        ("hash mismatch:", false),
        ("CLOUD_SOURCE_PAUSED:", false),
    ] {
        let error = CloudError::source_failure(
            &format!("{prefix} C:\\private\\FIXTURE_SECRET raw helper output"),
            false,
            false,
        );
        assert_eq!(error.retryable, retryable);
        let encoded = serde_json::to_string(&error).unwrap();
        assert!(!encoded.contains("FIXTURE_SECRET"));
        assert!(!encoded.contains("private"));
        assert!(!encoded.contains("raw helper"));
    }
    assert!(
        CloudError::source_failure("CLOUD_SOURCE_COVERAGE: tail pending", true, false).retryable
    );
    assert!(
        !CloudError::source_failure(
            "CLOUD_SOURCE_COVERAGE: frozen source incomplete",
            false,
            false
        )
        .retryable
    );
    assert_eq!(
        CloudError::source_failure("anything", false, true).code,
        "CLOUD_PAUSED"
    );
    assert_eq!(
        CloudError::source_failure(
            "CLOUD_SOURCE_PAUSED: forged without cancellation",
            false,
            false
        )
        .code,
        "CLOUD_SOURCE_CHANGED"
    );
}

#[cfg(windows)]
#[test]
fn recoverable_preparation_space_and_io_failures_offer_manual_resume_without_retry_loop() {
    for prefix in ["CLOUD_SOURCE_SPACE:", "CLOUD_SOURCE_IO:"] {
        let root = fixture();
        let mock = Mock::new(vec![]);
        let calls = Arc::new(AtomicUsize::new(0));
        let source_calls = calls.clone();
        let resolver = fixture_source(root.path().join("source.mp4"));
        let service = make_service_with_source(
            root.path(),
            mock.clone(),
            Arc::new(move |source, context| {
                if source_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err(format!("{prefix} internal fixture details"));
                }
                resolver(source, context)
            }),
        );
        service.enqueue_with_visibility(reference(), CloudVisibility::Unlisted).unwrap();
        wait_until(|| service.snapshot().unwrap().jobs[0].state == "failed");
        wait_done(&service);
        let failed = service.snapshot().unwrap().jobs.remove(0);
        assert_eq!(
            failed.error.as_ref().unwrap().code,
            prefix.trim_end_matches(':')
        );
        assert!(
            failed.error.as_ref().unwrap().retryable,
            "the existing UI must show its resume action"
        );
        assert!(
            failed.retry_at.is_none(),
            "disk failures must not repeatedly prepare expensive volumes automatically"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(mock.requests.lock().unwrap().is_empty());
        mock.push(opened());
        mock.push(offset(None));
        mock.push(complete());
        mock.push(facts(OWNER, "unlisted", "PT44S", "succeeded"));
        service.resume(&failed.id).unwrap();
        wait_until(|| service.snapshot().unwrap().jobs[0].state == "ready");
        wait_done(&service);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(service.snapshot().unwrap().jobs.len(), 1);
        service.shutdown_and_wait().unwrap();
    }
}
