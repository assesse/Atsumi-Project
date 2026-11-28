use super::*;
use super::super::{live_relay::SelectionPhase, live_store::SourceFacts, upload::{Request, Response, Transport}};
use crate::streaming::relay_status::{Inspection, NativeSidecarOwner, SidecarInspection};
use serde_json::json;
use std::{fs, path::Path};
const CHANNEL: &str = "0123456789abcdef0123456789abcdef";
struct NoNetwork;
impl Transport for NoNetwork {
    fn send(&self, _: Request) -> Result<Response, CloudError> { panic!("no network in sidecar receipt test") }
}
fn make_service(root: &Path) -> Arc<CloudService> {
    CloudService::with_transport(root.into(), Arc::new(|_, _| Err("no source".into())),
        Arc::new(|_| Err("no browser".into())), Arc::new(NoNetwork)).unwrap()
}
fn fixture(service: &CloudService, root: &Path) -> (String, LiveRelaySnapshot, std::path::PathBuf) {
    let id = uuid::Uuid::new_v4().to_string();
    let run = uuid::Uuid::new_v4().to_string();
    let session = uuid::Uuid::new_v4().to_string();
    let source_id = uuid::Uuid::new_v4().to_string();
    let directory = root.join(&run); fs::create_dir(&directory).unwrap();
    let directory = fs::canonicalize(directory).unwrap();
    let source = SourceFacts { provider: "chzzk".into(), channel_id: CHANNEL.into(), broadcast_key: None,
        session_id: session.clone(), epoch: 1, account_revision: 7 };
    let native = NativeSidecarOwner::Chzzk { run_id: run.clone(), session_id: session.clone(), source_id: source_id.clone(),
        document_epoch: 1, account_revision: 7, channel_id: CHANNEL.into(), started_at_ms: 1000 };
    let owner = json!({"version":1,"provider":"chzzk","runId":run,"sessionId":session,"sourceId":source_id,
        "documentEpoch":1,"accountRevision":7,"channelId":CHANNEL,"startedAtMs":1000,"clock":"native_receipt","remoteArchiveMapping":"unknown"});
    fs::write(directory.join("relay-owner.json"), serde_json::to_vec(&owner).unwrap()).unwrap();
    let summary = json!({"version":1,"binding":owner,"summary":{"sidecarId":run,"joined":false,
        "rawRows":1,"eventRows":1,"bytes":100,"interrupted":true,"assetsIncomplete":false,"failure":false,
        "textState":"incomplete","clockQuality":"native_receipt_and_separate_source_observations","timingRecords":0,"timingIncomplete":false},
        "assetExpected":0,"assetIncomplete":false,"remoteArchiveMapping":"unknown"});
    fs::write(directory.join("relay-summary.json"), serde_json::to_vec(&summary).unwrap()).unwrap();
    fs::write(directory.join("relay-timing.jsonl"), b"").unwrap();
    fs::write(directory.join("chat.events.jsonl"), b"{}\n").unwrap();
    fs::write(directory.join("viewer-metrics.jsonl"), b"").unwrap();
    let message = json!({"sequence":1,"sender":"fixture","text":"recorded appearance","serverTime":1000,
        "receivedAt":1000,"offsetSeconds":0.0,"rich":{"nicknameColor":"#56AA89","badges":[],"emojis":[]}});
    fs::write(directory.join("chat.jsonl"), format!("{}\n", message)).unwrap();
    let mut receipt = LiveReceipt::selected(id.clone(), source);
    receipt.view.state = "failed".into();
    receipt.view.run_id = Some(run.clone());
    receipt.view.manager_joined = true; receipt.view.worker_joined = true; receipt.view.sidecar_joined = true;
    receipt.sidecar_id = Some(run.clone());
    assert!(receipt.valid());
    let mut inner = service.lock().unwrap(); let mut candidate = inner.catalog.clone();
    candidate.live_receipts.push(receipt); service.publish(&mut inner, candidate).unwrap(); drop(inner);
    let observed = LiveRelaySnapshot { selection_id: id.clone(), provider: "chzzk".into(), channel_id: CHANNEL.into(),
        broadcast_key: None, state: SelectionPhase::Failed, run_id: Some(run.clone()), destination: None,
        stop_requested: true, interrupted: true, fault: None, first_relay_fault: None, manager_joined: true, archive_ready: false,
        relay: Some(Inspection { run_id: run.clone(), manager_joined: true, worker_joined: true, delivery: Some(Default::default()),
            sidecar: Some(SidecarInspection { sidecar_id: run, joined: true, ..Default::default() }),
            sidecar_directory: Some(directory.clone()), sidecar_owner: Some(native), ..Default::default() }) };
    (id, observed, directory)
}
#[test]
fn actual_join_receipt_reopens_original_provider_history_without_media_job() {
    let temp = tempfile::tempdir().unwrap(); let service = make_service(temp.path());
    let (id, observed, directory) = fixture(&service, temp.path());
    let original = fs::read(directory.join("chat.jsonl")).unwrap();
    service.finish_live_sidecar(&id, &observed).unwrap();
    let saved = service.vault.load().unwrap();
    let receipt = &saved.live_receipts[0];
    assert!(receipt.valid() && receipt.sidecar.as_ref().unwrap().sealed.is_some());
    assert!(!receipt.view.archive_ready && saved.jobs.is_empty());
    let lease = service.open_live_history(&id).unwrap();
    assert_eq!(lease.facts().remote_anchor, "unknown");
    assert!(lease.facts().history_incomplete);
    match lease.page(None, 20).unwrap() {
        LiveHistoryPage::Chzzk { items, cursor } => {
            assert_eq!(items.len(), 1); assert!(cursor.is_none());
            assert_eq!(items[0].rich.as_ref().unwrap().nickname_color.as_deref(), Some("#56AA89"));
        }, _ => panic!("original provider not retained"),
    }
    drop(lease);
    assert_eq!(fs::read(directory.join("chat.jsonl")).unwrap(), original);
    assert!(serde_json::to_vec(receipt).unwrap().len() <= 8192);
}
#[test]
fn quit_preserves_pending_native_authority_and_restart_finishes_same_root() {
    let temp = tempfile::tempdir().unwrap(); let first = make_service(temp.path());
    let (id, observed, directory) = fixture(&first, temp.path());
    first.request_live_shutdown().unwrap();
    assert!(first.finish_live_sidecar(&id, &observed).is_err());
    let saved = first.vault.load().unwrap();
    assert!(saved.live_receipts[0].sidecar.as_ref().unwrap().sealed.is_none());
    drop(first);
    let second = make_service(temp.path());
    let lease = second.open_live_history(&id).unwrap();
    assert_eq!(lease.capture.root(), directory);
    assert!(matches!(lease.page(None, 1).unwrap(), LiveHistoryPage::Chzzk { .. }));
    assert!(second.vault.load().unwrap().live_receipts[0].sidecar.as_ref().unwrap().sealed.is_some());
}
#[test]
fn foreign_native_source_or_epoch_never_gains_receipt_authority() {
    for changed in ["source", "epoch", "selection"] {
        let temp = tempfile::tempdir().unwrap(); let service = make_service(temp.path());
        let (id, mut observed, _) = fixture(&service, temp.path());
        match changed {
            "source" => if let Some(NativeSidecarOwner::Chzzk { source_id, .. }) = observed.relay.as_mut().unwrap().sidecar_owner.as_mut() { *source_id = uuid::Uuid::new_v4().to_string(); },
            "epoch" => if let Some(NativeSidecarOwner::Chzzk { document_epoch, .. }) = observed.relay.as_mut().unwrap().sidecar_owner.as_mut() { *document_epoch += 1; },
            _ => observed.selection_id = uuid::Uuid::new_v4().to_string(),
        }
        assert!(service.finish_live_sidecar(&id, &observed).is_err());
        assert!(service.vault.load().unwrap().live_receipts[0].sidecar.as_ref().is_none_or(|s| s.sealed.is_none()));
        assert!(service.open_live_history(&id).is_err());
    }
}
#[test]
fn cancelled_player_open_preserves_pending_receipt_and_other_reader() {
    let temp = tempfile::tempdir().unwrap(); let first = make_service(temp.path());
    let (id, observed, directory) = fixture(&first, temp.path());
    let original = fs::read(directory.join("chat.jsonl")).unwrap();
    first.request_live_shutdown().unwrap();
    assert!(first.finish_live_sidecar(&id, &observed).is_err());
    drop(first);
    let service = make_service(temp.path());
    let cancelled_open = Arc::new(AtomicBool::new(true));
    assert!(service.open_live_history_cancellable(&id, cancelled_open).is_err());
    assert!(service.vault.load().unwrap().live_receipts[0].sidecar.as_ref().unwrap().sealed.is_none());
    assert!(service.lock().unwrap().live.sidecar_work.is_empty());
    assert_eq!(fs::read(directory.join("chat.jsonl")).unwrap(), original);

    let player_cancel = Arc::new(AtomicBool::new(false));
    let player = service.open_live_history_cancellable(&id, player_cancel.clone()).unwrap();
    let independent = service.open_live_history(&id).unwrap();
    player_cancel.store(true, Ordering::Release);
    assert!(player.page(None, 1).is_err());
    assert!(matches!(independent.page(None, 1).unwrap(), LiveHistoryPage::Chzzk { .. }));
    drop(player);
    assert_eq!(service.lock().unwrap().live.sidecar_work.len(), 1);
    assert_eq!(fs::read(directory.join("chat.jsonl")).unwrap(), original);
}
#[test]
fn modified_history_is_rejected_and_held_handles_deny_replacement() {
    let temp = tempfile::tempdir().unwrap(); let service = make_service(temp.path());
    let (id, observed, directory) = fixture(&service, temp.path());
    service.finish_live_sidecar(&id, &observed).unwrap();
    let lease = service.open_live_history(&id).unwrap();
    #[cfg(windows)] assert!(fs::write(directory.join("chat.jsonl"), b"changed\n").is_err());
    drop(lease);
    fs::write(directory.join("chat.jsonl"), b"changed\n").unwrap();
    assert!(service.open_live_history(&id).is_err());
}
#[test]
fn failed_persistence_and_shutdown_do_not_promote_or_reclassify_relay() {
    let temp = tempfile::tempdir().unwrap(); let service = make_service(temp.path());
    let (id, observed, _) = fixture(&service, temp.path());
    service.vault.fail_once();
    assert!(service.finish_live_sidecar(&id, &observed).is_err());
    let saved = service.vault.load().unwrap();
    assert!(saved.live_receipts[0].sidecar.is_none());
    assert_eq!(saved.live_receipts[0].view.state, "failed");
    assert!(saved.live_receipts[0].view.manager_joined && saved.live_receipts[0].view.worker_joined);
    assert!(service.open_live_history(&id).is_err());
    let temp = tempfile::tempdir().unwrap(); let service = make_service(temp.path());
    let (id, observed, _) = fixture(&service, temp.path());
    service.finish_live_sidecar(&id, &observed).unwrap();
    let lease = service.open_live_history(&id).unwrap();
    service.request_live_shutdown().unwrap();
    assert!(lease.page(None, 1).is_err());
}
#[test]
fn idle_lease_does_not_block_exit_but_actual_inflight_read_must_drain() {
    let temp = tempfile::tempdir().unwrap(); let service = make_service(temp.path());
    let (id, observed, _) = fixture(&service, temp.path());
    service.finish_live_sidecar(&id, &observed).unwrap();
    let lease = service.open_live_history(&id).unwrap();
    assert!(service.active_ids().is_empty());
    let active = lease.work.begin().unwrap();
    assert!(!service.active_ids().is_empty());
    service.request_live_shutdown().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let closing = service.clone();
    let worker = std::thread::spawn(move || { let result = closing.wait_live_history(); tx.send(result.is_ok()).unwrap(); });
    assert!(rx.recv_timeout(std::time::Duration::from_millis(30)).is_err());
    drop(active);
    assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap());
    worker.join().unwrap();
    assert!(service.active_ids().is_empty());
    assert!(lease.page(None, 1).is_err());
    // The idle capability is still held here; only actual work gated the wait.
}
#[test]
fn abandoned_metadata_temp_and_partial_chat_tail_do_not_block_preserved_history() {
    let temp = tempfile::tempdir().unwrap(); let service = make_service(temp.path());
    let (id, observed, directory) = fixture(&service, temp.path());
    let stale = directory.join(format!(".relay-history-{}.pending", uuid::Uuid::new_v4()));
    fs::write(&stale, b"interrupted metadata write").unwrap();
    use std::io::Write;
    fs::OpenOptions::new().append(true).open(directory.join("chat.jsonl")).unwrap().write_all(b"{partial").unwrap();
    service.finish_live_sidecar(&id, &observed).unwrap();
    let original_seal = fs::read(directory.join("relay-history-seal.json")).unwrap();
    let lease = service.open_live_history(&id).unwrap();
    match lease.page(None, 120).unwrap() { LiveHistoryPage::Chzzk { items, cursor } => { assert_eq!(items.len(), 1); assert!(cursor.is_none()); }, _ => panic!() }
    drop(lease);
    assert_eq!(fs::read(directory.join("relay-history-seal.json")).unwrap(), original_seal);
    assert!(stale.exists()); // Never delete an unknown prior file during retry.
}
#[test]
fn unavailable_frame_measurement_does_not_break_history_or_claim_remote_sync() {
    let temp = tempfile::tempdir().unwrap(); let service = make_service(temp.path());
    let (id, mut observed, directory) = fixture(&service, temp.path());
    observed.relay.as_mut().unwrap().frame_reference = Some(crate::streaming::relay_status::FrameReferenceInspection::unavailable("tools_unavailable"));
    let pending = directory.join(format!(".relay-frame-reference-{}.pending", uuid::Uuid::new_v4()));
    fs::write(&pending, b"abandoned numeric reference").unwrap();
    service.finish_live_sidecar(&id, &observed).unwrap();
    let lease = service.open_live_history(&id).unwrap();
    let facts = lease.facts();
    assert!(lease.references().unwrap().is_none());
    assert_eq!(facts.source_reference, "unavailable");
    assert_eq!(facts.source_reference_reason.as_deref(), Some("tools_unavailable"));
    assert_eq!(facts.remote_anchor, "unknown");
    assert!(matches!(lease.page(None, 1).unwrap(), LiveHistoryPage::Chzzk { .. }));
    assert!(pending.exists());
}

