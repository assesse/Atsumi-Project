use super::super::{
    live_api::{PreparationStage, ResourceReceipt},
    upload::{Request, Response, Transport},
};
use super::*;
use serde_json::json;
use std::sync::atomic::AtomicUsize;
const OWNER: &str = "UCabcdefghijklmnopqrstuv";
struct NoNetwork;
impl Transport for NoNetwork {
    fn send(&self, _: Request) -> Result<Response, CloudError> {
        panic!("network forbidden")
    }
}
fn service(root: &std::path::Path) -> Arc<CloudService> {
    CloudService::with_transport(
        root.to_owned(),
        Arc::new(|_, _| Err("no source".into())),
        Arc::new(|_| Err("no browser".into())),
        Arc::new(NoNetwork),
    )
    .unwrap()
}
fn install(service: &CloudService) -> (String, Arc<LiveRelayCoordinator>) {
    let source = NativeRelaySource::Soop(crate::soop::original::SoopRelaySource::test_fixture());
    let coordinator = Arc::new(LiveRelayCoordinator::select_native(source.clone()));
    let id = coordinator.status().selection_id;
    let mut inner = service.lock().unwrap();
    let mut candidate = inner.catalog.clone();
    candidate.client = Some(oauth::ClientConfig {
        client_id: "123-fixture.apps.googleusercontent.com".into(),
        client_secret: "fixture-secret".into(),
    });
    let grant = oauth::Grant {
        access_token: "fixture-access".into(),
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
    candidate
        .live_receipts
        .push(LiveReceipt::selected(id.clone(), source.facts()));
    service.publish(&mut inner, candidate).unwrap();
    inner.live.operations.insert(
        id.clone(),
        Operation {
            source,
            coordinator: coordinator.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            worker: false,
        },
    );
    (id, coordinator)
}
fn plan(service: &CloudService, id: &str) -> String {
    let mut inner = service.lock().unwrap();
    service
        .update_live_receipt(&mut inner, id, |r| {
            r.plan = Some(CloudLivePlan {
                title: "Controlled fixture".into(),
                scheduled_start_ms: now(),
                made_for_kids: false,
            });
            r.view.state = "preparing".into();
            r.changed()
        })
        .unwrap();
    inner
        .catalog
        .live_receipts
        .iter()
        .find(|r| r.view.selection_id == id)
        .unwrap()
        .request_id
        .clone()
}
fn auth() -> (NativeDestinationOwner, LiveAuthorization) {
    let owner = NativeDestinationOwner {
        channel_id: OWNER.into(),
        generation: 7,
    };
    let auth = LiveAuthorization::from_granted_scopes(
        "fixture-access".into(),
        owner.clone(),
        "https://www.googleapis.com/auth/youtube.force-ssl",
    )
    .unwrap();
    (owner, auth)
}

#[test]
fn selected_stop_retires_runtime_and_preserves_upload_grant() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let (id, _) = install(&service);
    let before = serde_json::to_vec(&service.lock().unwrap().catalog.grant).unwrap();
    let snapshot = service.stop_live(&id, 1).unwrap();
    assert_eq!(snapshot.operations[0].state, "failed");
    assert!(snapshot.operations[0].stop_requested);
    assert!(service.lock().unwrap().live.operations.is_empty());
    assert_eq!(
        serde_json::to_vec(&service.lock().unwrap().catalog.grant).unwrap(),
        before
    );
}

#[test]
fn normal_shutdown_changes_auth_nonce_not_destination_generation() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let (id, coordinator) = install(&service);
    service
        .lock()
        .unwrap()
        .live
        .operations
        .get_mut(&id)
        .unwrap()
        .worker = true;
    service.request_live_shutdown().unwrap();
    let inner = service.lock().unwrap();
    assert!(inner.closing);
    assert_eq!(inner.catalog.live_generation, 7);
    assert_eq!(inner.live.auth_nonce, 1);
    assert!(inner.live.operations[&id].cancel.load(Ordering::Acquire));
    assert_eq!(inner.live.active_ids(), vec![format!("live:{id}")]);
    drop(inner);
    let view = coordinator.status();
    assert!(view.stop_requested);
    assert!(!view.interrupted && view.fault.is_none());
}

