use super::super::super::{
    live_store::SourceFacts,
    upload::{Request, Response},
};
use super::*;
use serde_json::json;
use std::sync::atomic::AtomicUsize;

const OWNER: &str = "UCabcdefghijklmnopqrstuv";
struct NoNetwork;
impl Transport for NoNetwork {
    fn send(&self, _: Request) -> Result<Response, CloudError> {
        panic!("external network forbidden")
    }
}
fn service(root: &std::path::Path) -> Arc<CloudService> {
    CloudService::with_transport(
        root.into(),
        Arc::new(|_, _| Err("no source".into())),
        Arc::new(|_| Err("no browser".into())),
        Arc::new(NoNetwork),
    )
    .unwrap()
}
fn install(service: &CloudService) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let mut receipt = LiveReceipt::selected(
        id.clone(),
        SourceFacts {
            provider: "soop".into(),
            channel_id: "fixture".into(),
            broadcast_key: Some("123".into()),
            session_id: uuid::Uuid::new_v4().to_string(),
            epoch: 1,
            account_revision: 0,
        },
    );
    receipt.view.state = "joined".into();
    receipt.view.preparation_stage = Some("prepared".into());
    receipt.view.destination_channel_id = Some(OWNER.into());
    receipt.owner_generation = Some(7);
    receipt.view.stream_id = Some("stream_fixture".into());
    receipt.view.broadcast_id = Some("abcdefghijk".into());
    receipt.view.run_id = Some(uuid::Uuid::new_v4().to_string());
    receipt.stream_owner_verified = true;
    receipt.broadcast_owner_verified = true;
    receipt.view.manager_joined = true;
    receipt.view.worker_joined = true;
    receipt.view.sidecar_joined = true;
    receipt.view.end_list_acked = true;
    receipt.view.stop_requested = true;
    assert!(receipt.valid());
    let mut inner = service.lock().unwrap();
    let mut candidate = inner.catalog.clone();
    candidate.client = Some(oauth::ClientConfig {
        client_id: "123-fixture.apps.googleusercontent.com".into(),
        client_secret: "fixture-secret".into(),
    });
    let grant = oauth::Grant {
        access_token: "fixture-token".into(),
        refresh_token: "fixture-refresh".into(),
        expires_at: now() + 3_600_000,
        channel_id: OWNER.into(),
        channel_name: "Fixture".into(),
    };
    candidate.grant = Some(grant.clone());
    candidate.live_generation = 7;
    candidate.live_grant = Some(oauth::LiveGrant {
        grant,
        scopes: vec!["https://www.googleapis.com/auth/youtube.force-ssl".into()],
    });
    candidate.live_receipts.push(receipt);
    service.publish(&mut inner, candidate).unwrap();
    id
}
fn ticket(service: &CloudService, id: &str, explicit: bool, at: u64) -> Ticket {
    let mut inner = service.lock().unwrap();
    let owner = current_owner(&inner).unwrap();
    service
        .reserve_archive(&mut inner, id, owner, explicit, at)
        .unwrap()
        .unwrap()
}
fn authorization(ticket: &Ticket) -> Authorization {
    Authorization::new(
        "fixture-token".into(),
        ticket.owner.channel_id.clone(),
        ticket.owner.generation,
    )
    .unwrap()
}
struct Replies {
    service: Arc<CloudService>,
    calls: AtomicUsize,
    mode: u8,
}
impl Transport for Replies {
    fn send(&self, request: Request) -> Result<Response, CloudError> {
        let call = self.calls.fetch_add(1, Ordering::AcqRel);
        let stored = self.service.vault.load().unwrap();
        let record = stored.live_receipts[0].archive.as_ref().unwrap();
        assert_eq!(
            record.requests as usize,
            call + 1,
            "each GET must first be durable"
        );
        assert_eq!(request.method, reqwest::Method::GET);
        assert!(request.body.is_empty());
        assert!(request.url.contains(if call == 0 {
            "/liveBroadcasts?"
        } else {
            "/videos?"
        }));
        if self.mode == 1 {
            self.service.vault.fail_once();
        }
        if self.mode == 2 {
            let mut inner = self.service.lock().unwrap();
            let mut candidate = inner.catalog.clone();
            candidate.live_generation += 1;
            self.service.publish(&mut inner, candidate).unwrap();
        }
        if self.mode == 3 {
            panic!("controlled worker panic");
        }
        let value = if call == 0 {
            json!({"id":"abcdefghijk","kind":"youtube#liveBroadcast","snippet":{"channelId":OWNER,"actualEndTime":"2026-11-18T01:00:00Z"},"status":{"lifeCycleStatus":"complete","recordingStatus":"recorded","privacyStatus":"unlisted"},"contentDetails":{"boundStreamId":"stream_fixture"}})
        } else {
            json!({"id":"abcdefghijk","kind":"youtube#video","snippet":{"channelId":OWNER},"status":{"privacyStatus":"unlisted","uploadStatus":"processed","embeddable":true},"contentDetails":{"duration":"PT1H"},"processingDetails":{"processingStatus":"succeeded"}})
        };
        Ok(Response {
            status: 200,
            headers: vec![],
            body: serde_json::to_vec(&json!({"items":[value]})).unwrap(),
        })
    }
}
fn run(
    service: &CloudService,
    ticket: &Ticket,
    replies: &Replies,
    at: u64,
) -> Result<(), ArchiveReason> {
    let result = service.run_archive_check(
        ticket,
        replies,
        || Ok(authorization(ticket)),
        || at,
        |_| panic!("unexpected wait"),
    );
    if let Err(reason) = result {
        service.stop_archive_ticket(ticket, reason);
    }
    service.release_archive_ticket(ticket);
    result
}

