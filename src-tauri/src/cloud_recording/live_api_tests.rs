use super::*;
use std::{collections::VecDeque, sync::Mutex};

const OWNER: &str = "UCabcdefghijklmnopqrstuv";
const VIDEO: &str = "abcdefghijk";
const STREAM: &str = "stream_123";
const TARGET: &str =
    "https://a.upload.youtube.com/http_upload_hls?cid=only-native-key&copy=0&file=";

struct Script {
    responses: Mutex<VecDeque<Result<Response, CloudError>>>,
    requests: Mutex<Vec<Request>>,
}
impl Script {
    fn new(values: Vec<Value>) -> Self {
        Self {
            responses: Mutex::new(
                values
                    .into_iter()
                    .map(|v| {
                        Ok(Response {
                            status: 200,
                            headers: vec![],
                            body: serde_json::to_vec(&v).unwrap(),
                        })
                    })
                    .collect(),
            ),
            requests: Mutex::new(vec![]),
        }
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}
impl Transport for Script {
    fn send(&self, request: Request) -> Result<Response, CloudError> {
        assert!(allowed(&request));
        self.requests.lock().unwrap().push(request);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected retry/request")
    }
}
fn auth() -> LiveAuthorization {
    LiveAuthorization::from_granted_scopes(
        "native-token".into(),
        NativeDestinationOwner {
            channel_id: OWNER.into(),
            generation: 7,
        },
        LIVE_SCOPE,
    )
    .unwrap()
}
fn plan() -> PreparationPlan {
    PreparationPlan {
        request_id: Uuid::new_v4(),
        title: "Selected SOOP broadcast".into(),
        scheduled_start_ms: 1_800_000_000_000,
        made_for_kids: false,
    }
}
fn stream() -> Value {
    json!({"kind":"youtube#liveStream","id":STREAM,"snippet":{"channelId":OWNER},
        "cdn":{"ingestionType":"hls","ingestionInfo":{"ingestionAddress":TARGET}},"contentDetails":{"isReusable":false}})
}
fn broadcast(bound: bool) -> Value {
    let mut value = json!({"kind":"youtube#liveBroadcast","id":VIDEO,"snippet":{"channelId":OWNER},
        "status":{"privacyStatus":"unlisted","selfDeclaredMadeForKids":false,"lifeCycleStatus":"ready"},
        "contentDetails":{"enableAutoStart":true,"enableAutoStop":true,"enableDvr":true,"enableEmbed":true,"recordFromStart":true,
            "monitorStream":{"enableMonitorStream":false}}});
    if bound {
        value["contentDetails"]["boundStreamId"] = STREAM.into();
    }
    value
}
fn failure(result: Result<PreparationSuccess, PreparationFailure>) -> PreparationFailure {
    match result {
        Ok(_) => panic!("unexpected prepared destination"),
        Err(value) => value,
    }
}

#[test]
fn upload_and_readonly_cannot_be_used_as_live_authority() {
    for scopes in ["", super::super::oauth::UPLOAD_SCOPE, super::super::oauth::READ_SCOPE,
        "https://www.googleapis.com/auth/youtube.upload https://www.googleapis.com/auth/youtube.readonly"] {
        assert!(LiveAuthorization::from_granted_scopes("native-token".into(), NativeDestinationOwner { channel_id: OWNER.into(), generation: 7 }, scopes).is_err());
    }
    assert!(LiveAuthorization::from_granted_scopes(
        "native-token".into(),
        NativeDestinationOwner {
            channel_id: OWNER.into(),
            generation: 7
        },
        FULL_SCOPE
    )
    .is_ok());
}

#[test]
fn ordered_unlisted_hls_creation_and_binding_has_durable_receipts() {
    let script = Script::new(vec![stream(), broadcast(false), broadcast(true)]);
    let plan = plan();
    let mut receipts = vec![];
    let success = prepare(&script, &auth(), &plan, |p| {
        receipts.push((script.count(), p.clone()));
        Ok(())
    })
    .unwrap_or_else(|e| panic!("{}", e.error.code));
    assert_eq!(success.progress.stage, PreparationStage::Prepared);
    assert!(!success.progress.outcome_uncertain);
    assert_eq!(script.count(), 3);
    for (stage, count) in [
        (PreparationStage::CreatingStream, 0),
        (PreparationStage::CreatingBroadcast, 1),
        (PreparationStage::Binding, 2),
    ] {
        assert!(receipts
            .iter()
            .any(|(n, p)| *n == count && p.stage == stage && p.outcome_uncertain));
    }
    let requests = script.requests.lock().unwrap();
    let stream_body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        stream_body["snippet"]["description"],
        format!("Atsumi live archive reference: {}", plan.request_id)
    );
    assert_eq!(
        stream_body["cdn"],
        json!({"ingestionType":"hls","frameRate":"variable","resolution":"variable"})
    );
    assert_eq!(stream_body["contentDetails"]["isReusable"], false);
    let broadcast_body: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(
        broadcast_body["status"],
        json!({"privacyStatus":"unlisted","selfDeclaredMadeForKids":false})
    );
    assert!(requests[2].body.is_empty());
    let binding = Url::parse(&requests[2].url).unwrap();
    assert!(binding
        .query_pairs()
        .any(|(k, v)| k == "streamId" && v == STREAM));
    assert!(binding.query_pairs().any(|(k, v)| k == "id" && v == VIDEO));
    let serialized = serde_json::to_string(&success.progress).unwrap();
    assert!(!serialized.contains("only-native-key"));
    assert!(!serialized.contains("native-token"));
    assert!(!serialized.contains("http"));
}