/// Actual producer caller supplies its live native snapshot directly. There is
/// intentionally no JSON/native-owner reconstruction entry point.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActualHistoryReport {
    pub provider: String,
    pub files: usize,
    pub pages: usize,
    pub events: usize,
    pub assets: usize,
    pub assets_read: usize,
    pub reopened: bool,
    pub native_source: bool,
    pub archive_ready: bool,
    pub remote_anchor: &'static str,
    pub local_media_jobs: usize,
    pub source_reference: String,
    pub source_reference_reason: Option<String>,
    pub source_reference_verified: bool,
    pub source_reference_windows: usize,
    pub source_event_identity_matches: usize,
    pub source_positions: usize,
    pub source_unmapped: std::collections::BTreeMap<String, usize>,
    pub source_timing: serde_json::Value,
}
pub(crate) fn verify_actual_joined_history(temp_owned_root: &Path, source: SourceFacts, observed: &LiveRelaySnapshot) -> Result<ActualHistoryReport, CloudError> {
    // Caller must choose a fresh test-only vault directory; never reuse user data.
    fs::create_dir(temp_owned_root).map_err(|_| unavailable())?;
    let service = make_service(temp_owned_root);
    {
        let mut inner = service.lock()?;
        let mut candidate = inner.catalog.clone();
        candidate.live_receipts.push(LiveReceipt::selected(observed.selection_id.clone(), source));
        service.publish(&mut inner, candidate)?;
    }
    service.live_observe(&observed.selection_id, observed, true)?;
    service.finish_live_sidecar(&observed.selection_id, observed)?;
    let lease = service.open_live_history(&observed.selection_id)?;
    let mut cursor = None;
    let mut pages = 0;
    let mut events = 0;
    let mut source_event_identity_matches = 0;
    let mut source_positions = 0;
    let mut source_unmapped = std::collections::BTreeMap::<String, usize>::new();
    let indexing_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    for _ in 0..1024 {
        pages += 1;
        match lease.page(cursor.as_deref(), 120)? {
            LiveHistoryPage::Chzzk { items, cursor: next } => { events += items.len(); cursor = next; },
            LiveHistoryPage::Soop(page) => {
                if page.state == "indexing" {
                    if std::time::Instant::now() >= indexing_deadline { return Err(unavailable()); }
                    std::thread::sleep(std::time::Duration::from_millis(50)); continue;
                }
                let positions = lease.soop_source_chat_page(cursor.as_deref(), 120)?;
                if positions.remote_anchor != "unknown" || positions.positions.len() != page.items.len()
                    || !positions.page.items.iter().zip(&page.items).all(|(a,b)|a.id==b.id) { return Err(unavailable()); }
                for item in positions.positions {
                    if let Some(point) = item.position {
                        if point.numeric_uncertainty_ticks90k != 2 || point.presentation_uncertainty != "unknown"
                            || point.segment_sha256.len() != 64 { return Err(unavailable()); }
                        source_event_identity_matches += 1; source_positions += 1;
                    } else {
                        let reason = item.unmapped_reason.ok_or_else(unavailable)?;
                        // These outcomes prove one exact UUID match but do NOT prove usable timing.
                        if matches!(reason,"inactive_or_hidden_source_clock"|"outside_acknowledged_envelope") {
                            source_event_identity_matches += 1;
                        }
                        *source_unmapped.entry(reason.into()).or_default() += 1;
                    }
                }
                events += page.items.len(); cursor = page.cursor;
            },
        }
        if cursor.is_none() { break; }
    }
    if cursor.is_some() { return Err(unavailable()); }
    let assets = lease.history.asset_ids(&lease.provider);
    let mut assets_read = 0;
    for id in &assets { if lease.asset(id)?.is_some() { assets_read += 1; } }
    let facts = lease.facts();
    let timing = lease.timing().summary();
    let source_timing = json!({"records":timing.records,"segmentAcks":timing.acknowledged_segments,
        "prefixSegments":timing.prefix_segments,"endSeen":timing.end_seen,"transportComplete":timing.transport_complete,
        "timingIncomplete":timing.timing_incomplete,"observations":timing.observations,
        "usableObservations":timing.usable_observations,"unknownObservations":timing.unknown_observations,
        "remoteAnchor":"unknown","presentationUncertainty":"unknown"});
    let references = lease.references()?;
    let source_reference_verified = references.is_some();
    let source_reference_windows = references.map_or(0, |r| r.references().windows.len());
    drop(lease);
    let stored = service.vault.load()?;
    let archive_ready = stored.live_receipts[0].view.archive_ready;
    let local_media_jobs = stored.jobs.len();
    drop(service);
    let reopened = make_service(temp_owned_root);
    let lease = reopened.open_live_history(&observed.selection_id)?;
    let _page = lease.page(None, 1)?;
    let _references = lease.references()?;
    Ok(ActualHistoryReport { provider: facts.provider, files: facts.files, pages, events, assets: assets.len(), assets_read,
        reopened: true, native_source: true, archive_ready, remote_anchor: facts.remote_anchor, local_media_jobs,
        source_reference: facts.source_reference, source_reference_reason: facts.source_reference_reason,
        source_reference_verified, source_reference_windows, source_event_identity_matches, source_positions, source_unmapped, source_timing })
}

