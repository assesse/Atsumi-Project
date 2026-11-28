use super::*;

fn ready(service: &CloudService) -> Job {
    let mut job = install_job(service);
    job.view.source.recording_id = uuid::Uuid::new_v4().to_string();
    job.view.state = "ready".into();
    job.view.video_id = Some(VIDEO.into());
    job.view.total_bytes = 100;
    job.view.uploaded_bytes = 100;
    job.view.duration_seconds = 44.0;
    job.source_sha256 = Some("a".repeat(64));
    let mut inner = service.lock().unwrap();
    let mut candidate = inner.catalog.clone();
    *candidate.jobs.iter_mut().find(|value| value.view.id == job.view.id).unwrap() = job.clone();
    service.publish(&mut inner, candidate).unwrap();
    job
}

#[test]
fn unlisted_replay_read_uses_persisted_receipt_without_grant_or_source_io() {
    let root = tempfile::tempdir().unwrap();
    let mock = Mock::new(vec![]);
    let service = make_service_with_source(root.path(), mock.clone(), Arc::new(|_,_| panic!("replay must not resolve video")));
    let job = ready(&service);
    {
        let mut inner = service.lock().unwrap();
        let mut candidate = inner.catalog.clone(); candidate.grant = None;
        service.publish(&mut inner, candidate).unwrap();
    }
    let before = std::fs::read(root.path().join("app/cloud-recording/state.bin")).unwrap();
    let authorization = service.replay_authorization(&root.path().join("app"), &job.view.id).unwrap();
    assert_eq!(authorization.owner_channel_id, OWNER);
    assert_eq!(authorization.original_inputs[0].sha256, "a".repeat(64));
    assert_eq!(authorization.original_inputs[0].part_index, None);
    assert!(!root.path().join("source.mp4").exists());
    assert!(mock.requests.lock().unwrap().is_empty());
    assert_eq!(before, std::fs::read(root.path().join("app/cloud-recording/state.bin")).unwrap());
    let view = service.snapshot().unwrap().jobs.into_iter().find(|value| value.id == job.view.id).unwrap();
    assert_eq!(view.replay_cohort_id.as_deref(), Some(authorization.cohort_id.as_str()));
    assert_eq!(view.state, "ready");
    assert!(service.resume(&job.view.id).is_err());
}

#[test]
fn other_upload_owner_can_read_unlisted_but_does_not_gain_mutation_authority() {
    let root = tempfile::tempdir().unwrap();
    let service = make_service_with_source(root.path(), Mock::new(vec![]), Arc::new(|_,_| panic!("no video reads")));
    let job = ready(&service);
    {
        let mut inner = service.lock().unwrap();
        let mut candidate = inner.catalog.clone(); candidate.grant.as_mut().unwrap().channel_id = OTHER.into();
        service.publish(&mut inner, candidate).unwrap();
    }
    assert!(service.replay_authorization(&root.path().join("app"), &job.view.id).is_ok());
    assert_eq!(service.resume(&job.view.id).unwrap_err().code, "CLOUD_ACCOUNT_MISMATCH");
}

#[test]
fn nonready_invalid_receipts_and_forged_identities_never_authorize_chat() {
    let root = tempfile::tempdir().unwrap();
    let service = make_service(root.path(), Mock::new(vec![]));
    let job = ready(&service);
    let mutations: [fn(&mut Job); 11] = [
        |job: &mut Job| job.view.state = "processing".into(),
        |job: &mut Job| job.view.recording_plan = true,
        |job: &mut Job| job.view.uploaded_bytes -= 1,
        |job: &mut Job| job.source_sha256 = None,
        |job: &mut Job| job.view.video_id = None,
        |job: &mut Job| job.view.duration_seconds = f64::NAN,
        |job: &mut Job| job.view.media_offset_seconds = f64::INFINITY,
        |job: &mut Job| job.channel_id = "invalid-owner".into(),
        |job: &mut Job| job.view.source.provider = "other".into(),
        |job: &mut Job| job.view.source.recording_id = "../other".into(),
        |job: &mut Job| { job.view.source.plan_id = Some(uuid::Uuid::new_v4().to_string()); job.view.source.unit_id = Some(uuid::Uuid::new_v4().to_string()); },
    ];
    for mutate in mutations {
        let mut broken = job.clone(); mutate(&mut broken);
        service.lock().unwrap().catalog.jobs[0] = broken;
        assert!(service.replay_authorization(&root.path().join("app"), &job.view.id).is_err());
    }
    service.lock().unwrap().catalog.jobs[0] = job.clone();
    for invalid in [String::new(), "../state.bin".into(), uuid::Uuid::new_v4().to_string(), "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA".into()] {
        assert!(service.replay_authorization(&root.path().join("app"), &invalid).is_err());
    }
}

#[test]
fn cohort_ignores_polling_and_units_but_separates_owners_and_original_generation() {
    let root = tempfile::tempdir().unwrap();
    let service = make_service(root.path(), Mock::new(vec![]));
    let mut job = ready(&service);
    let original = replay::cohort_id(&job).unwrap();
    job.view.updated_at += 100; job.view.state = "paused".into();
    assert_eq!(replay::cohort_id(&job).unwrap(), original);
    job.channel_id = OTHER.into(); assert_ne!(replay::cohort_id(&job).unwrap(), original);
    job.channel_id = OWNER.into(); job.source_sha256 = Some("b".repeat(64));
    assert_ne!(replay::cohort_id(&job).unwrap(), original);
    job.view.source.plan_id = Some(uuid::Uuid::new_v4().to_string());
    job.view.source.unit_id = Some(uuid::Uuid::new_v4().to_string());
    let planned = replay::cohort_id(&job).unwrap();
    job.view.source.unit_id = Some(uuid::Uuid::new_v4().to_string()); job.source_sha256 = Some("c".repeat(64));
    assert_eq!(replay::cohort_id(&job).unwrap(), planned);
    job.view.source.plan_id = Some(uuid::Uuid::new_v4().to_string());
    assert_ne!(replay::cohort_id(&job).unwrap(), planned);
}

#[test]
fn legacy_selected_part_never_promotes_parent_coverage_and_exposes_no_secrets() {
    let root = tempfile::tempdir().unwrap();
    let service = make_service(root.path(), Mock::new(vec![]));
    let mut job = ready(&service);
    job.view.source = CloudSourceRef { provider: "chzzk".into(), recording_id: uuid::Uuid::new_v4().simple().to_string(), part_index: Some(11), plan_id: None, unit_id: None };
    job.view.media_offset_seconds = 660.0;
    service.lock().unwrap().catalog.jobs[0] = job.clone();
    let authorization = service.replay_authorization(&root.path().join("app"), &job.view.id).unwrap();
    assert_eq!(authorization.original_inputs.len(), 1);
    assert_eq!(authorization.original_inputs[0].part_index, Some(11));
    assert_eq!(authorization.description.coverage.state, "incomplete");
    assert_eq!(authorization.description.parts[0].media_offset_seconds, 660.0);
    let output = replay::CloudReplayOpen { provider: "chzzk".into(), token: "opaque".into(), cohort_id: authorization.cohort_id,
        description: authorization.description, session: None };
    let serialized = serde_json::to_string(&output).unwrap();
    for secret in [OWNER, ACCESS, REFRESH, SESSION, &"a".repeat(64)] { assert!(!serialized.contains(secret)); }
}