#[test]
fn audience_is_explicit_and_not_forced_to_old_test_choice() {
    let mut selected = plan();
    selected.made_for_kids = true;
    let mut created = broadcast(false);
    created["status"]["selfDeclaredMadeForKids"] = true.into();
    let mut bound = broadcast(true);
    bound["status"]["selfDeclaredMadeForKids"] = true.into();
    let script = Script::new(vec![stream(), created, bound]);
    assert!(prepare(&script, &auth(), &selected, |_| Ok(())).is_ok());
    let request: Value = serde_json::from_slice(&script.requests.lock().unwrap()[1].body).unwrap();
    assert_eq!(request["status"]["selfDeclaredMadeForKids"], true);
}

#[test]
fn failed_initial_receipt_causes_zero_remote_requests() {
    let script = Script::new(vec![]);
    let result = failure(prepare(&script, &auth(), &plan(), |_| {
        Err(CloudError::storage())
    }));
    assert_eq!(script.count(), 0);
    assert_eq!(result.progress.stage, PreparationStage::Selected);
}

#[test]
fn failed_receipt_after_create_retains_id_and_stops_before_next_mutation() {
    let script = Script::new(vec![stream()]);
    let result = failure(prepare(&script, &auth(), &plan(), |p| {
        if p.stream.is_some() {
            Err(CloudError::storage())
        } else {
            Ok(())
        }
    }));
    assert_eq!(script.count(), 1);
    assert_eq!(result.error.code, "CLOUD_STORAGE");
    assert_eq!(result.progress.stream.unwrap().id, STREAM);
}

#[test]
fn final_receipt_failure_never_returns_destination() {
    let script = Script::new(vec![stream(), broadcast(false), broadcast(true)]);
    let result = failure(prepare(&script, &auth(), &plan(), |p| {
        if p.stage == PreparationStage::Prepared {
            Err(CloudError::storage())
        } else {
            Ok(())
        }
    }));
    assert_eq!(script.count(), 3);
    assert_eq!(result.error.code, "CLOUD_STORAGE");
    assert_eq!(result.progress.broadcast.unwrap().id, VIDEO);
}

#[test]
fn timeout_keeps_partial_receipt_and_never_retries_creation() {
    let script = Script::new(vec![stream()]);
    script
        .responses
        .lock()
        .unwrap()
        .push_back(Err(error("CLOUD_LIVE_NETWORK", "bounded")));
    let result = failure(prepare(&script, &auth(), &plan(), |_| Ok(())));
    assert_eq!(script.count(), 2);
    assert_eq!(result.progress.stage, PreparationStage::CreatingBroadcast);
    assert!(result.progress.outcome_uncertain);
    assert_eq!(result.progress.stream.unwrap().id, STREAM);
    assert!(result.progress.broadcast.is_none());
    assert!(!result.error.retryable);
}