fn reference_fixture(service: &CloudService, root: &Path) -> (String, LiveRelaySnapshot, std::path::PathBuf, super::super::live_fingerprint::ReferenceSet) {
    use super::super::live_fingerprint::{FrameFeature, FrameReference, ReferenceSet, ReferenceWindow};
    let (id, mut observed, directory) = fixture(service, root);
    let relay = observed.relay.as_mut().unwrap();
    let Some(NativeSidecarOwner::Chzzk { run_id, source_id, .. }) = &relay.sidecar_owner else { panic!() };
    let stream = uuid::Uuid::new_v4();
    let reference = ReferenceSet { version: 1, run_id: uuid::Uuid::parse_str(run_id).unwrap(),
        source_id: uuid::Uuid::parse_str(source_id).unwrap(), stream_id: Some(stream),
        windows: (0..3).map(|n| ReferenceWindow { ordinal: n, segment_sha256: format!("{:064x}", n + 1),
            source_origin_ticks: 48000, source_origin_timescale: 48000, source_video_timescale: 90000,
            frames: (0..12).map(|i| FrameReference { pts90k: n * 180000 + i * 7500,
                feature: FrameFeature { gradient: [(n * 100 + i) as u32, 1, 2, 3], mean: 100, spread: 20 } }).collect() }).collect(),
        acknowledged_segments: 3, sampled_segments: 3, queue_dropped: 0, decode_failures: 0, static_windows: 0,
        cancelled: false, incomplete: false, remote_anchor: "unknown".into() };
    let mut rows = Vec::new();
    for n in 0..3u64 {
        let v = 90000 + n * 180000; let a = 48000 + n * 96000;
        rows.push(json!({"version":1,"kind":"segment_ack","streamId":stream,"ordinal":n,
            "sha256":format!("{:064x}",n+1),"nativeBeforeNs":100+n*100,"nativeAfterNs":110+n*100,
            "source":{"originTicks":48000,"originTimescale":48000,"segmentStart90k":n*180000,"segmentEnd90k":(n+1)*180000,
                "video":{"timescale":90000,"firstDts":v,"lastDtsEnd":v+180000,"minPts":v,"maxPtsEnd":v+180000,"samples":60,
                    "firstDts90k":n*180000,"lastDtsEnd90k":(n+1)*180000,"minPts90k":n*180000,"maxPtsEnd90k":(n+1)*180000},
                "audio":{"timescale":48000,"firstDts":a,"lastDtsEnd":a+96000,"minPts":a,"maxPtsEnd":a+96000,"samples":94,
                    "firstDts90k":n*180000,"lastDtsEnd90k":(n+1)*180000,"minPts90k":n*180000,"maxPtsEnd90k":(n+1)*180000}}}));
    }
    rows.push(json!({"version":1,"kind":"end_ack","streamId":stream,"segments":3,"nativeBeforeNs":1000,"nativeAfterNs":1010,
        "origin":{"ticks":48000,"timescale":48000,"videoBase":90000,"audioBase":48000,"videoRemainder":0,"audioRemainder":0},
        "preroll":{"videoSamples":0,"audioSamples":0,"bytes":0}}));
    fs::write(directory.join("relay-timing.jsonl"), rows.iter().map(|r|format!("{r}\n")).collect::<String>()).unwrap();
    let path = directory.join("relay-summary.json");
    let mut summary: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    summary["summary"]["timingRecords"] = json!(4);
    fs::write(path, serde_json::to_vec(&summary).unwrap()).unwrap();
    fs::write(directory.join(super::super::live_fingerprint::FILE_NAME), serde_json::to_vec(&reference).unwrap()).unwrap();
    relay.frame_reference = Some(crate::streaming::relay_status::FrameReferenceInspection::settled(&reference));
    let delivery = relay.delivery.as_mut().unwrap(); delivery.segment_acks = 3; delivery.end_list_acked = true;
    let mut inner = service.lock().unwrap(); let mut candidate = inner.catalog.clone();
    let receipt = candidate.live_receipts.iter_mut().find(|r| r.view.selection_id == id).unwrap();
    receipt.view.segment_acks = 3; receipt.view.end_list_acked = true;
    service.publish(&mut inner, candidate).unwrap();
    (id, observed, directory, reference)
}
#[test]
fn native_reference_reader_reuses_borrowed_admission_and_reopens_after_restart() {
    let temp = tempfile::tempdir().unwrap(); let service = make_service(temp.path());
    let (id, observed, directory, source) = reference_fixture(&service, temp.path());
    let original = fs::read(directory.join("chat.jsonl")).unwrap();
    service.finish_live_sidecar(&id, &observed).unwrap();
    let lease = service.open_live_history(&id).unwrap();
    let first = lease.references().unwrap().unwrap();
    assert_eq!(first.references().run_id, source.run_id);
    assert_eq!(first.references().source_id, source.source_id);
    assert_eq!(first.references().windows.len(), 3);
    assert_eq!(first.timing().stream_id, source.stream_id);
    assert_eq!(first.timing().remote_anchor, timing::RemoteAnchor::Unknown);
    assert!(std::ptr::eq(first, lease.references().unwrap().unwrap()));
    service.request_live_shutdown().unwrap();
    assert!(lease.references().is_err());
    drop(lease); drop(service);
    let reloaded = make_service(temp.path()); let lease = reloaded.open_live_history(&id).unwrap();
    assert_eq!(lease.references().unwrap().unwrap().references().windows.len(), 3);
    assert_eq!(fs::read(directory.join("chat.jsonl")).unwrap(), original);
    assert!(!reloaded.vault.load().unwrap().live_receipts[0].view.archive_ready);
    assert!(reloaded.vault.load().unwrap().jobs.is_empty());
}
#[test]
fn direct_source_chat_projection_preserves_rich_rows_and_rejects_foreign_unknown_or_outside_times() {
    let temp=tempfile::tempdir().unwrap();let service=make_service(temp.path());
    let (id,observed,directory,reference)=reference_fixture(&service,temp.path());
    let mut first:serde_json::Value=serde_json::from_slice(&fs::read(directory.join("chat.jsonl")).unwrap()).unwrap();
    first["offsetSeconds"]=json!(999.);
    first["replayClock"]=json!({"version":1,"receivedAtMs":1000,"observedMonotonicMs":20.,"sourceGeneration":3,
        "clock":"mse_presentation_v1","mediaTimeSeconds":2.,"sourceTimeSeconds":2.,"sourceId":reference.source_id,"playbackRate":1.});
    let mut rows=vec![first];
    let mut foreign=rows[0].clone();foreign["sequence"]=json!(2);foreign["replayClock"]["sourceId"]=json!(uuid::Uuid::new_v4());rows.push(foreign);
    let mut unknown=rows[0].clone();unknown["sequence"]=json!(3);unknown.as_object_mut().unwrap().remove("replayClock");unknown["offsetSeconds"]=json!(1.);rows.push(unknown);
    let mut outside=rows[0].clone();outside["sequence"]=json!(4);outside["replayClock"]["mediaTimeSeconds"]=json!(99.);outside["replayClock"]["sourceTimeSeconds"]=json!(99.);rows.push(outside);
    let mut boundary=rows[0].clone();boundary["sequence"]=json!(5);boundary["replayClock"]["mediaTimeSeconds"]=json!(3.);boundary["replayClock"]["sourceTimeSeconds"]=json!(3.);rows.push(boundary);
    let original=rows.iter().map(|r|format!("{r}\n")).collect::<String>();
    fs::write(directory.join("chat.jsonl"),&original).unwrap();
    service.finish_live_sidecar(&id,&observed).unwrap();
    let lease=service.open_live_history(&id).unwrap();
    let page=lease.source_chat_page(None,120).unwrap();assert_eq!(page.items.len(),5);assert!(page.cursor.is_none());
    let position=page.items[0].position.as_ref().unwrap();
    assert_eq!(position.source_ticks90k,180000);assert_eq!(position.transport_ticks90k,90000);
    assert_eq!(position.source_generation,3);assert_eq!(position.segment_ordinal,0);
    assert_eq!(position.segment_sha256,format!("{:064x}",1));
    assert_eq!(position.numeric_uncertainty_ticks90k,2);assert_eq!(position.presentation_uncertainty,"unknown");
    assert_eq!(page.items[0].message.offset_seconds,999.);
    assert_eq!(page.items[0].message.rich.as_ref().unwrap().nickname_color.as_deref(),Some("#56AA89"));
    assert_eq!(page.items[0].message.replay_clock.as_ref().unwrap().source_time_seconds,Some(2.));
    assert_eq!(page.items[1].unmapped_reason,Some("foreign_source_clock"));
    assert_eq!(page.items[2].unmapped_reason,Some("missing_source_clock"));
    for index in [3,4] {assert_eq!(page.items[index].unmapped_reason,Some("outside_acknowledged_envelope"));}
    for item in &page.items[1..] {assert!(item.position.is_none());}
    assert_eq!(page.remote_anchor,"unknown");assert!(!lease.facts().transport_complete || !service.vault.load().unwrap().live_receipts[0].view.archive_ready);
    assert!(lease.source_chat_page(Some("1"),120).is_err());
    let one=lease.source_chat_page(None,1).unwrap();assert_eq!(one.items.len(),1);assert!(one.cursor.is_some());
    let next=lease.source_chat_page(one.cursor.as_deref(),1).unwrap();assert_eq!(next.items[0].message.sequence,2);
    drop(lease);assert_eq!(fs::read_to_string(directory.join("chat.jsonl")).unwrap(),original);
}
#[test]
fn direct_source_chat_keeps_legacy_rows_and_stops_after_shutdown() {
    let temp=tempfile::tempdir().unwrap();let service=make_service(temp.path());
    let (id,observed,_directory)=fixture(&service,temp.path());service.finish_live_sidecar(&id,&observed).unwrap();
    let lease=service.open_live_history(&id).unwrap();let page=lease.source_chat_page(None,120).unwrap();
    assert_eq!(page.items.len(),1);assert!(page.items[0].position.is_none());
    assert_eq!(page.items[0].unmapped_reason,Some("missing_source_clock"));
    service.request_live_shutdown().unwrap();assert!(lease.source_chat_page(None,120).is_err());
}
#[test]
fn sealed_json_does_not_mint_native_reference_ownership_or_ack_proof() {
    for fault in ["run", "source", "stream", "hash", "ordinal", "origin", "pts", "count", "readiness"] {
        let temp = tempfile::tempdir().unwrap(); let service = make_service(temp.path());
        let (id, observed, directory, mut reference) = reference_fixture(&service, temp.path());
        match fault {
            "run" => reference.run_id = uuid::Uuid::new_v4(),
            "source" => reference.source_id = uuid::Uuid::new_v4(),
            "stream" => reference.stream_id = Some(uuid::Uuid::new_v4()),
            "hash" => reference.windows[1].segment_sha256 = "ff".repeat(32),
            "ordinal" => reference.windows[1].ordinal = 2,
            "origin" => reference.windows[1].source_origin_ticks += 1,
            "pts" => reference.windows[1].frames[11].pts90k = 360000,
            "count" => reference.acknowledged_segments += 1,
            _ => reference.incomplete = true,
        }
        fs::write(directory.join(super::super::live_fingerprint::FILE_NAME), serde_json::to_vec(&reference).unwrap()).unwrap();
        assert!(service.finish_live_sidecar(&id, &observed).is_err(), "{fault}");
        let lease = service.open_live_history(&id).unwrap();
        assert!(lease.references().is_err(), "{fault}");
        // Invalid alignment evidence does not destroy manual original history.
        assert!(matches!(lease.page(None, 1).unwrap(), LiveHistoryPage::Chzzk { .. }));
    }
}
