use super::*;

fn fixture(root: &Path, derived: bool) -> Plan {
    let reference = CloudSourceRef { provider: "soop".into(), recording_id: uuid::Uuid::new_v4().to_string(), part_index: None, plan_id: None, unit_id: None };
    let inputs = vec![Identity { bytes: 100, modified_nanos: 1_700_000_000_000_000_000, digest: Some("a".repeat(64)), duration: 44.0, offset: 0.0 }];
    let fingerprint = digest(&serde_json::to_vec(&(reference.clone(), "", &inputs)).unwrap());
    let units = if derived { vec![Unit { first: 0, end: 1, cut_start: Some(0.0), cut_end: Some(20.0) }, Unit { first: 0, end: 1, cut_start: Some(20.0), cut_end: Some(44.0) }] }
        else { vec![Unit { first: 0, end: 1, cut_start: None, cut_end: None }] };
    let plan_id = id(&serde_json::to_vec(&(4u32, &fingerprint, MAX_SECONDS, MAX_BYTES, &units)).unwrap());
    let parts = units.iter().enumerate().map(|(index, unit)| {
        let mut source = reference.clone(); source.plan_id = Some(plan_id.clone()); source.unit_id = Some(id(format!("{plan_id}:{index}").as_bytes()));
        SourcePart { source, unit_index: index as u32, part_index: None, duration_seconds: unit.cut_end.unwrap_or(44.0)-unit.cut_start.unwrap_or(0.0),
            media_offset_seconds: unit.cut_start.unwrap_or(0.0), bytes: if derived { estimated(100) } else { 100 }, kind: if derived { "derived" } else { "direct" }.into() }
    }).collect();
    let plan = Plan { version: 1, description: SourceDescription { source: reference, title: "Fixture".into(), plan_id: Some(plan_id),
        coverage: SourceCoverage { state: "complete".into(), expected_segments: None, covered_segments: None, missing_from_segment: None, reason: None, capture_warning: None },
        preparation_required: false, duration_seconds: 44.0, bytes: 100, parts }, fingerprint, input_parts: false, inputs, units };
    save(root, &plan).unwrap(); plan
}

#[test]
fn persisted_direct_plan_binds_receipt_without_video_or_current_grant() {
    let directory = tempfile::tempdir().unwrap();
    let plan = fixture(directory.path(), false);
    let reference = &plan.description.parts[0].source;
    let (description, inputs, fingerprint, parts) = replay_binding(directory.path(), reference, &"a".repeat(64), 100, 44.0, 0.0).unwrap();
    assert_eq!(description.coverage.state, "complete"); assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].part_index, None); assert_eq!(inputs[0].sha256, "a".repeat(64));
    assert!(!parts); assert_eq!(fingerprint, Some(plan.fingerprint));
    assert!(replay_binding(directory.path(), reference, &"b".repeat(64), 100, 44.0, 0.0).is_err());
    assert!(replay_binding(directory.path(), reference, &"a".repeat(64), 99, 44.0, 0.0).is_err());
    assert!(replay_binding(directory.path(), reference, &"a".repeat(64), 100, 43.0, 0.0).is_err());
    let mut forged = reference.clone(); forged.recording_id = uuid::Uuid::new_v4().to_string();
    assert!(replay_binding(directory.path(), &forged, &"a".repeat(64), 100, 44.0, 0.0).is_err());
}

#[test]
fn derived_unit_retains_original_identity_and_rejects_changed_input_fingerprint() {
    let directory = tempfile::tempdir().unwrap();
    let plan = fixture(directory.path(), true);
    let reference = &plan.description.parts[1].source;
    let (description, inputs, fingerprint, input_parts) = replay_binding(directory.path(), reference, &"b".repeat(64), 42, 24.0, 20.0).unwrap();
    assert_eq!(description.parts.len(), 2); assert_eq!(inputs[0].sha256, "a".repeat(64));
    let mut authorization = super::super::super::replay::CloudReplayAuthorization {
        job_id: uuid::Uuid::new_v4().to_string(), source: reference.clone(), owner_channel_id: "UCaaaaaaaaaaaaaaaaaaaaaa".into(),
        cohort_id: "opaque".into(), source_sha256: "b".repeat(64), bytes: 42, duration_seconds: 24.0, media_offset_seconds: 20.0,
        original_inputs: inputs, plan_fingerprint: fingerprint, input_parts, description,
    };
    assert!(authorization.matches_recording_generation(""));
    authorization.original_inputs[0].sha256 = "c".repeat(64);
    assert!(!authorization.matches_recording_generation(""));
    assert!(!authorization.matches_recording_generation("other"));
    let mut forged = reference.clone(); forged.unit_id = Some(uuid::Uuid::new_v4().to_string());
    assert!(replay_binding(directory.path(), &forged, &"b".repeat(64), 42, 24.0, 20.0).is_err());
}