#[test]
fn wrong_owner_id_is_recovery_information_without_authority() {
    let mut wrong = stream();
    wrong["snippet"]["channelId"] = "UCotherotherotherotheroth".into();
    let script = Script::new(vec![wrong]);
    let result = failure(prepare(&script, &auth(), &plan(), |_| Ok(())));
    assert_eq!(script.count(), 1);
    assert!(result.progress.outcome_uncertain);
    let receipt = result.progress.stream.unwrap();
    assert_eq!(receipt.id, STREAM);
    assert!(!receipt.owner_verified);
}

#[test]
fn bad_ingestion_url_keeps_created_resource_without_exposing_key() {
    let mut wrong = stream();
    wrong["cdn"]["ingestionInfo"]["ingestionAddress"] = "https://evil.test/?key=secret".into();
    let script = Script::new(vec![wrong]);
    let result = failure(prepare(&script, &auth(), &plan(), |_| Ok(())));
    assert_eq!(script.count(), 1);
    assert!(result.progress.stream.is_some());
    assert!(!format!("{result:?}").contains("evil"));
    assert!(!format!("{result:?}").contains("secret"));
}

#[test]
fn public_or_mismatched_bound_response_never_produces_capability() {
    for (path, value) in [
        ("/status/privacyStatus", "public"),
        ("/contentDetails/boundStreamId", "other_stream"),
        ("/id", "anotherid12"),
    ] {
        let mut wrong = broadcast(true);
        *wrong.pointer_mut(path).unwrap() = value.into();
        let script = Script::new(vec![stream(), broadcast(false), wrong]);
        let result = failure(prepare(&script, &auth(), &plan(), |_| Ok(())));
        assert_eq!(script.count(), 3);
        assert_eq!(result.progress.broadcast.unwrap().id, VIDEO);
        assert!(result.progress.outcome_uncertain);
    }
}

#[test]
fn malformed_success_remains_uncertain_and_stops() {
    let script = Script::new(vec![json!({"kind":"youtube#liveStream"})]);
    let result = failure(prepare(&script, &auth(), &plan(), |_| Ok(())));
    assert_eq!(script.count(), 1);
    assert!(result.progress.outcome_uncertain);
}

#[test]
fn activation_error_is_bounded_and_does_not_trigger_retry() {
    let script = Script::new(vec![]);
    script.responses.lock().unwrap().push_back(Ok(Response { status: 403, headers: vec![],
        body: serde_json::to_vec(&json!({"error":{"message":"untrusted secret payload","errors":[{"reason":"liveStreamingNotEnabled"}]}})).unwrap() }));
    let result = failure(prepare(&script, &auth(), &plan(), |_| Ok(())));
    assert_eq!(script.count(), 1);
    assert!(!result.progress.outcome_uncertain);
    assert_eq!(result.error.code, "CLOUD_LIVE_NOT_ENABLED");
    assert!(!format!("{result:?}").contains("payload"));
}

#[test]
fn live_transport_does_not_accept_page_targets_or_delete_methods() {
    let mut r = request(
        &auth(),
        "liveStreams",
        &[("part", "snippet,cdn")],
        json!({}),
    )
    .unwrap();
    assert!(allowed(&r));
    for value in [
        "https://www.googleapis.com.evil.test/youtube/v3/liveStreams",
        "http://www.googleapis.com/youtube/v3/liveStreams",
        "https://www.googleapis.com/youtube/v3/videos",
        "https://user@www.googleapis.com/youtube/v3/liveStreams",
    ] {
        r.url = value.into();
        assert!(!allowed(&r));
    }
    r.url = "https://www.googleapis.com/youtube/v3/liveStreams".into();
    r.method = Method::DELETE;
    assert!(!allowed(&r));
}

#[test]
fn invalid_plan_does_not_create_anything() {
    for title in ["", "<script>", "bad\nname"] {
        let mut p = plan();
        p.title = title.into();
        let script = Script::new(vec![]);
        assert!(prepare(&script, &auth(), &p, |_| Ok(())).is_err());
        assert_eq!(script.count(), 0);
    }
}