#[test]
fn live_only_restart_persists_recovery_and_never_recreates_runtime() {
    let root = tempfile::tempdir().unwrap();
    let first = service(root.path());
    let (id, _) = install(&first);
    plan(&first, &id);
    drop(first);
    let reopened = service(root.path());
    let snapshot = reopened.live_snapshot().unwrap();
    assert_eq!(snapshot.operations[0].state, "recovery_required");
    assert!(reopened.lock().unwrap().live.operations.is_empty());
    assert_eq!(
        reopened.vault.load().unwrap().live_receipts[0].view.state,
        "recovery_required"
    );
}

#[test]
fn durable_progress_failure_preserves_returned_id_as_nonpersisted_and_first_fault() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let (id, _) = install(&service);
    let request = plan(&service, &id);
    service.vault.fail_once();
    let progress = PreparationProgress {
        request_id: request,
        stage: PreparationStage::CreatingStream,
        stream: Some(ResourceReceipt {
            id: "returned-stream".into(),
            owner_verified: true,
        }),
        broadcast: None,
        outcome_uncertain: false,
    };
    assert!(service.live_progress(&id, &progress).is_err());
    let _ = service.live_fail(&id, stopped());
    let view = &service.live_snapshot().unwrap().operations[0];
    assert!(!view.receipt_persisted);
    assert_eq!(view.stream_id.as_deref(), Some("returned-stream"));
    assert_eq!(view.error.as_ref().unwrap().code, "CLOUD_STORAGE");
    assert!(service.vault.load().unwrap().live_receipts[0]
        .view
        .stream_id
        .is_none());
}

struct CancelResponse {
    service: Arc<CloudService>,
    cancel: Arc<AtomicBool>,
    calls: AtomicUsize,
    fail_save: bool,
}
impl Transport for CancelResponse {
    fn send(&self, _: Request) -> Result<Response, CloudError> {
        assert_eq!(
            self.calls.fetch_add(1, Ordering::AcqRel),
            0,
            "must not create next resource"
        );
        self.cancel.store(true, Ordering::Release);
        if self.fail_save {
            self.service.vault.fail_once();
        }
        Ok(Response {
            status: 200,
            headers: vec![],
            body: serde_json::to_vec(
                &json!({"id":"returned-stream","snippet":{"channelId":OWNER}}),
            )
            .unwrap(),
        })
    }
}
#[test]
fn cancellation_during_remote_response_preserves_id_before_stale_check() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let (id, _) = install(&service);
    let request = plan(&service, &id);
    let cancel = Arc::new(AtomicBool::new(false));
    let transport = CancelResponse {
        service: service.clone(),
        cancel: cancel.clone(),
        calls: AtomicUsize::new(0),
        fail_save: false,
    };
    let (owner, auth) = auth();
    assert!(service
        .provision_live(&transport, &auth, &id, &request, &owner, &cancel, || true)
        .is_err());
    assert_eq!(transport.calls.load(Ordering::Acquire), 1);
    assert_eq!(
        service.vault.load().unwrap().live_receipts[0]
            .view
            .stream_id
            .as_deref(),
        Some("returned-stream")
    );
}
#[test]
fn response_storage_failure_blocks_next_mutation_and_keeps_memory_recovery_id() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let (id, _) = install(&service);
    let request = plan(&service, &id);
    let cancel = Arc::new(AtomicBool::new(false));
    let transport = CancelResponse {
        service: service.clone(),
        cancel: cancel.clone(),
        calls: AtomicUsize::new(0),
        fail_save: true,
    };
    let (owner, auth) = auth();
    assert!(service
        .provision_live(&transport, &auth, &id, &request, &owner, &cancel, || true)
        .is_err());
    assert_eq!(transport.calls.load(Ordering::Acquire), 1);
    let view = &service.live_snapshot().unwrap().operations[0];
    assert_eq!(view.stream_id.as_deref(), Some("returned-stream"));
    assert!(!view.receipt_persisted);
    assert!(
        service.vault.load().unwrap().live_receipts[0]
            .view
            .outcome_uncertain
    );
}

