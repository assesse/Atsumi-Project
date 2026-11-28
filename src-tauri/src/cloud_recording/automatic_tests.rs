use super::*;
use crate::cloud_recording::automatic::{Admission, Origin};

const RECORDING: &str = "915e586b-68d5-447a-884b-60850d7a2b3e";
fn selected_recording() -> CloudAutomaticTarget {
    CloudAutomaticTarget::Recording {
        provider: "soop".into(),
        recording_id: RECORDING.into(),
    }
}
fn selected_channel() -> CloudAutomaticTarget {
    CloudAutomaticTarget::Channel {
        provider: "soop".into(),
        channel_id: "selected_channel".into(),
    }
}
fn ready(at: u64) -> AutomaticCandidate {
    AutomaticCandidate {
        source: CloudSourceRef {
            provider: "soop".into(),
            recording_id: RECORDING.into(),
            part_index: None,
            plan_id: None,
            unit_id: None,
        },
        channel_id: "selected_channel".into(),
        started_at_ms: at,
        readiness: AutomaticReadiness::Ready,
    }
}
fn automatic_service(
    root: &Path,
    transport: Arc<dyn Transport>,
    connected: bool,
) -> Arc<CloudService> {
    let service = CloudService::with_transport_and_planner(
        root.join("app"),
        fixture_source(root.join("source.mp4")),
        Arc::new(|_| Err("unexpected auth".into())),
        transport,
        Some(Arc::new(|source, _| {
            let mut planned = source.clone();
            planned.plan_id = Some("ca707928-0662-44fa-a474-6dc968322987".into());
            planned.unit_id = Some("ed5b757c-98a6-4632-be35-00b0c60ed5b8".into());
            Ok(vec![planned])
        })),
    )
    .unwrap();
    if connected {
        let mut inner = service.lock().unwrap();
        let mut catalog = inner.catalog.clone();
        catalog.client = Some(client());
        catalog.grant = Some(grant());
        service.publish(&mut inner, catalog).unwrap();
    }
    service
}
fn enable(service: &CloudService, target: CloudAutomaticTarget) -> CloudAutomaticRule {
    let revision = service.snapshot().unwrap().automatic.revision;
    service
        .automatic_set_with_visibility(target, true, revision, Some(CloudVisibility::Unlisted))
        .unwrap()
        .automatic
        .rules
        .into_iter()
        .last()
        .unwrap()
}
fn upload_responses() -> Vec<Result<Response, CloudError>> {
    vec![
        opened(),
        offset(None),
        complete(),
        facts(OWNER, "unlisted", "PT44S", "succeeded"),
    ]
}

#[test]
fn no_rules_no_account_no_scan_or_browser_and_explicit_target_validation() {
    let root = fixture();
    let mock = Mock::default();
    let mock = Arc::new(mock);
    let service = automatic_service(root.path(), mock.clone(), false);
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    service
        .start_automatic(Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(vec![ready(now())])
        }))
        .unwrap();
    assert!(service.snapshot().unwrap().automatic.rules.is_empty());
    enable(&service, selected_recording());
    assert_eq!(
        service.snapshot().unwrap().automatic.rules[0].status,
        "waiting_account"
    );
    service.wake.notify_all();
    std::thread::sleep(Duration::from_millis(40));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(mock.requests.lock().unwrap().is_empty());
    let revision = service.snapshot().unwrap().automatic.revision;
    assert!(service
        .automatic_set(
            CloudAutomaticTarget::Channel {
                provider: "youtube".into(),
                channel_id: "anything".into()
            },
            true,
            revision
        )
        .is_err());
    assert!(service
        .automatic_set(selected_channel(), true, revision - 1)
        .is_err());
    service.shutdown_and_wait().unwrap();
}

