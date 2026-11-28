//! Selected live-source ownership. This layer never provisions an account or
//! destination, and never turns a transport ACK into an archive receipt.
use crate::{
    soop::original::{SelectedRelayHandle, SoopOriginalHost, SoopRelayInspection, SoopRelaySource},
    streaming::relay_hls::HlsTarget,
};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

/// Native capabilities only. Persisted source facts never reconstruct this enum.
#[derive(Clone)]
pub(crate) enum NativeRelaySource {
    Soop(SoopRelaySource),
    Chzzk(crate::streaming::browser::ChzzkRelaySource),
}
impl NativeRelaySource {
    pub(crate) fn facts(&self) -> super::live_store::SourceFacts {
        let (provider, channel, broadcast, session, epoch, revision) = match self {
            Self::Soop(s) => ("soop", s.channel(), Some(s.broadcast()), s.session_id(), s.epoch(), s.account_revision()),
            Self::Chzzk(s) => ("chzzk", s.channel(), s.broadcast(), s.session_id(), s.epoch(), s.account_revision()),
        };
        super::live_store::SourceFacts { provider: provider.into(), channel_id: channel.into(), broadcast_key: broadcast.map(str::to_owned), session_id: session.into(), epoch, account_revision: revision }
    }
    pub(crate) fn current(&self, app: &AppHandle) -> bool {
        match self {
            Self::Soop(s) => app.try_state::<SoopOriginalHost>().is_some_and(|host| host.relay_source_current(s)),
            Self::Chzzk(s) => s.current(),
        }
    }
    fn start(&self, app: &AppHandle, run: Uuid, target: HlsTarget) -> Result<Arc<dyn RelayControl>, RelaySelectionError> {
        match self {
            Self::Soop(s) => app.try_state::<SoopOriginalHost>().ok_or(RelaySelectionError::SourceChanged)?
                .start_selected_hls_relay(app, s, run, target).map(|h| Arc::new(h) as Arc<dyn RelayControl>).map_err(|_| RelaySelectionError::StartRejected),
            Self::Chzzk(s) => s.start(app, run, target).map(|h| Arc::new(h) as Arc<dyn RelayControl>).map_err(|_| RelaySelectionError::StartRejected),
        }
    }
}

/// Supplied by the native account/provisioning owner, never deserialized from IPC.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct NativeDestinationOwner {
    pub(crate) channel_id: String,
    pub(crate) generation: u64,
}

/// Contains the secret ingestion template; deliberately not Debug/Serialize/Clone.
pub(crate) struct PreparedLiveDestination {
    owner: NativeDestinationOwner,
    public: DestinationView,
    target: HlsTarget,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DestinationView {
    pub broadcast_id: String,
    pub stream_id: String,
    pub visibility: &'static str,
}

impl PreparedLiveDestination {
    /// The authenticated native provisioner must first verify resource ownership,
    /// bound stream and lifecycle, and persist its partial-operation receipt.
    /// This factory validates transport shape; it is not OAuth authority.
    pub(crate) fn from_native_response(
        owner: NativeDestinationOwner,
        broadcast_id: String,
        stream_id: String,
        privacy: &str,
        hls_url: &str,
    ) -> Result<Self, RelaySelectionError> {
        if privacy != "unlisted" || !valid_owner(&owner)
            || !identifier(&broadcast_id, 11, 11) || !identifier(&stream_id, 1, 128)
        {
            return Err(RelaySelectionError::Destination);
        }
        let target = HlsTarget::from_api(hls_url).map_err(|_| RelaySelectionError::Destination)?;
        Ok(Self { owner, public: DestinationView { broadcast_id, stream_id, visibility: "unlisted" }, target })
    }

