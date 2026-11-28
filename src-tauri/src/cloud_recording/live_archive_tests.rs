use super::*;
use serde_json::json;
use std::{collections::VecDeque, sync::Mutex};
const OWNER: &str = "UCabcdefghijklmnopqrstuv";
const VIDEO: &str = "abcdefghijk";
fn identity() -> Identity {
    Identity {
        selection_id: uuid::Uuid::new_v4().to_string(),
        request_id: uuid::Uuid::new_v4().to_string(),
        run_id: uuid::Uuid::new_v4().to_string(),
        source_fingerprint: "a".repeat(64),
        owner_channel_id: OWNER.into(),
        creation_owner_generation: 7,
        broadcast_id: VIDEO.into(),
        stream_id: "stream_fixture".into(),
    }
}
fn auth() -> Authorization {
    Authorization::new("fixture-token".into(), OWNER.into(), 9).unwrap()
}
fn broadcast() -> Value {
    json!({"id":VIDEO,"kind":"youtube#liveBroadcast","snippet":{"channelId":OWNER,"actualStartTime":"2026-11-18T00:00:00Z","actualEndTime":"2026-11-18T01:00:00Z"},"status":{"lifeCycleStatus":"complete","recordingStatus":"recorded","privacyStatus":"unlisted"},"contentDetails":{"boundStreamId":"stream_fixture"}})
}
fn video() -> Value {
    json!({"id":VIDEO,"kind":"youtube#video","snippet":{"channelId":OWNER},"status":{"privacyStatus":"unlisted","uploadStatus":"processed","embeddable":true},"contentDetails":{"duration":"PT59M58.25S"},"processingDetails":{"processingStatus":"succeeded"},"liveStreamingDetails":{"actualEndTime":"2026-11-18T01:00:00Z"}})
}
struct Mock {
    responses: Mutex<VecDeque<Response>>,
    requests: Mutex<Vec<Request>>,
}
impl Mock {
    fn new(values: Vec<Value>) -> Self {
        Self {
            responses: Mutex::new(
                values
                    .into_iter()
                    .map(|v| Response {
                        status: 200,
                        headers: vec![],
                        body: serde_json::to_vec(&json!({"items":[v]})).unwrap(),
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
impl Transport for Mock {
    fn send(&self, request: Request) -> Result<Response, CloudError> {
        assert!(allowed(&request));
        self.requests.lock().unwrap().push(request);
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected request"))
    }
}
#[test]
fn exact_metadata_is_verified_without_playback_or_sync_authority() {
    let i = identity();
    let m = Mock::new(vec![broadcast(), video()]);
    let a = auth();
    let mut r = Record::begin(i.clone(), a.generation, 1_000, None).unwrap();
    r.start_round(1_000).unwrap();
    let round = verify_round(&m, &a, &i, || r.request(1_000));
    r.apply(round, 2_000).unwrap();
    assert_eq!(m.count(), 2);
    assert!(r.valid());
    assert_eq!(r.state, ArchiveState::MetadataVerified);
    let s = r.snapshot();
    assert_eq!(s.video_id.as_deref(), Some(VIDEO));
    assert_eq!(s.duration_seconds, Some(3598.25));
    assert!(!s.playback_verified && !s.synchronized_replay_ready);
    assert_eq!(s.remote_time_origin, "unknown");
    assert_eq!(r.identity.creation_owner_generation, 7);
    assert_eq!(r.authorization_generation, 9);
    let encoded = serde_json::to_string(&r).unwrap();
    assert!(!encoded.contains("fixture-token"));
    assert!(encoded.len() < RECORD_BYTES);
}
#[test]
fn wrong_broadcast_owner_id_stream_or_privacy_stops_before_video_request() {
    for (field, value, reason) in [
        (
            "owner",
            "UCxxxxxxxxxxxxxxxxxxxxxx",
            ArchiveReason::OwnerMismatch,
        ),
        ("id", "zzzzzzzzzzz", ArchiveReason::IdentityMismatch),
        ("stream", "other", ArchiveReason::IdentityMismatch),
        ("privacy", "public", ArchiveReason::PrivacyMismatch),
    ] {
        let mut b = broadcast();
        match field {
            "owner" => b["snippet"]["channelId"] = value.into(),
            "id" => b["id"] = value.into(),
            "stream" => b["contentDetails"]["boundStreamId"] = value.into(),
            _ => b["status"]["privacyStatus"] = value.into(),
        };
        let m = Mock::new(vec![b]);
        let r = verify_round(&m, &auth(), &identity(), || Ok(()));
        assert_eq!(r.state, ArchiveState::Rejected);
        assert_eq!(r.reason, Some(reason));
        assert_eq!(m.count(), 1);
    }
}
#[test]
fn video_identity_processing_embedding_and_missing_fields_never_claim_playback() {
    for (change, state, reason) in [
        (0, ArchiveState::Rejected, ArchiveReason::OwnerMismatch),
        (1, ArchiveState::Rejected, ArchiveReason::PrivacyMismatch),
        (2, ArchiveState::Rejected, ArchiveReason::ProcessingFailed),
        (3, ArchiveState::Rejected, ArchiveReason::NotEmbeddable),
        (4, ArchiveState::Pending, ArchiveReason::Processing),
        (5, ArchiveState::Pending, ArchiveReason::MetadataMissing),
        (
            6,
            ArchiveState::Inconclusive,
            ArchiveReason::InvalidResponse,
        ),
    ] {
        let mut v = video();
        match change {
            0 => v["snippet"]["channelId"] = "UCxxxxxxxxxxxxxxxxxxxxxx".into(),
            1 => v["status"]["privacyStatus"] = "private".into(),
            2 => v["status"]["uploadStatus"] = "deleted".into(),
            3 => v["status"]["embeddable"] = false.into(),
            4 => v["processingDetails"]["processingStatus"] = "processing".into(),
            5 => v["contentDetails"]
                .as_object_mut()
                .unwrap()
                .remove("duration")
                .map(|_| ())
                .unwrap(),
            _ => v["contentDetails"]["duration"] = "PT0S".into(),
        };
        let m = Mock::new(vec![broadcast(), v]);
        let r = verify_round(&m, &auth(), &identity(), || Ok(()));
        assert_eq!((r.state, r.reason), (state, Some(reason)));
        assert!(r.facts.valid());
    }
    let mut v = video();
    v["processingDetails"]["processingStatus"] = "terminated".into();
    let m = Mock::new(vec![broadcast(), v]);
    let r = verify_round(&m, &auth(), &identity(), || Ok(()));
    assert_eq!(r.state, ArchiveState::MetadataVerified);
    assert_eq!(r.facts.processing.as_deref(), Some("terminated"));
}
#[test]
fn nonterminal_empty_and_missing_end_remain_pending() {
    for mode in 0..3 {
        let mut b = broadcast();
        if mode == 0 {
            b["status"]["lifeCycleStatus"] = "live".into();
        } else if mode == 1 {
            b["snippet"]
                .as_object_mut()
                .unwrap()
                .remove("actualEndTime");
        }
        let m = Mock::new(vec![b]);
        if mode == 2 {
            m.responses.lock().unwrap()[0].body = b"{\"items\":[]}".to_vec();
        }
        let r = verify_round(&m, &auth(), &identity(), || Ok(()));
        assert_eq!(r.state, ArchiveState::Pending);
        assert_eq!(m.count(), 1);
    }
}
#[test]
fn admission_failure_between_reads_never_sends_second_request() {
    let m = Mock::new(vec![broadcast()]);
    let mut n = 0;
    let r = verify_round(&m, &auth(), &identity(), || {
        n += 1;
        if n == 2 {
            Err(ArchiveReason::SaveFailed)
        } else {
            Ok(())
        }
    });
    assert_eq!(m.count(), 1);
    assert_eq!(r.reason, Some(ArchiveReason::SaveFailed));
    assert_eq!(r.state, ArchiveState::Inconclusive);
}
#[test]
fn progressive_virtual_clock_has_fixed_deadline_and_finite_request_budget() {
    let mut r = Record::begin(identity(), 9, 1000, None).unwrap();
    let deadline = r.deadline_at;
    let mut at = 1000;
    loop {
        if r.budget(at).is_err() {
            break;
        }
        r.start_round(at).unwrap();
        r.request(at).unwrap();
        r.request(at).unwrap();
        r.apply(
            result(
                ArchiveState::Pending,
                ArchiveReason::Processing,
                Observation::default(),
            ),
            at,
        )
        .unwrap();
        if r.state != ArchiveState::Pending {
            break;
        }
        let next = r.retry_at.unwrap();
        assert_eq!(next, at.saturating_add(delay_ms(r.rounds)).min(deadline));
        at = next;
    }
    assert_eq!(r.deadline_at, deadline);
    assert!(r.rounds <= 128 && r.requests <= 256);
    assert_eq!(at, deadline);
    assert!(r.start_round(at).is_err());
    assert_eq!(
        (delay_ms(4), delay_ms(5), delay_ms(14), delay_ms(15)),
        (5000, 60000, 60000, 900000)
    );
    let mut cap = Record::begin(identity(), 9, 1000, None).unwrap();
    cap.rounds = 127;
    cap.requests = 254;
    cap.start_round(1000).unwrap();
    cap.request(1000).unwrap();
    cap.request(1000).unwrap();
    assert!(cap.request(1000).is_err());
    cap.apply(
        result(
            ArchiveState::Pending,
            ArchiveReason::Processing,
            Observation::default(),
        ),
        1000,
    )
    .unwrap();
    assert_eq!(cap.reason, Some(ArchiveReason::RequestLimit));
}
#[test]
fn restart_resume_does_not_renew_attempt_or_discard_first_verified_proof() {
    let i = identity();
    let mut r = Record::begin(i.clone(), 9, 1000, None).unwrap();
    r.start_round(1000).unwrap();
    r.request(1000).unwrap();
    let before = (r.attempt_id.clone(), r.deadline_at, r.requests, r.rounds);
    r.recover();
    assert_eq!(r.state, ArchiveState::NeedsCheck);
    r.resume(10, 2000).unwrap();
    assert_eq!(
        before,
        (r.attempt_id.clone(), r.deadline_at, r.requests, r.rounds)
    );
    assert_eq!(r.identity.creation_owner_generation, 7);
    r.start_round(2000).unwrap();
    let m = Mock::new(vec![broadcast(), video()]);
    let result = verify_round(&m, &auth(), &i, || r.request(2000));
    r.apply(result, 3000).unwrap();
    let proof = serde_json::to_vec(r.verified.as_ref().unwrap()).unwrap();
    let next = Record::begin(i, 11, r.deadline_at + 1, Some(&r)).unwrap();
    assert_eq!(
        proof,
        serde_json::to_vec(next.verified.as_ref().unwrap()).unwrap()
    );
    assert_ne!(r.attempt_id, next.attempt_id);
    assert!(next.valid());
    let mut invalid = next.clone();
    invalid
        .verified
        .as_mut()
        .unwrap()
        .observation
        .duration_seconds = Some(1.0);
    assert!(!invalid.valid());
}
#[test]
fn exact_get_transport_policy_rejects_enumeration_mutation_and_redirect_targets() {
    let m = Mock::new(vec![broadcast(), video()]);
    verify_round(&m, &auth(), &identity(), || Ok(()));
    let mut requests = m.requests.lock().unwrap();
    let mut r = requests.remove(0);
    assert!(allowed(&r));
    r.method = Method::POST;
    assert!(!allowed(&r));
    r.method = Method::GET;
    r.url.push_str("&mine=true");
    assert!(!allowed(&r));
    r.url = r.url.replace("&mine=true", "");
    r.url = r.url.replace("www.googleapis.com", "example.org");
    assert!(!allowed(&r));
}
#[test]
fn quota_and_oversize_responses_stop_without_polling_or_raw_error_text() {
    for mode in 0..2 {
        let m = Mock::new(vec![broadcast()]);
        {
            let mut q = m.responses.lock().unwrap();
            if mode == 0 {
                q[0].status = 403;
                q[0].body=serde_json::to_vec(&json!({"error":{"message":"not-for-ui","errors":[{"reason":"quotaExceeded"}]}})).unwrap();
            } else {
                q[0].body = vec![b' '; MAX_RESPONSE + 1];
            }
        }
        let r = verify_round(&m, &auth(), &identity(), || Ok(()));
        assert_eq!(m.count(), 1);
        assert_eq!(
            r.reason,
            Some(if mode == 0 {
                ArchiveReason::Quota
            } else {
                ArchiveReason::InvalidResponse
            })
        );
    }
}
