use super::*;
const CHANNEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
#[test]
fn sealed_ts_segments_accumulate_exact_counts_ticks_and_reject_duplicate_range() {
    use encoded::fmp4::{
        fixtures,
        relay::{ts::TsMuxer, EncodedSampleReader},
    };
    // Structural parser fixtures only; no claim of codec decode equivalence.
    fn fragment(track: u32, count: u32, duration: u32) -> Vec<u8> {
        let mut bytes = fixtures::fragment(track, 0, count, true);
        let trun = bytes.windows(4).position(|s| s == b"trun").unwrap();
        let payload = bytes.windows(4).position(|s| s == b"mdat").unwrap() + 4;
        for i in 0..count as usize {
            let start = trun + 16 + i * 16;
            bytes[start..start + 4].copy_from_slice(&duration.to_be_bytes());
            if track == 0 && i % 60 == 0 {
                bytes[start + 8..start + 12].copy_from_slice(&0x02000000u32.to_be_bytes());
                bytes[payload + i * 6 + 4] = 0x65;
            }
        }
        bytes
    }
    let mut reader = EncodedSampleReader::new(fixtures::inputs()).unwrap();
    let mut mux = TsMuxer::new(reader.descriptor()).unwrap();
    assert!(mux
        .push(reader.push(0, &fragment(0, 120, 3000)).unwrap())
        .unwrap()
        .is_empty());
    let mut segments = mux
        .push(reader.push(1, &fragment(1, 188, 1024)).unwrap())
        .unwrap();
    let (tail, end) = mux.finish(reader.finish().unwrap()).unwrap();
    segments.extend(tail);
    assert_eq!(segments.len() as u64, end.segments());
    assert!(segments.len() >= 2);
    let mut report = Inspection::default();
    for segment in &segments {
        segment_facts(&mut report, segment).unwrap();
    }
    assert_eq!((report.video_samples, report.audio_samples), (120, 188));
    assert_eq!(
        (report.video_end, report.audio_end),
        (Some(360000), Some(192512))
    );
    assert_eq!(
        (report.video_timescale, report.audio_timescale),
        (Some(90000), Some(48000))
    );
    let before = serde_json::to_value(&report).unwrap();
    assert!(segment_facts(&mut report, &segments[0]).is_err());
    assert_eq!(before, serde_json::to_value(report).unwrap());
}
#[test]
fn retained_owner_is_reachable_after_pane_detach_until_explicit_join_release() {
    let (_root, host) = ready();
    let label = format!("chzzk-mado-{}", Uuid::new_v4().simple());
    let child = host.pane_controller(&label, CHANNEL).unwrap();
    let weak = Arc::downgrade(&child.inner);
    let run = Uuid::new_v4();
    lock(retained()).insert(run, child.inner.clone());
    child.inner.detached.store(true, Ordering::Release);
    drop(child);
    let reached = relay_hosts(&host).iter().any(|inner| inner.label == label);
    let retained_before = weak.upgrade().is_some();
    lock(retained()).remove(&run);
    let released_after = weak.upgrade().is_none();
    assert!(reached && retained_before && released_after);
}
fn ready() -> (tempfile::TempDir, OfficialBrowser) {
    let root = tempfile::tempdir().unwrap();
    let host = OfficialBrowser::new(root.path().to_owned()).unwrap();
    {
        let mut v = lock(&host.inner.view);
        v.open = true;
        v.ready = true;
        v.channel = Some(CHANNEL.into());
        v.page_generation = 3;
        v.auth_generation = 2;
    }
    (root, host)
}
#[test]
fn selection_only_existing_context_and_no_recording() {
    let (root, h) = ready();
    let before = h.inner.contexts.hosts().len();
    let s = h.select_relay_source_by_channel(CHANNEL).unwrap();
    assert!(s.current());
    assert_eq!(s.broadcast(), None);
    assert_eq!(s.epoch(), 3);
    assert_eq!(s.account_revision(), 2);
    assert_eq!(h.inner.contexts.hosts().len(), before);
    assert!(h.active_ids().is_empty());
    assert!(!root.path().join("streaming/relay-sidecars").exists());
    assert!(h.select_relay_source_by_channel(&"b".repeat(32)).is_err());
}
#[test]
fn selected_capability_rejects_epoch_account_and_channel_changes() {
    let (_r, h) = ready();
    let s = h.select_relay_source_by_channel(CHANNEL).unwrap();
    lock(&h.inner.view).page_generation += 1;
    assert!(!s.current());
    lock(&h.inner.view).page_generation = 3;
    lock(&h.inner.view).auth_generation += 1;
    assert!(!s.current());
    lock(&h.inner.view).auth_generation = 2;
    lock(&h.inner.view).channel = Some("b".repeat(32));
    assert!(!s.current());
}
#[test]
fn ambiguous_existing_channel_is_not_silently_selected() {
    let (_r, h) = ready();
    let label = format!("chzzk-mado-{}", Uuid::new_v4().simple());
    let child = h.pane_controller(&label, CHANNEL).unwrap();
    {
        let mut v = lock(&child.inner.view);
        v.ready = true;
        v.page_generation = 1;
    }
    assert!(h.select_relay_source_by_channel(CHANNEL).is_err());
    assert!(h.relay_available_channels().is_empty());
}
#[test]
fn closed_or_unready_source_never_becomes_capability() {
    let (_r, h) = ready();
    lock(&h.inner.view).ready = false;
    assert!(h.relay_available_channels().is_empty());
    assert!(h.select_relay_source_by_channel(CHANNEL).is_err());
    lock(&h.inner.view).ready = true;
    h.inner.detached.store(true, Ordering::Release);
    assert!(h.select_relay_source_by_channel(CHANNEL).is_err());
}
#[test]
fn init_bounds_and_duplicate_tracks_are_closed() {
    let t = || Track {
        track_index: 0,
        mime_type: "video/mp4;codecs=avc1.42e01e".into(),
        init: STANDARD.encode([1u8; 8]),
    };
    assert!(tracks(vec![t(), t()]).is_err());
    let mut v = t();
    v.init = STANDARD.encode(vec![0u8; 65537]);
    assert!(tracks(vec![v]).is_err());
    assert!(tracks(vec![]).is_err());
}
#[test]
fn clock_sample_is_source_bound_and_self_consistent() {
    let id = Uuid::new_v4();
    let mut c = Clock {
        clock: "mse_presentation_v1".into(),
        source_id: id.to_string(),
        source_time_seconds: 20.,
        current_time: 22.,
        timestamp_offset: 2.,
        track_index: 0,
        performance_now_ms: 100.,
        paused: None, seeking: None, ready_state: None, ended: None, playback_rate: None,
    };
    assert!(c.valid(id));
    assert!(!c.valid(Uuid::new_v4()));
    c.current_time = 23.;
    assert!(!c.valid(id));
    c.current_time = f64::NAN;
    assert!(!c.valid(id));
}
#[test]
fn clock_playback_state_preserves_legacy_and_rejects_partial_or_impossible_samples() {
    let id = Uuid::new_v4();
    let base = json!({"clock":"mse_presentation_v1","sourceId":id,"sourceTimeSeconds":20.,
        "currentTime":22.,"timestampOffset":2.,"trackIndex":0,"performanceNowMs":100.});
    assert!(serde_json::from_value::<Clock>(base.clone()).unwrap().valid(id));
    let mut partial = base.clone(); partial["paused"] = json!(false);
    assert!(!serde_json::from_value::<Clock>(partial).unwrap().valid(id));
    let mut complete = base;
    for (key,value) in [("paused",json!(false)),("seeking",json!(false)),("readyState",json!(4)),
        ("ended",json!(false)),("playbackRate",json!(1.))] { complete[key] = value; }
    for (key,value) in [("paused",json!(true)),("seeking",json!(true)),("readyState",json!(1)),("ended",json!(true))] {
        let mut observed = complete.clone(); observed[key] = value;
        // Non-playing is a valid observation, not a transport error.
        assert!(serde_json::from_value::<Clock>(observed).unwrap().valid(id));
    }
    for (key,value) in [("readyState",json!(5)),("playbackRate",json!(0.)),("playbackRate",json!(17.))] {
        let mut invalid = complete.clone(); invalid[key] = value;
        assert!(!serde_json::from_value::<Clock>(invalid).unwrap().valid(id));
    }
    let decoded = serde_json::from_value::<Clock>(complete).unwrap();
    let persisted = serde_json::to_value(&decoded).unwrap();
    assert_eq!(persisted["paused"],false); assert_eq!(persisted["playbackRate"],1.);
    assert!(!decoded.valid(Uuid::new_v4()));
}
#[test]
fn stop_deadline_before_begin_is_not_25_second_arm_wait() {
    let mut a = Admission::new();
    a.deadline = Some(Instant::now());
    a.expire();
    assert!(a.stopped && a.interrupted && a.cancel.load(Ordering::Acquire));
    a.stop(false);
    assert!(a.interrupted);
}
#[test]
fn strict_wire_never_accepts_target_or_arbitrary_clock_fields() {
    let id = Uuid::new_v4();
    let base =
        json!({"runId":id,"epoch":1,"sourceId":id,"packet":{"type":"finish","interrupted":false}});
    assert!(serde_json::from_value::<Wire>(base.clone()).is_ok());
    let mut injected = base;
    injected["destination"] = "https://not-authority".into();
    assert!(serde_json::from_value::<Wire>(injected).is_err());
    assert!(serde_json::from_value::<Clock>(json!({"url":"private"})).is_err());
}
