use super::*;
use std::sync::{atomic::{AtomicBool, AtomicUsize, Ordering}, mpsc};
use std::time::Duration;

fn account() -> NativeDestinationOwner {
    NativeDestinationOwner { channel_id: "UCabcdefghijklmnopqrstuv".into(), generation: 1 }
}
fn destination() -> PreparedLiveDestination {
    PreparedLiveDestination::from_native_response(account(), "abcdefghijk".into(), "stream-one".into(),
        "unlisted", "https://a.upload.youtube.com/http_upload_hls?cid=controlled-secret&copy=0&file=").unwrap()
}
fn coordinator() -> LiveRelayCoordinator { LiveRelayCoordinator::select(SoopRelaySource::test_fixture()) }

#[derive(Default)]
struct FakeRun {
    report: Mutex<SoopRelayInspection>,
    current: AtomicBool,
    stops: Mutex<Vec<bool>>,
}
impl FakeRun {
    fn running(run: Uuid) -> Arc<Self> {
        Arc::new(Self { report: Mutex::new(SoopRelayInspection { run_id: run.to_string(), ..Default::default() }),
            current: AtomicBool::new(true), ..Default::default() })
    }
    fn joined(&self) {
        let mut report = lock(&self.report);
        report.manager_joined = true;
        report.worker_joined = true;
        report.input_ended = true;
        report.delivery.get_or_insert_with(Default::default).end_list_acked = true;
        let sidecar = report.sidecar.get_or_insert_with(Default::default);
        sidecar.joined = true;
        sidecar.text_state = "complete".into();
    }
}
impl RelayControl for FakeRun {
    fn observe(&self) -> Observation {
        Observation { current_source: self.current.load(Ordering::Acquire), relay: lock(&self.report).clone() }
    }
    fn stop(&self, interrupted: bool) { lock(&self.stops).push(interrupted); }
}

#[test]
fn prepared_destination_enforces_unlisted_owner_and_secret_transport_shape() {
    let target = "https://a.upload.youtube.com/http_upload_hls?cid=secret&copy=0&file=";
    for privacy in ["public", "private", "", "UNLISTED"] {
        assert!(PreparedLiveDestination::from_native_response(account(), "abcdefghijk".into(), "stream".into(), privacy, target).is_err());
    }
    let mut invalid = account(); invalid.generation = 0;
    assert!(PreparedLiveDestination::from_native_response(invalid, "abcdefghijk".into(), "stream".into(), "unlisted", target).is_err());
    for bad in ["http://a.upload.youtube.com/http_upload_hls?cid=secret&copy=0&file=", "https://other.invalid/path", "https://a.upload.youtube.com/http_upload_hls?cid=secret&copy=1&file="] {
        assert!(PreparedLiveDestination::from_native_response(account(), "abcdefghijk".into(), "stream".into(), "unlisted", bad).is_err());
    }
}

#[test]
fn owner_replacement_rejects_start_without_losing_created_remote_ids() {
    let relay = coordinator();
    let mut current = account(); current.generation += 1;
    let result = relay.start_with(&current, destination(), |_, _| panic!("stale owner must not start"));
    assert!(matches!(result, Err(RelaySelectionError::OwnerChanged)));
    let view = relay.status();
    assert_eq!(view.state, SelectionPhase::Failed);
    assert_eq!(view.destination.unwrap().broadcast_id, "abcdefghijk");
    assert!(!view.manager_joined && !view.archive_ready);
}

#[test]
fn failed_source_setup_keeps_destination_receipt_and_does_not_claim_join() {
    let relay = coordinator();
    assert!(matches!(relay.start_with(&account(), destination(), |_, _| Err(RelaySelectionError::StartRejected)), Err(RelaySelectionError::StartRejected)));
    let view = relay.status();
    assert_eq!(view.fault, Some(RelaySelectionError::StartRejected));
    assert!(view.destination.is_some() && view.run_id.is_some());
    assert!(!view.manager_joined);
    assert!(lock(&relay.state).control.is_none());
}