#[test]
fn ack_statistics_do_not_invalidate_stop_control_revision() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let (id, coordinator) = install(&service);
    let mut view = coordinator.status();
    view.run_id = Some(uuid::Uuid::new_v4().to_string());
    view.relay = Some(crate::streaming::relay_status::Inspection {
        run_id: view.run_id.clone().unwrap(),
        delivery: Some(Default::default()),
        ..Default::default()
    });
    service.live_observe(&id, &view, false).unwrap();
    let revision = service.live_snapshot().unwrap().operations[0].revision;
    view.relay
        .as_mut()
        .unwrap()
        .delivery
        .as_mut()
        .unwrap()
        .segment_acks = 4;
    service.live_observe(&id, &view, false).unwrap();
    assert_eq!(
        service.live_snapshot().unwrap().operations[0].revision,
        revision
    );
    assert_eq!(
        service.live_snapshot().unwrap().operations[0].segment_acks,
        4
    );
    assert!(service.stop_live(&id, revision).is_ok());
}

#[test]
fn client_replacement_invalidates_selected_receipts_without_promoting_live_scope() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let (id, _) = install(&service);
    let file = root.path().join("replacement.json");
    std::fs::write(&file, serde_json::to_vec(&json!({"installed":{"client_id":"456-fixture.apps.googleusercontent.com","client_secret":"other-secret","redirect_uris":["http://localhost"]}})).unwrap()).unwrap();
    service.import_client_path(&file).unwrap();
    let inner = service.lock().unwrap();
    assert!(inner.catalog.live_grant.is_none());
    assert!(inner.live.operations.is_empty());
    assert_eq!(
        inner
            .catalog
            .live_receipts
            .iter()
            .find(|r| r.view.selection_id == id)
            .unwrap()
            .view
            .state,
        "failed"
    );
    assert!(inner.catalog.live_generation > 7);
}

#[test]
fn failed_initial_checkpoint_issues_no_api_request() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let (id, _) = install(&service);
    let request = plan(&service, &id);
    service.vault.fail_once();
    let (owner, auth) = auth();
    assert!(service
        .provision_live(
            &NoNetwork,
            &auth,
            &id,
            &request,
            &owner,
            &AtomicBool::new(false),
            || true
        )
        .is_err());
}

#[test]
fn any_catalog_save_failure_cancels_live_auth_and_preparation_admission() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let (id, _) = install(&service);
    let auth_cancel = Arc::new(AtomicBool::new(false));
    let mut inner = service.lock().unwrap();
    inner.live.auth_cancel = Some(auth_cancel.clone());
    let operation_cancel = inner.live.operations[&id].cancel.clone();
    let candidate = inner.catalog.clone();
    service.vault.fail_once();
    assert!(service.publish(&mut inner, candidate).is_err());
    assert!(auth_cancel.load(Ordering::Acquire));
    assert!(operation_cancel.load(Ordering::Acquire));
    assert_eq!(inner.catalog.live_generation, 7);
}