#[test]
fn joined_start_owns_one_automatic_window_and_preserves_upload_and_local_facts() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path());
    let id = install(&service);
    let before = service.vault.load().unwrap();
    let t = ticket(&service, &id, false, 1000);
    assert_eq!(
        service.lock().unwrap().live.active_ids(),
        vec![format!("live-archive:{id}")]
    );
    let replies = Replies {
        service: service.clone(),
        calls: AtomicUsize::new(0),
        mode: 0,
    };
    run(&service, &t, &replies, 1000).unwrap();
    assert_eq!(replies.calls.load(Ordering::Acquire), 2);
    let after = service.vault.load().unwrap();
    let receipt = &after.live_receipts[0];
    assert_eq!(
        receipt.archive.as_ref().unwrap().state,
        ArchiveState::MetadataVerified
    );
    assert_eq!(receipt.view.state, "joined");
    assert!(
        !receipt.view.interrupted && receipt.view.error.is_none() && !receipt.view.archive_ready
    );
    assert_eq!(
        serde_json::to_vec(&before.grant).unwrap(),
        serde_json::to_vec(&after.grant).unwrap()
    );
    assert!(
        !receipt
            .view
            .archive_verification
            .as_ref()
            .unwrap()
            .playback_verified
    );
    let mut inner = service.lock().unwrap();
    let owner = current_owner(&inner).unwrap();
    assert!(service
        .reserve_archive(&mut inner, &id, owner, false, 2000)
        .unwrap()
        .is_none());
}
#[test]
fn save_failure_between_gets_stops_and_does_not_reclassify_local_relay() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path());
    let id = install(&service);
    let t = ticket(&service, &id, false, 1000);
    let replies = Replies {
        service: service.clone(),
        calls: AtomicUsize::new(0),
        mode: 1,
    };
    assert_eq!(
        run(&service, &t, &replies, 1000),
        Err(ArchiveReason::SaveFailed)
    );
    assert_eq!(replies.calls.load(Ordering::Acquire), 1);
    let view = service.live_snapshot().unwrap().operations.remove(0);
    assert!(!view.receipt_persisted);
    assert_eq!(view.state, "joined");
    assert!(!view.interrupted && view.error.is_none());
    assert_eq!(
        view.archive_verification.unwrap().reason,
        Some(ArchiveReason::SaveFailed)
    );
    assert!(service.vault.load().unwrap().live_receipts[0]
        .archive
        .as_ref()
        .unwrap()
        .verified
        .is_none());
}
#[test]
fn changed_check_authority_after_response_prevents_second_get_and_promotion() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path());
    let id = install(&service);
    let t = ticket(&service, &id, false, 1000);
    let replies = Replies {
        service: service.clone(),
        calls: AtomicUsize::new(0),
        mode: 2,
    };
    assert_eq!(
        run(&service, &t, &replies, 1000),
        Err(ArchiveReason::OwnerChanged)
    );
    assert_eq!(replies.calls.load(Ordering::Acquire), 1);
    let stored = service.vault.load().unwrap();
    let archive = stored.live_receipts[0].archive.as_ref().unwrap();
    assert_eq!(archive.identity.creation_owner_generation, 7);
    assert!(archive.verified.is_none());
    assert_eq!(archive.reason, Some(ArchiveReason::OwnerChanged));
}
#[test]
fn shutdown_and_save_intent_failure_issue_no_get() {
    for fail_save in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let service = service(temp.path());
        let id = install(&service);
        if fail_save {
            service.vault.fail_once();
            let mut inner = service.lock().unwrap();
            let owner = current_owner(&inner).unwrap();
            assert!(matches!(
                service.reserve_archive(&mut inner, &id, owner, false, 1000),
                Err(ArchiveReason::SaveFailed)
            ));
            assert!(inner.live.archive_checks.is_empty());
        } else {
            let t = ticket(&service, &id, false, 1000);
            service.request_live_shutdown().unwrap();
            let replies = Replies {
                service: service.clone(),
                calls: AtomicUsize::new(0),
                mode: 0,
            };
            assert_eq!(
                run(&service, &t, &replies, 1000),
                Err(ArchiveReason::Cancelled)
            );
            assert_eq!(replies.calls.load(Ordering::Acquire), 0);
            assert_eq!(
                service.vault.load().unwrap().live_receipts[0].view.state,
                "joined"
            );
        }
    }
}
#[test]
fn restart_requires_explicit_resume_same_window_and_allows_same_channel_reauth() {
    let temp = tempfile::tempdir().unwrap();
    let first = service(temp.path());
    let id = install(&first);
    let t = ticket(&first, &id, false, 1000);
    let before = first.vault.load().unwrap().live_receipts[0]
        .archive
        .clone()
        .unwrap();
    first.release_archive_ticket(&t);
    drop(first);
    let reopened = service(temp.path());
    let stored = reopened.vault.load().unwrap();
    let old = stored.live_receipts[0].archive.as_ref().unwrap();
    assert_eq!(old.state, ArchiveState::NeedsCheck);
    assert!(reopened.lock().unwrap().live.archive_checks.is_empty());
    {
        let mut inner = reopened.lock().unwrap();
        let mut candidate = inner.catalog.clone();
        candidate.live_generation = 9;
        reopened.publish(&mut inner, candidate).unwrap();
    }
    let next = ticket(&reopened, &id, true, 2000);
    let current = reopened.vault.load().unwrap().live_receipts[0]
        .archive
        .clone()
        .unwrap();
    assert_eq!(
        (
            current.attempt_id,
            current.deadline_at,
            current.rounds,
            current.requests
        ),
        (
            before.attempt_id,
            before.deadline_at,
            before.rounds,
            before.requests
        )
    );
    assert_eq!(current.identity.creation_owner_generation, 7);
    assert_eq!(current.authorization_generation, 9);
    reopened.release_archive_ticket(&next);
}
#[test]
fn stale_attempt_cannot_replace_new_attempt_or_release_its_owner() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path());
    let id = install(&service);
    let old = ticket(&service, &id, false, 1000);
    service.release_archive_ticket(&old);
    let new = ticket(&service, &id, true, 2000);
    service.stop_archive_ticket(&old, ArchiveReason::Network);
    service.release_archive_ticket(&old);
    let inner = service.lock().unwrap();
    assert!(inner
        .live
        .archive_checks
        .get(&id)
        .is_some_and(|c| Arc::ptr_eq(c, &new.cancel)));
    assert_eq!(
        inner.catalog.live_receipts[0]
            .archive
            .as_ref()
            .unwrap()
            .state,
        ArchiveState::Pending
    );
}
#[test]
fn unjoined_or_unverified_owner_receipt_never_admits_a_check() {
    for mode in 0..3 {
        let temp = tempfile::tempdir().unwrap();
        let service = service(temp.path());
        let id = install(&service);
        let mut inner = service.lock().unwrap();
        let mut candidate = inner.catalog.clone();
        let receipt = &mut candidate.live_receipts[0];
        if mode == 0 {
            receipt.view.sidecar_joined = false;
        } else if mode == 1 {
            receipt.stream_owner_verified = false;
        } else {
            receipt.view.outcome_uncertain = true;
        }
        service.publish(&mut inner, candidate).unwrap();
        let owner = current_owner(&inner).unwrap();
        assert!(matches!(
            service.reserve_archive(&mut inner, &id, owner, true, 1000),
            Err(ArchiveReason::IdentityMismatch)
        ));
        assert!(inner.live.archive_checks.is_empty());
    }
}
