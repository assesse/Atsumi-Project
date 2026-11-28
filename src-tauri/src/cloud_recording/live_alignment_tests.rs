use super::*;
use crate::cloud_recording::live_fingerprint::{FrameReference, ReferenceWindow};
use crate::cloud_recording::relay_timing_read::{
    Dialect, RemoteAnchor, SourceEnvelope, SourceTicks,
};

fn feature(index: u64, window: u64) -> FrameFeature {
    let mut x = (index + 1).wrapping_mul(0x9e3779b97f4a7c15)
        ^ (window + 17).wrapping_mul(0xd1b54a32d192ed03);
    let mut gradient = [0; 4];
    for value in &mut gradient {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *value = x as u32;
    }
    FrameFeature {
        gradient,
        mean: 120,
        spread: 38,
    }
}
fn fixture(
    offset: i64,
) -> (
    ReferenceSet,
    RelayTimingSummary,
    AlignmentBinding,
    Vec<RemoteWindow>,
) {
    let binding = AlignmentBinding {
        run_id: Uuid::new_v4(),
        source_id: Uuid::new_v4(),
        stream_id: Uuid::new_v4(),
        video_id: "NativeVid_1".into(),
        remote_source_id: Uuid::new_v4(),
        document_epoch: 8,
        source_generation: 3,
    };
    let windows: Vec<_> = (0..3)
        .map(|index| ReferenceWindow {
            ordinal: 1 + index * 9,
            segment_sha256: format!("{:064x}", index + 1),
            source_origin_ticks: 0,
            source_origin_timescale: 90_000,
            source_video_timescale: 90_000,
            frames: (0..24)
                .map(|frame| FrameReference {
                    pts90k: (13 + index * 20) * HZ + frame * 9000,
                    feature: feature(frame, index),
                })
                .collect(),
        })
        .collect();
    let remote = windows
        .iter()
        .enumerate()
        .map(|(index, w)| RemoteWindow {
            capture_id: Uuid::new_v4(),
            video_id: binding.video_id.clone(),
            document_epoch: binding.document_epoch,
            source_id: binding.remote_source_id,
            source_generation: binding.source_generation,
            seek_generation: index as u64 + 1,
            ad_observed: false,
            seeking_observed: false,
            buffering_observed: false,
            frames: w
                .frames
                .iter()
                .map(|frame| RemoteFrame {
                    media_pts90k: (frame.pts90k as i64 - offset) as u64,
                    uncertainty_ticks: 1,
                    feature: frame.feature.clone(),
                })
                .collect(),
        })
        .collect();
    let source = ReferenceSet {
        version: 1,
        run_id: binding.run_id,
        source_id: binding.source_id,
        stream_id: Some(binding.stream_id),
        windows,
        acknowledged_segments: 30,
        sampled_segments: 3,
        queue_dropped: 0,
        decode_failures: 0,
        static_windows: 0,
        cancelled: false,
        incomplete: false,
        remote_anchor: "unknown".into(),
    };
    let envelope = SourceEnvelope {
        timescale: 90_000,
        first_dts: 0,
        last_dts_end: 60 * HZ,
        min_pts: 0,
        max_pts_end: 60 * HZ,
        samples: 1800,
    };
    let timing = RelayTimingSummary {
        dialect: Dialect::Soop,
        records: 31,
        acknowledged_segments: 30,
        prefix_segments: 30,
        transport_end_90k: Some(60 * HZ),
        stream_id: Some(binding.stream_id),
        origin: Some(SourceTicks {
            ticks: 0,
            timescale: 90_000,
        }),
        video: Some(envelope),
        audio: Some(envelope),
        end_seen: true,
        transport_complete: true,
        timing_incomplete: false,
        sidecar_incomplete: false,
        observations: 0,
        usable_observations: 0,
        unknown_observations: 0,
        preroll: None,
        remote_anchor: RemoteAnchor::Unknown,
    };
    (source, timing, binding, remote)
}
fn error(result: Result<MeasuredAlignment>, expected: AlignmentError) {
    assert_eq!(result.unwrap_err(), expected);
}
#[test]
fn three_dynamic_transcoded_windows_measure_bounded_signed_offset() {
    for offset in [0, 10 * HZ as i64, -5 * (HZ as i64)] {
        let (source, timing, binding, mut remote) = fixture(offset);
        for w in &mut remote {
            for f in &mut w.frames {
                f.feature.gradient[0] ^= 0b10101;
                f.feature.gradient[2] ^= 0b10;
                f.feature.mean += 3;
                f.feature.spread -= 2;
            }
        }
        let result = align(&source, &timing, &binding, &remote).unwrap();
        assert!(result.offset.lower <= offset && result.offset.upper >= offset);
        assert!(result.offset.upper - result.offset.lower <= MAX_OFFSET_WIDTH);
        assert_eq!(result.windows.len(), 3);
        assert!(!result.two_window_estimate);
        assert!(result
            .windows
            .iter()
            .all(|w| w.matched_frames >= 20 && w.mean_hamming_milli == 4000));
        assert_eq!(result.binding, binding);
    }
}
#[test]
fn sample_phase_does_not_create_false_subframe_precision() {
    let offset = 10 * HZ as i64;
    let (source, timing, binding, mut remote) = fixture(offset);
    for w in &mut remote {
        for f in &mut w.frames {
            f.media_pts90k += 2700;
        }
    }
    let result = align(&source, &timing, &binding, &remote).unwrap();
    assert!(result.offset.lower <= offset && result.offset.upper >= offset);
    assert!(result.offset.upper - result.offset.lower >= 9000);
}
#[test]
fn two_distinct_windows_remain_explicit_and_unavailable_history_is_preserved() {
    let (mut source, mut timing, binding, mut remote) = fixture(10 * HZ as i64);
    source.windows.remove(1);
    source.incomplete = true;
    remote.remove(1);
    timing.sidecar_incomplete = true;
    let result = align(&source, &timing, &binding, &remote).unwrap();
    assert!(result.two_window_estimate);
    assert!(result.history_incomplete);
    assert_eq!(result.windows.len(), 2);
    remote.pop();
    error(
        align(&source, &timing, &binding, &remote),
        AlignmentError::InsufficientWindows,
    );
}
#[test]
fn advertisements_buffering_seek_boundaries_and_foreign_generation_are_rejected() {
    let (source, timing, binding, remote) = fixture(10 * HZ as i64);
    let mut changed = remote.clone();
    changed[1].ad_observed = true;
    error(
        align(&source, &timing, &binding, &changed),
        AlignmentError::Advertisement,
    );
    let mut changed = remote.clone();
    changed[1].seeking_observed = true;
    error(
        align(&source, &timing, &binding, &changed),
        AlignmentError::Boundary,
    );
    let mut changed = remote.clone();
    changed[1].buffering_observed = true;
    error(
        align(&source, &timing, &binding, &changed),
        AlignmentError::Boundary,
    );
    let mut changed = remote.clone();
    changed[1].source_generation += 1;
    error(
        align(&source, &timing, &binding, &changed),
        AlignmentError::RemoteIdentity,
    );
    let mut changed = remote.clone();
    changed[1].document_epoch += 1;
    error(
        align(&source, &timing, &binding, &changed),
        AlignmentError::RemoteIdentity,
    );
    let mut changed = remote.clone();
    changed[1].source_id = Uuid::new_v4();
    error(
        align(&source, &timing, &binding, &changed),
        AlignmentError::RemoteIdentity,
    );
    let mut changed = remote.clone();
    changed[1].video_id = "OtherVid__1".into();
    error(
        align(&source, &timing, &binding, &changed),
        AlignmentError::RemoteIdentity,
    );
    let mut changed = remote.clone();
    changed[1].seek_generation = 0;
    error(
        align(&source, &timing, &binding, &changed),
        AlignmentError::Boundary,
    );
    let mut changed = remote.clone();
    changed[1].capture_id = changed[0].capture_id;
    error(
        align(&source, &timing, &binding, &changed),
        AlignmentError::Boundary,
    );
    // Stable, separately requested increasing seek generations are intentional.
    assert!(align(&source, &timing, &binding, &remote).is_ok());
}
#[test]
fn static_or_periodic_content_cannot_choose_an_arbitrary_phase() {
    let (mut source, timing, binding, mut remote) = fixture(10 * HZ as i64);
    for f in &mut source.windows[0].frames {
        f.feature = feature(0, 0);
    }
    error(
        align(&source, &timing, &binding, &remote),
        AlignmentError::Static,
    );
    for (i, f) in source.windows[0].frames.iter_mut().enumerate() {
        f.feature = feature(i as u64 % 8, 0);
    }
    remote[0].frames.truncate(8);
    for (i, f) in remote[0].frames.iter_mut().enumerate() {
        f.feature = feature(i as u64, 0);
    }
    error(
        align(&source, &timing, &binding, &remote),
        AlignmentError::Ambiguous,
    );
}
#[test]
fn repeated_windows_and_reused_content_do_not_supply_independent_anchors() {
    let (mut source, timing, binding, remote) = fixture(10 * HZ as i64);
    let features: Vec<_> = source.windows[0]
        .frames
        .iter()
        .map(|f| f.feature.clone())
        .collect();
    for (frame, feature) in source.windows[1].frames.iter_mut().zip(&features) {
        frame.feature = feature.clone();
    }
    error(
        align(&source, &timing, &binding, &remote),
        AlignmentError::Ambiguous,
    );
    let (source, timing, binding, mut remote) = fixture(10 * HZ as i64);
    for (frame, feature) in remote[1].frames.iter_mut().zip(features) {
        frame.feature = feature;
    }
    error(
        align(&source, &timing, &binding, &remote),
        AlignmentError::ReusedWindow,
    );
}
#[test]
fn drifting_or_edited_remote_timeline_is_not_repaired_with_wallclock() {
    let (source, timing, binding, mut remote) = fixture(10 * HZ as i64);
    for frame in &mut remote[1].frames {
        frame.media_pts90k += HZ;
    }
    error(
        align(&source, &timing, &binding, &remote),
        AlignmentError::OffsetDisagreement,
    );
}
#[test]
fn unknown_transport_ownership_and_partial_collection_do_not_gain_authority() {
    let (source, timing, binding, remote) = fixture(10 * HZ as i64);
    let mut bad = source.clone();
    bad.run_id = Uuid::new_v4();
    error(
        align(&bad, &timing, &binding, &remote),
        AlignmentError::SourceIdentity,
    );
    let mut bad = source.clone();
    bad.queue_dropped = 1;
    error(
        align(&bad, &timing, &binding, &remote),
        AlignmentError::IncompleteSource,
    );
    let mut bad = source.clone();
    bad.decode_failures = 1;
    error(
        align(&bad, &timing, &binding, &remote),
        AlignmentError::IncompleteSource,
    );
    let mut bad = source.clone();
    bad.windows[1].source_origin_ticks = 1;
    error(
        align(&bad, &timing, &binding, &remote),
        AlignmentError::SourceShape,
    );
    let mut bad = timing.clone();
    bad.transport_complete = false;
    error(
        align(&source, &bad, &binding, &remote),
        AlignmentError::IncompleteSource,
    );
    let mut bad = timing.clone();
    bad.stream_id = Some(Uuid::new_v4());
    error(
        align(&source, &bad, &binding, &remote),
        AlignmentError::SourceIdentity,
    );
}
#[test]
fn clock_bounds_uncertainty_and_missing_observations_never_default_to_zero() {
    let (source, timing, binding, remote) = fixture(0);
    error(
        align(&source, &timing, &binding, &[]),
        AlignmentError::InsufficientWindows,
    );
    let mut bad = remote.clone();
    bad[0].frames[0].media_pts90k = u64::MAX;
    error(
        align(&source, &timing, &binding, &bad),
        AlignmentError::Clock,
    );
    let mut bad = remote.clone();
    bad[0].frames[0].uncertainty_ticks = 0;
    error(
        align(&source, &timing, &binding, &bad),
        AlignmentError::Uncertainty,
    );
    let mut bad = remote.clone();
    bad[0].frames[0].uncertainty_ticks = MAX_REMOTE_UNCERTAINTY + 1;
    error(
        align(&source, &timing, &binding, &bad),
        AlignmentError::Uncertainty,
    );
    let mut bad = remote.clone();
    bad[0].frames[1].media_pts90k = bad[0].frames[0].media_pts90k;
    error(
        align(&source, &timing, &binding, &bad),
        AlignmentError::Clock,
    );
    let mut bad = remote.clone();
    for f in &mut bad[1].frames {
        f.feature = feature(f.media_pts90k, 777);
    }
    error(
        align(&source, &timing, &binding, &bad),
        AlignmentError::NoMatch,
    );
}
#[test]
fn compositor_uncertainty_is_retained_and_excessive_measurement_is_refused() {
    let (source, timing, binding, mut remote) = fixture(10 * HZ as i64);
    for window in &mut remote {
        for frame in &mut window.frames {
            frame.uncertainty_ticks = 6_000;
        }
    }
    let result = align(&source, &timing, &binding, &remote).unwrap();
    assert!(result.offset.upper - result.offset.lower >= 21_000);
    for frame in &mut remote[1].frames {
        frame.uncertainty_ticks = 22_500;
    }
    error(
        align(&source, &timing, &binding, &remote),
        AlignmentError::Uncertainty,
    );
}