    /// Controlled localhost destination, not a provisioned YouTube broadcast.
    #[cfg(test)]
    pub(crate) fn loopback_test(owner: NativeDestinationOwner, target: HlsTarget) -> Self {
        Self {
            owner,
            public: DestinationView {
                broadcast_id: "fixture0001".into(), stream_id: "controlled-stream".into(), visibility: "unlisted",
            },
            target,
        }
    }
}

fn identifier(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len())
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn valid_owner(owner: &NativeDestinationOwner) -> bool {
    owner.generation != 0 && owner.channel_id.starts_with("UC") && identifier(&owner.channel_id, 24, 24)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RelaySelectionError {
    Destination,
    AlreadyStarted,
    OwnerChanged,
    SourceChanged,
    StartRejected,
    Cancelled,
    RelayFailed,
    Observation,
    Persistence,
}
impl std::fmt::Display for RelaySelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SelectionPhase { Selected, Starting, Running, Stopping, Joined, Failed }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LiveRelaySnapshot {
    pub selection_id: String,
    pub provider: String,
    pub channel_id: String,
    pub broadcast_key: Option<String>,
    pub state: SelectionPhase,
    pub run_id: Option<String>,
    pub destination: Option<DestinationView>,
    pub stop_requested: bool,
    pub interrupted: bool,
    pub fault: Option<RelaySelectionError>,
    pub first_relay_fault: Option<String>,
    pub manager_joined: bool,
    pub relay: Option<SoopRelayInspection>,
    /// Only the future verified remote archive lifecycle may establish this.
    pub archive_ready: bool,
}

struct State {
    view: LiveRelaySnapshot,
    destination_owner: Option<NativeDestinationOwner>,
    control: Option<Arc<dyn RelayControl>>,
}
struct Observation { current_source: bool, relay: SoopRelayInspection }
trait RelayControl: Send + Sync {
    fn observe(&self) -> Observation;
    fn stop(&self, interrupted: bool);
}
impl RelayControl for SelectedRelayHandle {
    fn observe(&self) -> Observation {
        Observation { current_source: self.source_current(), relay: self.inspection() }
    }
    fn stop(&self, interrupted: bool) { self.request_stop(interrupted); }
}
impl RelayControl for crate::streaming::browser::ChzzkSelectedRelayHandle {
    fn observe(&self) -> Observation { Observation { current_source: self.source_current(), relay: self.inspection() } }
    fn stop(&self, interrupted: bool) { self.request_stop(interrupted); }
}

/// One explicit selected operation. Its bounded state retains a single source,
/// destination receipt and exact run; collection/selection UI belongs to caller.
/// Drop requests cancellation, while the source's existing work registration
/// remains owned until the manager, input worker and chat writer actually join.
pub(crate) struct LiveRelayCoordinator {
    source: NativeRelaySource,
    state: Mutex<State>,
}
impl LiveRelayCoordinator {
    pub(crate) fn select(source: SoopRelaySource) -> Self {
        Self::select_native(NativeRelaySource::Soop(source))
    }
    pub(crate) fn select_native(source: NativeRelaySource) -> Self {
        let facts = source.facts();
        let view = LiveRelaySnapshot {
            selection_id: Uuid::new_v4().to_string(), provider: facts.provider,
            channel_id: facts.channel_id, broadcast_key: facts.broadcast_key,
            state: SelectionPhase::Selected, run_id: None, destination: None,
            stop_requested: false, interrupted: false, fault: None,
            first_relay_fault: None,
            manager_joined: false, relay: None, archive_ready: false,
        };
        Self { source, state: Mutex::new(State { view, destination_owner: None, control: None }) }
    }

    /// Safe to check again before each provisioning stage. Start still performs
    /// the atomic host admission check; this observation is not a start permit.
    pub(crate) fn source_current(&self, host: &SoopOriginalHost) -> bool {
        match &self.source { NativeRelaySource::Soop(source) => host.relay_source_current(source), NativeRelaySource::Chzzk(source) => source.current() }
    }

    pub(crate) fn start(
        &self, app: &AppHandle, host: &SoopOriginalHost,
        current_owner: &NativeDestinationOwner, destination: PreparedLiveDestination,
    ) -> Result<LiveRelaySnapshot, RelaySelectionError> {
        self.start_with(current_owner, destination, |run, target| {
            let NativeRelaySource::Soop(source) = &self.source else { return Err(RelaySelectionError::SourceChanged); };
            host.start_selected_hls_relay(app, source, run, target)
                .map(|handle| Arc::new(handle) as Arc<dyn RelayControl>)
                .map_err(|_| RelaySelectionError::StartRejected)
        })
    }
    pub(crate) fn start_native(
        &self, app: &AppHandle, owner: &NativeDestinationOwner, destination: PreparedLiveDestination,
        checkpoint: impl FnOnce(Uuid) -> Result<(), RelaySelectionError>,
    ) -> Result<LiveRelaySnapshot, RelaySelectionError> {
        self.start_with_checkpoint(owner, destination, checkpoint, |run, target| self.source.start(app, run, target))
    }

    fn start_with(
        &self, current_owner: &NativeDestinationOwner, destination: PreparedLiveDestination,
        start: impl FnOnce(Uuid, HlsTarget) -> Result<Arc<dyn RelayControl>, RelaySelectionError>,
    ) -> Result<LiveRelaySnapshot, RelaySelectionError> {
        self.start_with_checkpoint(current_owner, destination, |_| Ok(()), start)
    }
    fn start_with_checkpoint(
        &self, current_owner: &NativeDestinationOwner, destination: PreparedLiveDestination,
        checkpoint: impl FnOnce(Uuid) -> Result<(), RelaySelectionError>,
        start: impl FnOnce(Uuid, HlsTarget) -> Result<Arc<dyn RelayControl>, RelaySelectionError>,
    ) -> Result<LiveRelaySnapshot, RelaySelectionError> {
        let run = Uuid::new_v4();
        {
            let mut state = lock(&self.state);
            if state.view.state != SelectionPhase::Selected || state.view.stop_requested {
                return Err(RelaySelectionError::AlreadyStarted);
            }
            // Preserve known remote IDs even if a source/account became stale
            // after provisioning. Caller still owns remote cleanup/reconciliation.
            state.view.destination = Some(destination.public.clone());
            state.destination_owner = Some(destination.owner.clone());
            if !valid_owner(current_owner) || current_owner != &destination.owner {
                state.view.state = SelectionPhase::Failed;
                fail(&mut state, RelaySelectionError::OwnerChanged);
                return Err(RelaySelectionError::OwnerChanged);
            }
            state.view.state = SelectionPhase::Starting;
            state.view.run_id = Some(run.to_string());
        }
        if let Err(error) = checkpoint(run) {
            let mut state = lock(&self.state); state.view.state = SelectionPhase::Failed;
            fail(&mut state, error); return Err(error);
        }
        // No coordinator lock around source admission, OS/WebView or worker work.
        let started = start(run, destination.target);
        let (control, cancel, stop) = {
            let mut state = lock(&self.state);
            match started {
                Ok(control) => {
                    state.control = Some(control.clone());
                    state.view.state = if state.view.stop_requested { SelectionPhase::Stopping } else { SelectionPhase::Running };
                    (Some(control), state.view.interrupted, state.view.stop_requested)
                }
                Err(error) => {
                    state.view.state = SelectionPhase::Failed;
                    fail(&mut state, error);
                    return Err(error);
                }
            }
        };
        if stop { if let Some(control) = control { control.stop(cancel); } }
        Ok(self.status())
    }

    pub(crate) fn status(&self) -> LiveRelaySnapshot {
        let control = lock(&self.state).control.clone();
        if let Some(control) = control {
            let observation = control.observe();
            let (cancel, retired) = {
                let mut state = lock(&self.state);
                if !state.control.as_ref().is_some_and(|current| Arc::ptr_eq(current, &control)) {
                    // A concurrent observer already retired this exact owner.
                    // Its joined receipt must not regress to an older snapshot.
                    return state.view.clone();
                }
                if state.view.run_id.as_deref() != Some(observation.relay.run_id.as_str()) {
                    fail(&mut state, RelaySelectionError::Observation);
                    state.view.stop_requested = true;
                    state.view.state = SelectionPhase::Stopping;
                    (true, None)
                } else {
                    // A source epoch/account replacement cancels only this run.
                    let cancel = !observation.current_source && !observation.relay.manager_joined;
                    if cancel {
                        fail(&mut state, RelaySelectionError::SourceChanged);
                        state.view.stop_requested = true;
                    }
                    if observation.relay.fault.is_some() || observation.relay.interrupted {
                        fail(&mut state, RelaySelectionError::RelayFailed);
                        state.view.stop_requested = true;
                    }
                    if let Some(fault) = &observation.relay.fault {
                        state.view.first_relay_fault.get_or_insert_with(|| fault.clone());
                    }
                    let joined = observation.relay.manager_joined
                        && (!observation.relay.codec_admitted || observation.relay.worker_joined)
                        && observation.relay.sidecar.as_ref().is_none_or(|s| s.joined);
                    state.view.manager_joined = joined;
                    state.view.relay = Some(observation.relay);
                    let retired = if joined {
                        state.view.state = if state.view.fault.is_some() { SelectionPhase::Failed } else { SelectionPhase::Joined };
                        state.control.take()
                    } else {
                        if state.view.stop_requested { state.view.state = SelectionPhase::Stopping; }
                        None
                    };
                    (cancel, retired)
                }
            };
            if cancel { control.stop(true); }
            drop(retired); // Source/OS cleanup cannot run under coordinator mutex.
        }
        lock(&self.state).view.clone()
    }

    pub(crate) fn stop(&self, interrupted: bool) -> LiveRelaySnapshot {
        let control = {
            let mut state = lock(&self.state);
            if matches!(state.view.state, SelectionPhase::Joined | SelectionPhase::Failed) {
                return state.view.clone();
            }
            state.view.stop_requested = true;
            if interrupted { fail(&mut state, RelaySelectionError::Cancelled); }
            state.view.state = if state.view.run_id.is_none() { SelectionPhase::Failed } else { SelectionPhase::Stopping };
            state.control.clone()
        };
        if let Some(control) = control { control.stop(interrupted); }
        self.status()
    }

    /// Called by the native account owner when disconnect/replacement is observed.
    /// Does not alter the existing upload account or SOOP login/defer policies.
    pub(crate) fn owner_changed(&self, current: Option<&NativeDestinationOwner>) -> LiveRelaySnapshot {
        let control = {
            let mut state = lock(&self.state);
            if state.destination_owner.as_ref().is_none_or(|owner| Some(owner) == current)
                || matches!(state.view.state, SelectionPhase::Joined | SelectionPhase::Failed)
            { return state.view.clone(); }
            fail(&mut state, RelaySelectionError::OwnerChanged);
            state.view.stop_requested = true;
            state.view.state = SelectionPhase::Stopping;
            state.control.clone()
        };
        if let Some(control) = control { control.stop(true); }
        self.status()
    }
}
impl Drop for LiveRelayCoordinator {
    fn drop(&mut self) {
        let control = lock(&self.state).control.take();
        if let Some(control) = control { control.stop(true); }
    }
}
fn fail(state: &mut State, error: RelaySelectionError) {
    state.view.interrupted = true;
    state.view.fault.get_or_insert(error);
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(test)]
#[path = "live_relay_tests.rs"]
mod tests;