#[test]
fn future_channel_and_exact_recording_wait_for_completion_then_share_one_upload() {
    let root = fixture();
    let mock = Mock::new(upload_responses());
    let service = automatic_service(root.path(), mock.clone(), true);
    let channel = enable(&service, selected_channel());
    let old = ready(channel.activated_at_ms - 1);
    let mut wrong = ready(channel.activated_at_ms);
    wrong.channel_id = "other".into();
    let mut processing = ready(channel.activated_at_ms);
    processing.readiness = AutomaticReadiness::Processing;
    service
        .reconcile_automatic(vec![old.clone(), wrong, processing])
        .unwrap();
    assert!(service.snapshot().unwrap().jobs.is_empty());
    assert!(mock.requests.lock().unwrap().is_empty());
    enable(&service, selected_recording());
    service
        .reconcile_automatic(vec![old.clone(), ready(channel.activated_at_ms)])
        .unwrap();
    wait_until(|| {
        service
            .snapshot()
            .unwrap()
            .jobs
            .first()
            .is_some_and(|job| job.state == "ready")
    });
    wait_done(&service);
    let snapshot = service.snapshot().unwrap();
    assert_eq!(snapshot.jobs.len(), 1);
    assert_eq!(snapshot.jobs[0].visibility, CloudVisibility::Unlisted);
    assert_eq!(service.vault.load().unwrap().jobs[0].view.visibility, CloudVisibility::Unlisted);
    assert_eq!(snapshot.jobs[0].source.recording_id, RECORDING);
    service
        .reconcile_automatic(vec![old, ready(channel.activated_at_ms)])
        .unwrap();
    assert_eq!(service.snapshot().unwrap().jobs.len(), 1);
    assert_eq!(
        mock.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.method == reqwest::Method::POST
                && request.url.contains("uploadType=resumable"))
            .count(),
        1
    );
    service.shutdown_and_wait().unwrap();
}

#[test]
fn disabled_generation_rejects_late_admission_and_unbound_rules_bind_once() {
    let root = fixture();
    let mock = Mock::new(vec![]);
    let service = automatic_service(root.path(), mock.clone(), true);
    let rule = enable(&service, selected_recording());
    let revision = service.snapshot().unwrap().automatic.revision;
    service
        .automatic_set(selected_recording(), false, revision)
        .unwrap();
    let candidate = ready(1);
    let admission = Admission {
        rule_id: rule.id,
        generation: rule.generation,
        candidate: candidate.clone(),
    };
    assert_eq!(
        service
            .enqueue_kind(vec![candidate.source], true, Some(admission), CloudVisibility::Private)
            .unwrap_err()
            .code,
        "CLOUD_RULE_CHANGED"
    );
    service.reconcile_automatic(vec![ready(1)]).unwrap();
    assert!(service.snapshot().unwrap().jobs.is_empty());
    {
        let mut inner = service.lock().unwrap();
        let mut catalog = inner.catalog.clone();
        let mut other = grant();
        other.channel_id = OTHER.into();
        catalog.grant = Some(other);
        service.publish(&mut inner, catalog).unwrap();
    }
    let revision = service.snapshot().unwrap().automatic.revision;
    assert_eq!(
        service
            .automatic_set(selected_recording(), true, revision)
            .unwrap_err()
            .code,
        "CLOUD_ACCOUNT_MISMATCH"
    );
    assert!(mock.requests.lock().unwrap().is_empty());
    service.shutdown_and_wait().unwrap();
    let root = fixture();
    let service = automatic_service(root.path(), Mock::new(vec![]), false);
    enable(&service, selected_recording());
    {
        let mut inner = service.lock().unwrap();
        let mut catalog = inner.catalog.clone();
        catalog.automatic.bind_unbound(OWNER);
        catalog.automatic.bind_unbound(OTHER);
        catalog.client = Some(client());
        catalog.grant = Some(grant());
        service.publish(&mut inner, catalog).unwrap();
    }
    assert_eq!(
        service.snapshot().unwrap().automatic.rules[0]
            .owner_channel_id
            .as_deref(),
        Some(OWNER)
    );
    service.shutdown_and_wait().unwrap();
}

fn install_auto_paused(
    service: &CloudService,
    rule: &CloudAutomaticRule,
    resume_intent: bool,
) -> String {
    let mut job = install_job(service);
    let id = job.view.id.clone();
    job.view.source = ready(1).source;
    job.view.state = "paused".into();
    job.view.error = Some(CloudError::cancelled());
    job.automatic = Some(Origin {
        rule_id: rule.id.clone(),
        generation: rule.generation,
        resume_intent,
        paused_by_user: false,
    });
    let mut inner = service.lock().unwrap();
    let mut catalog = inner.catalog.clone();
    catalog.jobs[0] = job;
    service.publish(&mut inner, catalog).unwrap();
    id
}