struct OrderedPreparation {
    service: Arc<CloudService>,
    index: AtomicUsize,
}
impl Transport for OrderedPreparation {
    fn send(&self, request: Request) -> Result<Response, CloudError> {
        let index = self.index.fetch_add(1, Ordering::AcqRel);
        let receipt = &self.service.vault.load().unwrap().live_receipts[0];
        assert!(
            receipt.view.outcome_uncertain,
            "mutation requires durable uncertain intent first"
        );
        assert_eq!(
            receipt.view.preparation_stage.as_deref(),
            Some(["creating_stream", "creating_broadcast", "binding"][index])
        );
        if index > 0 {
            assert_eq!(receipt.view.stream_id.as_deref(), Some("created-stream"));
        }
        if index == 2 {
            assert_eq!(receipt.view.broadcast_id.as_deref(), Some("abcdefghijk"));
        }
        assert!(request.bearer.is_some());
        let value = if index == 0 {
            json!({"kind":"youtube#liveStream","id":"created-stream","snippet":{"channelId":OWNER},
                "cdn":{"ingestionType":"hls","ingestionInfo":{"ingestionAddress":"https://a.upload.youtube.com/http_upload_hls?cid=only-native-key&copy=0&file="}},"contentDetails":{"isReusable":false}})
        } else {
            let mut value = json!({"kind":"youtube#liveBroadcast","id":"abcdefghijk","snippet":{"channelId":OWNER},
                "status":{"privacyStatus":"unlisted","selfDeclaredMadeForKids":false,"lifeCycleStatus":"ready"},
                "contentDetails":{"enableAutoStart":true,"enableAutoStop":true,"enableDvr":true,"enableEmbed":true,"recordFromStart":true,"monitorStream":{"enableMonitorStream":false}}});
            if index == 2 {
                value["contentDetails"]["boundStreamId"] = "created-stream".into();
            }
            value
        };
        Ok(Response {
            status: 200,
            headers: vec![],
            body: serde_json::to_vec(&value).unwrap(),
        })
    }
}
#[test]
fn production_provisioner_checkpoints_each_mutation_without_serializing_secret_target() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let (id, _) = install(&service);
    let request = plan(&service, &id);
    let transport = OrderedPreparation {
        service: service.clone(),
        index: AtomicUsize::new(0),
    };
    let (owner, auth) = auth();
    let prepared = service
        .provision_live(
            &transport,
            &auth,
            &id,
            &request,
            &owner,
            &AtomicBool::new(false),
            || true,
        )
        .unwrap();
    assert_eq!(transport.index.load(Ordering::Acquire), 3);
    assert_eq!(prepared.progress.stage, PreparationStage::Prepared);
    let saved = service.vault.load().unwrap();
    let item = &saved.live_receipts[0];
    assert_eq!(item.view.broadcast_id.as_deref(), Some("abcdefghijk"));
    assert!(!item.view.outcome_uncertain && !item.view.archive_ready);
    let encoded = serde_json::to_string(&saved.live_receipts).unwrap();
    assert!(
        !encoded.contains("only-native-key")
            && !encoded.contains("http_upload_hls")
            && !encoded.contains("fixture-access")
    );
}

#[test]
fn joined_video_preserves_separate_chat_image_and_timing_coverage() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let (id, coordinator) = install(&service);
    let mut view = coordinator.status();
    view.run_id = Some(uuid::Uuid::new_v4().to_string());
    view.manager_joined = true;
    view.relay = Some(crate::streaming::relay_status::Inspection {
        run_id: view.run_id.clone().unwrap(),
        manager_joined: true,
        worker_joined: true,
        delivery: Some(crate::streaming::relay_status::DeliveryInspection {
            end_list_acked: true,
            ..Default::default()
        }),
        sidecar: Some(crate::streaming::relay_status::SidecarInspection {
            sidecar_id: uuid::Uuid::new_v4().to_string(),
            joined: true,
            text_state: "partial".into(),
            assets_incomplete: true,
            timing_incomplete: true,
            clock_quality: "receive-monotonic-only".into(),
            ..Default::default()
        }),
        ..Default::default()
    });
    service.live_observe(&id, &view, true).unwrap();
    let saved = service.vault.load().unwrap();
    let row = &saved.live_receipts[0].view;
    assert_eq!(row.state, "joined");
    assert!(row.error.is_none() && !row.archive_ready);
    assert!(row.assets_incomplete && row.timing_incomplete && row.sidecar_joined);
    assert_eq!(row.sidecar_text_state.as_deref(), Some("partial"));
    assert_eq!(row.clock_quality.as_deref(), Some("receive-monotonic-only"));
}