#[test]
fn failed_run_intent_persistence_never_installs_source_manager() {
    let relay = coordinator();
    let result = relay.start_with_checkpoint(&account(), destination(), |run| {
        assert!(!run.is_nil());
        assert!(relay.state.try_lock().is_ok(), "durable callback must run outside coordinator lock");
        Err(RelaySelectionError::Persistence)
    }, |_, _| panic!("manager installed before durable intent"));
    assert!(matches!(result, Err(RelaySelectionError::Persistence)));
    let view = relay.status();
    assert_eq!(view.fault, Some(RelaySelectionError::Persistence));
    assert!(view.destination.is_some() && view.run_id.is_some());
    assert!(!view.manager_joined && lock(&relay.state).control.is_none());
}

#[test]
fn stop_during_durable_checkpoint_still_retains_installed_owner_until_join() {
    let relay = coordinator();
    let mut installed = None;
    relay.start_with_checkpoint(&account(), destination(), |_| {
        relay.stop(false);
        Ok(())
    }, |run, _| {
        let control = FakeRun::running(run); installed = Some(control.clone()); Ok(control)
    }).unwrap();
    let control = installed.unwrap();
    assert_eq!(*lock(&control.stops), vec![false]);
    assert!(!relay.status().manager_joined);
    control.joined(); assert!(relay.status().manager_joined);
}

#[test]
fn duplicate_start_does_not_replace_selected_run_or_destination() {
    let relay = coordinator();
    relay.start_with(&account(), destination(), |run, _| Ok(FakeRun::running(run))).unwrap();
    let original = relay.status().run_id;
    assert!(matches!(relay.start_with(&account(), destination(), |_, _| panic!("duplicate")), Err(RelaySelectionError::AlreadyStarted)));
    assert_eq!(relay.status().run_id, original);
}

#[test]
fn stop_and_owner_replacement_during_start_keep_installed_owner_until_join() {
    let relay = Arc::new(coordinator());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (run_tx, run_rx) = mpsc::channel();
    let pending = relay.clone();
    let thread = std::thread::spawn(move || pending.start_with(&account(), destination(), |run, _| {
        let control = FakeRun::running(run);
        run_tx.send(control.clone()).unwrap();
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        Ok(control)
    }));
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let stopping = relay.stop(false);
    assert_eq!(stopping.state, SelectionPhase::Stopping);
    assert!(!stopping.manager_joined);
    let mut replaced = account(); replaced.generation += 1;
    assert_eq!(relay.owner_changed(Some(&replaced)).fault, Some(RelaySelectionError::OwnerChanged));
    release_tx.send(()).unwrap();
    let view = thread.join().unwrap().unwrap();
    assert_eq!(view.fault, Some(RelaySelectionError::OwnerChanged));
    let control = run_rx.recv().unwrap();
    assert_eq!(*lock(&control.stops), vec![true]);
    assert!(lock(&relay.state).control.is_some());
    control.joined();
    assert!(relay.status().manager_joined);
    assert!(lock(&relay.state).control.is_none());
}

#[test]
fn endlist_ack_does_not_release_worker_or_sidecar_and_never_means_archive_ready() {
    let relay = coordinator();
    let mut control = None;
    relay.start_with(&account(), destination(), |run, _| {
        let value = FakeRun::running(run); control = Some(value.clone()); Ok(value)
    }).unwrap();
    let control = control.unwrap();
    {
        let mut report = lock(&control.report);
        report.codec_admitted = true;
        report.manager_joined = true;
        report.delivery.get_or_insert_with(Default::default).end_list_acked = true;
        report.sidecar.get_or_insert_with(Default::default);
    }
    assert!(!relay.status().manager_joined);
    lock(&control.report).worker_joined = true;
    assert!(!relay.status().manager_joined);
    lock(&control.report).sidecar.as_mut().unwrap().joined = true;
    let view = relay.status();
    assert_eq!(view.state, SelectionPhase::Joined);
    assert!(view.manager_joined && !view.archive_ready);
    assert!(lock(&relay.state).control.is_none());
}