#[test]
fn explicit_pause_survives_reconcile_restart_and_disable_without_upload() {
    let root = fixture();
    let mock = Mock::new(vec![]);
    let service = automatic_service(root.path(), mock.clone(), true);
    let rule = enable(&service, selected_recording());
    let id = install_auto_paused(&service, &rule, true);
    service.pause(&id).unwrap();
    service.reconcile_automatic(vec![ready(1)]).unwrap();
    assert_eq!(service.snapshot().unwrap().jobs[0].state, "paused");
    service.shutdown_and_wait().unwrap();
    drop(service);
    let restored = automatic_service(root.path(), mock.clone(), true);
    restored.reconcile_automatic(vec![ready(1)]).unwrap();
    assert_eq!(restored.snapshot().unwrap().jobs[0].state, "paused");
    let revision = restored.snapshot().unwrap().automatic.revision;
    restored
        .automatic_set(selected_recording(), false, revision)
        .unwrap();
    restored.reconcile_automatic(vec![ready(1)]).unwrap();
    assert!(mock.requests.lock().unwrap().is_empty());
    restored.shutdown_and_wait().unwrap();
}

#[test]
fn automatic_restart_intent_resumes_same_job_and_completed_jobs_stay_deduplicated() {
    let root = fixture();
    let mock = Mock::new(upload_responses());
    let service = automatic_service(root.path(), mock.clone(), true);
    let rule = enable(&service, selected_recording());
    let id = install_auto_paused(&service, &rule, true);
    service.shutdown_and_wait().unwrap();
    drop(service);
    let restored = automatic_service(root.path(), mock.clone(), true);
    restored.reconcile_automatic(vec![ready(1)]).unwrap();
    wait_until(|| restored.snapshot().unwrap().jobs[0].state == "ready");
    wait_done(&restored);
    assert_eq!(restored.snapshot().unwrap().jobs[0].id, id);
    restored.reconcile_automatic(vec![ready(1)]).unwrap();
    assert_eq!(restored.snapshot().unwrap().jobs.len(), 1);
    restored.shutdown_and_wait().unwrap();
}

#[test]
fn manual_existing_upload_is_observed_without_claiming_or_resuming_it() {
    let root = fixture();
    let mock = Mock::new(vec![]);
    let service = automatic_service(root.path(), mock.clone(), true);
    let mut job = install_job(&service);
    job.view.source = ready(1).source;
    job.view.state = "paused".into();
    {
        let mut inner = service.lock().unwrap();
        let mut catalog = inner.catalog.clone();
        catalog.jobs[0] = job;
        service.publish(&mut inner, catalog).unwrap();
    }
    enable(&service, selected_recording());
    service.reconcile_automatic(vec![ready(1)]).unwrap();
    let snapshot = service.snapshot().unwrap();
    assert_eq!(snapshot.jobs.len(), 1);
    assert_eq!(snapshot.automatic.rules[0].status, "paused");
    assert!(service.lock().unwrap().catalog.jobs[0].automatic.is_none());
    assert!(mock.requests.lock().unwrap().is_empty());
    service.shutdown_and_wait().unwrap();
}

#[test]
fn actual_supervisor_isolates_provider_failure_and_drains_normally() {
    let root = fixture();
    let mock = Mock::new(upload_responses());
    let service = automatic_service(root.path(), mock.clone(), true);
    enable(&service, selected_recording());
    enable(
        &service,
        CloudAutomaticTarget::Channel {
            provider: "chzzk".into(),
            channel_id: "a".repeat(32),
        },
    );
    service
        .start_automatic(Arc::new(|rules| {
            if rules[0].target.provider() == "chzzk" {
                Err("fixture source unavailable".into())
            } else {
                Ok(vec![ready(1)])
            }
        }))
        .unwrap();
    wait_until(|| {
        service
            .snapshot()
            .unwrap()
            .jobs
            .first()
            .is_some_and(|job| job.state == "ready")
    });
    wait_until(|| service.snapshot().unwrap().automatic.error.is_some());
    assert_eq!(service.snapshot().unwrap().jobs.len(), 1);
    service.shutdown_and_wait().unwrap();
    assert!(!service.lock().unwrap().automatic_running);
}

