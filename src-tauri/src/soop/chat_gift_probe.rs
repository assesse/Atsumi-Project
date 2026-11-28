//! Explicit ignored, bounded, anonymous observation. Never persists wire data.
use super::*;
use std::{cell::RefCell, collections::BTreeMap};

const FRAMES: u64 = 20_000;
const BYTES: u64 = 8 * 1024 * 1024;
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Schema {
    frames: u64,
    min_fields: usize,
    max_fields: usize,
    valid_gifts: u64,
    invalid_gifts: u64,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    frames: u64,
    bytes: u64,
    limit_reached: bool,
    services: BTreeMap<u16, u64>,
    public_rich_schemas: BTreeMap<u16, Schema>,
}
thread_local! { static OBSERVATION: RefCell<Option<Observation>> = const { RefCell::new(None) }; }

pub(super) fn observe(frame: &Frame<'_>, bytes: usize) -> bool {
    OBSERVATION.with(|state| {
        let mut state = state.borrow_mut();
        let Some(state) = state.as_mut() else { return true; };
        if state.frames >= FRAMES || state.bytes.saturating_add(bytes as u64) > BYTES {
            state.limit_reached = true;
            return false;
        }
        state.frames += 1;
        state.bytes += bytes as u64;
        *state.services.entry(frame.service).or_default() += 1;
        if matches!(frame.service, 18 | 33 | 105) || gifts::known_unsupported(frame.service) {
            let fields: Vec<_> = frame.body.strip_prefix('\x0c')
                .and_then(|body| body.strip_suffix('\x0c'))
                .map(|body| body.split('\x0c').collect()).unwrap_or_default();
            let schema = state.public_rich_schemas.entry(frame.service).or_default();
            if schema.frames == 0 { schema.min_fields = fields.len(); }
            schema.frames += 1;
            schema.min_fields = schema.min_fields.min(fields.len());
            schema.max_fields = schema.max_fields.max(fields.len());
            if matches!(frame.service, 18 | 33 | 105) {
                if gifts::parse(frame.service, &fields).is_ok_and(|gift| gift.is_some()) {
                    schema.valid_gifts += 1;
                } else { schema.invalid_gifts += 1; }
            }
        }
        true
    })
}

#[test]
fn observer_only_counts_schema_and_stops_at_its_bounds() {
    OBSERVATION.with(|state| *state.borrow_mut() = Some(Observation::default()));
    let bytes = b"\x0cprivate-body\x0c";
    let frame = Frame { service: 9, result: 0, id: Some("not-persisted"), body: std::str::from_utf8(bytes).unwrap() };
    assert!(observe(&frame, 50));
    OBSERVATION.with(|state| {
        let mut state = state.borrow_mut();
        let observed = state.as_mut().unwrap();
        assert!(observed.public_rich_schemas.is_empty());
        let encoded = serde_json::to_string(observed).unwrap();
        assert!(!encoded.contains("private-body")); assert!(!encoded.contains("not-persisted"));
        observed.frames = FRAMES;
    });
    assert!(!observe(&frame, 50));
    OBSERVATION.with(|state| *state.borrow_mut() = Some(Observation { bytes: BYTES - 1, ..Observation::default() }));
    assert!(!observe(&frame, 2));
    OBSERVATION.with(|state| *state.borrow_mut() = None);
}

#[test]
#[ignore = "explicit public anonymous SOOP gift probe; 120s/20k parsed frames/8MiB observed frame bytes; schema and sanitized gifts only"]
fn public_guest_gift_probe() {
    let channel = std::env::var("ATSUMI_SOOP_GIFT_TEST_CHANNEL").expect("explicit public channel required");
    let broadcast = std::env::var("ATSUMI_SOOP_GIFT_TEST_BROADCAST").expect("explicit broadcast required");
    assert!(super::super::valid_id(&channel) && valid_broadcast(&broadcast), "invalid public target");
    OBSERVATION.with(|state| *state.borrow_mut() = Some(Observation::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let deadline = stop.clone();
    let deadline_worker = thread::spawn(move || {
        let started = Instant::now();
        while !deadline.load(Ordering::Acquire) && started.elapsed() < Duration::from_secs(120) {
            thread::sleep(Duration::from_millis(50));
        }
        deadline.store(true, Ordering::Release);
    });
    let mut observed_gifts = Vec::new();
    let stop_after_gifts = stop.clone();
    let started = Instant::now();
    let result = capture_bound(&channel, Some(&broadcast), |event| {
        if let Some(gift) = gifts::from_raw(&serde_json::to_value(&event).map_err(|_| "probe serialization")?) {
            // No author, message text, account/frame ID, absolute timestamp or
            // raw filename. This is observed schema evidence, not a recording.
            observed_gifts.push(serde_json::json!({"elapsedMs":event.elapsed_ms,"soopGift":gift}));
            if observed_gifts.len() >= 3 { stop_after_gifts.store(true, Ordering::Release); }
        }
        Ok(())
    }, stop.clone());
    stop.store(true, Ordering::Release);
    deadline_worker.join().unwrap();
    let observation = OBSERVATION.with(|state| state.borrow_mut().take().unwrap());
    let summary = result.as_ref().ok().map(|summary| serde_json::to_value(summary).unwrap());
    let artifact = serde_json::json!({
        "schema":1,"status":if observed_gifts.is_empty(){"no-gift-observed"}else{"public-gift-observed"},
        "actualVideoSynchronizationVerified":false,"identitiesAndTextRemoved":true,
        "durationMs":started.elapsed().as_millis(),"maximumDurationSeconds":120,
        "maxParsedFrames":FRAMES,"maxObservedFrameBytes":BYTES,
        "broadcastPinned":true,"broadcastMatchVerified":result.as_ref().is_ok_and(|summary|summary.connections>0),
        "chatMessagesSent":0,"videoRequests":0,"observation":observation,
        "summary":summary,"receiverSucceeded":result.is_ok(),"gifts":observed_gifts,
    });
    let audit = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
        .join(".runtime/requirements-audit/20261117-cloud-goal-resume");
    assert!(audit.is_dir(), "existing audit directory required");
    let target = audit.join(format!("soop-gift-passive-{}.json", uuid::Uuid::new_v4()));
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&target).unwrap();
    std::io::Write::write_all(&mut output, &serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    println!("SOOP passive gift observation: gift_count={}, receiver_succeeded={}, frame_count={}, video_requests=0, messages_sent=0, artifact={}",
        observed_gifts.len(), result.is_ok(), observation.frames, target.display());
    // Empty observation is explicitly inconclusive, not an unsupported claim
    // that gift collection or video synchronization succeeded.
    assert!(result.is_ok(), "bounded public receiver did not complete successfully; sanitized artifact retained");
}