#[test]
fn transport_first_fault_survives_later_owner_change_and_clean_worker_report() {
    let relay = coordinator();
    let mut control = None;
    relay.start_with(&account(), destination(), |run, _| {
        let value = FakeRun::running(run); control = Some(value.clone()); Ok(value)
    }).unwrap();
    let control = control.unwrap();
    lock(&control.report).fault = Some("Delivery:Network".into());
    assert_eq!(relay.status().fault, Some(RelaySelectionError::RelayFailed));
    relay.owner_changed(None);
    control.joined();
    lock(&control.report).fault = None;
    let view = relay.status();
    assert_eq!(view.fault, Some(RelaySelectionError::RelayFailed));
    assert_eq!(view.first_relay_fault.as_deref(), Some("Delivery:Network"));
    assert_eq!(view.state, SelectionPhase::Failed);
}

#[test]
fn replaced_source_and_drop_cancel_only_the_owned_run_without_fake_join() {
    let relay = coordinator();
    let mut control = None;
    relay.start_with(&account(), destination(), |run, _| {
        let value = FakeRun::running(run); control = Some(value.clone()); Ok(value)
    }).unwrap();
    let control = control.unwrap();
    control.current.store(false, Ordering::Release);
    let view = relay.status();
    assert_eq!(view.fault, Some(RelaySelectionError::SourceChanged));
    assert!(!view.manager_joined);
    drop(relay);
    assert!(lock(&control.stops).iter().all(|cancel| *cancel));
    assert_eq!(lock(&control.stops).len(), 2);
    assert!(!lock(&control.report).manager_joined);
}

struct DelayedObservation {
    run: Uuid,
    count: AtomicUsize,
    entered: Mutex<Option<mpsc::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl RelayControl for DelayedObservation {
    fn observe(&self) -> Observation {
        let count = self.count.fetch_add(1, Ordering::AcqRel);
        if count == 1 {
            lock(&self.entered).take().unwrap().send(()).unwrap();
            lock(&self.release).recv_timeout(Duration::from_secs(2)).unwrap();
        }
        Observation { current_source: true, relay: SoopRelayInspection {
            run_id: self.run.to_string(), manager_joined: count >= 2, worker_joined: count >= 2,
            input_ended: count >= 2, ..Default::default()
        } }
    }
    fn stop(&self, _: bool) {}
}

#[test]
fn concurrent_old_running_observation_cannot_regress_joined_receipt() {
    let relay = Arc::new(coordinator());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    relay.start_with(&account(), destination(), |run, _| Ok(Arc::new(DelayedObservation {
        run, count: AtomicUsize::new(0), entered: Mutex::new(Some(entered_tx)), release: Mutex::new(release_rx),
    }))).unwrap();
    let pending = relay.clone();
    let old = std::thread::spawn(move || pending.status());
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let joined = relay.status();
    assert!(joined.manager_joined);
    release_tx.send(()).unwrap();
    assert!(old.join().unwrap().manager_joined);
    assert_eq!(relay.status().state, SelectionPhase::Joined);
}

#[test]
fn serialized_status_has_no_ingestion_secret_account_authority_or_native_path() {
    let relay = coordinator();
    relay.start_with(&account(), destination(), |run, _| {
        let value = FakeRun::running(run);
        lock(&value.report).sidecar_directory = Some("private-native-path".into());
        Ok(value)
    }).unwrap();
    let json = serde_json::to_string(&relay.status()).unwrap();
    for forbidden in ["controlled-secret", "http_upload_hls", "generation", "private-native-path", "UCabcdefghijklmnopqrstuv"] {
        assert!(!json.contains(forbidden), "status must stay sanitized");
    }
    assert!(json.contains("abcdefghijk") && json.contains("unlisted"));
}