#[test]
fn overlapping_explicit_selection_takes_over_disabled_rule_but_never_user_pause() {
    for user_pause in [false, true] {
        let root = fixture();
        let mock = Mock::new(if user_pause {
            vec![]
        } else {
            upload_responses()
        });
        let service = automatic_service(root.path(), mock.clone(), true);
        let channel = enable(&service, selected_channel());
        let id = install_auto_paused(&service, &channel, true);
        if user_pause {
            service.pause(&id).unwrap();
        }
        enable(&service, selected_recording());
        let revision = service.snapshot().unwrap().automatic.revision;
        service
            .automatic_set(selected_channel(), false, revision)
            .unwrap();
        service
            .reconcile_automatic(vec![ready(channel.activated_at_ms)])
            .unwrap();
        if user_pause {
            assert_eq!(service.snapshot().unwrap().jobs[0].state, "paused");
            assert!(mock.requests.lock().unwrap().is_empty());
        } else {
            wait_until(|| service.snapshot().unwrap().jobs[0].state == "ready");
            assert_eq!(service.snapshot().unwrap().jobs[0].id, id);
        }
        service.shutdown_and_wait().unwrap();
    }
}

#[test]
fn legacy_catalog_defaults_to_no_rules_and_unknown_new_config_is_not_discarded() {
    let mut old = serde_json::to_value(catalog::Catalog::default()).unwrap();
    old.as_object_mut().unwrap().remove("automatic");
    let parsed: catalog::Catalog = serde_json::from_value(old).unwrap();
    assert!(parsed.valid());
    assert_eq!(parsed.automatic.revision, 1);
    let mut value = serde_json::to_value(catalog::Catalog::default()).unwrap();
    value["automatic"]["unsupportedFutureField"] = json!(true);
    assert!(serde_json::from_value::<catalog::Catalog>(value).is_err());
}

#[test]
fn new_automatic_selection_defaults_private_and_future_edits_do_not_change_admitted_jobs() {
    let root = fixture();
    let mock = Mock::new(vec![opened(), offset(None), complete(), facts(OWNER,"private","PT44S","succeeded")]);
    let service = automatic_service(root.path(), mock.clone(), true);
    let revision = service.snapshot().unwrap().automatic.revision;
    let selected = service.automatic_set(selected_recording(), true, revision).unwrap();
    let rule = &selected.automatic.rules[0];
    assert_eq!(rule.visibility, CloudVisibility::Private);
    service.reconcile_automatic(vec![ready(1)]).unwrap();
    wait_until(|| service.snapshot().unwrap().jobs.first().is_some_and(|job| job.state == "ready"));
    wait_done(&service);
    let before = service.snapshot().unwrap();
    let changed = service.automatic_set_with_visibility(selected_recording(), true, before.automatic.revision, Some(CloudVisibility::Unlisted)).unwrap();
    assert_eq!(changed.automatic.rules[0].visibility, CloudVisibility::Unlisted);
    assert_eq!(changed.automatic.rules[0].generation, rule.generation);
    assert_eq!(changed.automatic.rules[0].activated_at_ms, rule.activated_at_ms);
    service.reconcile_automatic(vec![ready(1)]).unwrap();
    assert_eq!(service.snapshot().unwrap().jobs.len(), 1);
    assert_eq!(service.snapshot().unwrap().jobs[0].visibility, CloudVisibility::Private);
    assert_eq!(mock.requests.lock().unwrap().len(), 4);
    assert_eq!(service.vault.load().unwrap().jobs[0].view.visibility, CloudVisibility::Private);
    service.shutdown_and_wait().unwrap();
}

#[test]
fn old_selected_rules_keep_link_access_and_reject_unknown_visibility() {
    let root = fixture();
    let service = automatic_service(root.path(), Mock::new(vec![]), true);
    enable(&service, selected_recording());
    let mut value = serde_json::to_value(service.vault.load().unwrap()).unwrap();
    value["automatic"]["rules"][0].as_object_mut().unwrap().remove("visibility");
    let recovered: catalog::Catalog = serde_json::from_value(value.clone()).unwrap();
    let saved = serde_json::to_value(recovered).unwrap();
    assert_eq!(saved["automatic"]["rules"][0]["visibility"], "unlisted");
    value["automatic"]["rules"][0]["visibility"] = json!("public");
    assert!(serde_json::from_value::<catalog::Catalog>(value).is_err());
    service.shutdown_and_wait().unwrap();
}
