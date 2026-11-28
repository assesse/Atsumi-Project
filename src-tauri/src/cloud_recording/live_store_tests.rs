use super::super::{catalog::Catalog, live_api::ResourceReceipt};
use super::*;

fn receipt() -> LiveReceipt {
    LiveReceipt::selected(
        uuid::Uuid::new_v4().to_string(),
        SourceFacts {
            provider: "soop".into(),
            channel_id: "fixture".into(),
            broadcast_key: Some("1".into()),
            session_id: uuid::Uuid::new_v4().to_string(),
            epoch: 1,
            account_revision: 1,
        },
    )
}

#[test]
fn old_catalog_defaults_live_fields_without_authority() {
    let old = r#"{"version":1,"client":null,"grant":null,"jobs":[]}"#;
    let catalog: Catalog = serde_json::from_str(old).unwrap();
    assert!(catalog.valid());
    assert!(catalog.live_grant.is_none() && catalog.live_receipts.is_empty());
    assert_eq!(catalog.live_generation, 0);
}

#[test]
fn receipt_limits_and_closed_shape_reject_forged_success() {
    let item = receipt();
    assert!(item.valid());
    let mut bad = item.clone();
    bad.view.state = "joined".into();
    assert!(!bad.valid());
    bad = item.clone();
    bad.view.archive_ready = true;
    assert!(!bad.valid());
    bad = item.clone();
    bad.view.receipt_persisted = false;
    assert!(!bad.valid());
    bad = item.clone();
    bad.view.stream_id = Some("https://secret.invalid/key".into());
    assert!(!bad.valid());
    let mut value = serde_json::to_value(&item).unwrap();
    value["target"] = "secret".into();
    assert!(serde_json::from_value::<LiveReceipt>(value).is_err());
    let mut list = (0..RECEIPTS).map(|_| receipt()).collect::<Vec<_>>();
    assert!(valid(&list));
    list.push(receipt());
    assert!(!valid(&list));
}

#[test]
fn returned_ids_survive_recovery_without_becoming_restart_authority() {
    let mut item = receipt();
    let progress = PreparationProgress {
        request_id: item.request_id.clone(),
        stage: PreparationStage::CreatingBroadcast,
        stream: Some(ResourceReceipt {
            id: "created-stream".into(),
            owner_verified: true,
        }),
        broadcast: None,
        outcome_uncertain: true,
    };
    item.progress(&progress).unwrap();
    item.view.state = "preparing".into();
    let mut catalog = Catalog::default();
    catalog.live_receipts.push(item);
    catalog.recover();
    let restored = &catalog.live_receipts[0];
    assert!(catalog.valid());
    assert_eq!(restored.view.state, "recovery_required");
    assert_eq!(restored.view.stream_id.as_deref(), Some("created-stream"));
    assert!(restored.view.outcome_uncertain && restored.view.interrupted);
    assert!(!restored.view.archive_ready && restored.view.run_id.is_none());
}

#[test]
fn progress_cannot_change_known_resource_or_request_identity() {
    let mut item = receipt();
    let mut progress = PreparationProgress {
        request_id: item.request_id.clone(),
        stage: PreparationStage::StreamCreated,
        stream: Some(ResourceReceipt {
            id: "first".into(),
            owner_verified: true,
        }),
        broadcast: None,
        outcome_uncertain: false,
    };
    item.progress(&progress).unwrap();
    progress.stream.as_mut().unwrap().id = "second".into();
    assert!(item.progress(&progress).is_err());
    assert_eq!(item.view.stream_id.as_deref(), Some("first"));
    progress.request_id = uuid::Uuid::new_v4().to_string();
    assert!(item.progress(&progress).is_err());
}

#[test]
fn chzzk_missing_broadcast_is_preserved_as_unknown() {
    let mut item = receipt();
    item.source.provider = "chzzk".into();
    item.source.broadcast_key = None;
    item.view.provider = "chzzk".into();
    item.view.broadcast_key = None;
    assert!(item.valid());
    let copy: LiveReceipt = serde_json::from_slice(&serde_json::to_vec(&item).unwrap()).unwrap();
    assert!(copy.view.broadcast_key.is_none());
    item.view.provider = "soop".into();
    item.source.provider = "soop".into();
    assert!(!item.valid());
}
