//! Selected Live operations share the encrypted store, not upload authorization.
use super::{
    live_api::{self, LiveAuthorization, PreparationProgress},
    live_relay::{
        LiveRelayCoordinator, LiveRelaySnapshot, NativeDestinationOwner, NativeRelaySource,
        RelaySelectionError,
    },
    live_store::{self, LiveReceipt},
    model::*,
    oauth, CloudService, Inner,
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager};

const OPERATIONS: usize = 16;
pub(super) struct Operation {
    source: NativeRelaySource,
    coordinator: Arc<LiveRelayCoordinator>,
    cancel: Arc<AtomicBool>,
    worker: bool,
}
#[derive(Default)]
pub(super) struct Runtime {
    pub(super) operations: HashMap<String, Operation>,
    pub(super) authorizing: bool,
    pub(super) auth_workers: usize,
    pub(super) auth_nonce: u64,
    pub(super) auth_cancel: Option<Arc<AtomicBool>>,
    pub(super) account_error: Option<CloudError>,
    // Bounded by the already admitted receipt IDs. Returned remote IDs remain
    // visible even if the encrypted replacement fails; never called durable.
    pending_receipts: HashMap<String, LiveReceipt>,
    archive_checks: HashMap<String, Arc<AtomicBool>>,
    pub(super) sidecar_work: HashMap<String, super::live_sidecar::WorkState>,
    pub(super) replays: Arc<super::live_replay_host::LiveReplayRegistry>,
}
fn error(code: &str, message: &str) -> CloudError {
    CloudError::new(code, message, false)
}
fn changed() -> CloudError {
    error(
        "CLOUD_LIVE_CHANGED",
        "선택한 방송 또는 YouTube 연결 상태가 바뀌었습니다. 다시 선택해 주세요.",
    )
}
fn stopped() -> CloudError {
    error(
        "CLOUD_LIVE_STOPPED",
        "실시간 전송 준비를 중단했습니다. 생성 기록은 보존됩니다.",
    )
}
fn auth_required() -> CloudError {
    error(
        "CLOUD_LIVE_SCOPE",
        "실시간 전송을 위한 YouTube 권한 연결이 필요합니다.",
    )
}
fn reap_finished(threads: &mut Vec<std::thread::JoinHandle<()>>) {
    let mut index = 0;
    while index < threads.len() {
        if threads[index].is_finished() {
            let _ = threads.swap_remove(index).join();
        } else {
            index += 1;
        }
    }
}
fn current_owner(inner: &Inner) -> Option<NativeDestinationOwner> {
    inner
        .catalog
        .live_grant
        .as_ref()
        .map(|g| NativeDestinationOwner {
            channel_id: g.grant.channel_id.clone(),
            generation: inner.catalog.live_generation,
        })
}
fn snapshot(inner: &Inner) -> CloudLiveSnapshot {
    let grant = inner.catalog.live_grant.as_ref().map(|g| &g.grant);
    CloudLiveSnapshot {
        account: CloudLiveAccountSnapshot {
            configured: inner.catalog.client.is_some(),
            connected: grant.is_some(),
            authorizing: inner.live.authorizing,
            channel_id: grant.map(|g| g.channel_id.clone()),
            channel_name: grant.map(|g| g.channel_name.clone()),
            generation: inner.catalog.live_generation,
            error: inner
                .persistence_error
                .clone()
                .or_else(|| inner.live.account_error.clone()),
        },
        operations: inner
            .catalog
            .live_receipts
            .iter()
            .map(|r| {
                inner
                    .live
                    .pending_receipts
                    .get(&r.view.selection_id)
                    .unwrap_or(r)
                    .view
                    .clone()
            })
            .collect(),
    }
}
impl CloudService {
    pub fn live_snapshot(&self) -> Result<CloudLiveSnapshot, CloudError> {
        let inner = self.lock()?;
        Ok(snapshot(&inner))
    }

    fn update_live_receipt(
        &self,
        inner: &mut Inner,
        id: &str,
        update: impl FnOnce(&mut LiveReceipt) -> Result<(), CloudError>,
    ) -> Result<(), CloudError> {
        let mut candidate = inner.catalog.clone();
        let receipt = candidate
            .live_receipts
            .iter_mut()
            .find(|r| r.view.selection_id == id)
            .ok_or_else(changed)?;
        if let Some(pending) = inner.live.pending_receipts.get(id) {
            *receipt = pending.clone();
        }
        receipt.view.receipt_persisted = true;
        update(receipt)?;
        if !receipt.valid() {
            return Err(CloudError::protocol());
        }
        let mut pending = receipt.clone();
        match self.publish(inner, candidate) {
            Ok(()) => {
                inner.live.pending_receipts.remove(id);
                Ok(())
            }
            Err(e) => {
                pending.view.receipt_persisted = false;
                pending.view.error.get_or_insert_with(|| e.clone());
                pending.view.interrupted = true;
                inner.live.pending_receipts.insert(id.into(), pending);
                Err(e)
            }
        }
    }

    pub fn authorize_live(self: &Arc<Self>) -> Result<CloudLiveSnapshot, CloudError> {
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        if inner.live.authorizing
            || inner.live.auth_workers > 0
            || inner.catalog.live_grant.is_some()
        {
            return Ok(snapshot(&inner));
        }
        let client = inner.catalog.client.clone().ok_or_else(auth_required)?;
        let nonce = inner
            .live
            .auth_nonce
            .checked_add(1)
            .ok_or_else(CloudError::storage)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let mut threads = self.threads.lock().map_err(|_| CloudError::storage())?;
        reap_finished(&mut threads);
        inner.live.auth_nonce = nonce;
        inner.live.authorizing = true;
        inner.live.auth_workers += 1;
        inner.live.auth_cancel = Some(cancel.clone());
        inner.live.account_error = None;
        let service = self.clone();
        match std::thread::Builder::new()
            .name("atsumi-live-auth".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    oauth::authorize_live(
                        service.transport.as_ref(),
                        &client,
                        service.open_browser.as_ref(),
                        &cancel,
                    )
                }))
                .unwrap_or_else(|_| {
                    Err(error(
                        "CLOUD_LIVE_AUTH_STOPPED",
                        "실시간 전송 권한 연결이 중단됐습니다.",
                    ))
                });
                if let Ok(mut inner) = service.lock() {
                    inner.live.auth_workers = inner.live.auth_workers.saturating_sub(1);
                    if inner.live.auth_nonce != nonce {
                        return;
                    }
                    inner.live.authorizing = false;
                    inner.live.auth_cancel = None;
                    if inner.closing || cancel.load(Ordering::Acquire) {
                        return;
                    }
                    match result {
                        Ok(grant) => {
                            let mut candidate = inner.catalog.clone();
                            if let Some(next) = candidate.live_generation.checked_add(1) {
                                candidate.live_generation = next;
                                candidate.live_grant = Some(grant);
                                if let Err(e) = service.publish(&mut inner, candidate) {
                                    inner.live.account_error = Some(e);
                                }
                            } else {
                                inner.live.account_error = Some(CloudError::storage());
                            }
                        }
                        Err(e) => inner.live.account_error = Some(e),
                    }
                }
                service.wake.notify_all();
            }) {
            Ok(handle) => threads.push(handle),
            Err(_) => {
                inner.live.authorizing = false;
                inner.live.auth_workers -= 1;
                inner.live.auth_cancel = None;
                return Err(CloudError::storage());
            }
        }
        Ok(snapshot(&inner))
    }

    pub fn disconnect_live(&self) -> Result<CloudLiveSnapshot, CloudError> {
        let owners = {
            let mut inner = self.lock()?;
            Self::admit(&inner)?;
            let nonce = inner
                .live
                .auth_nonce
                .checked_add(1)
                .ok_or_else(CloudError::storage)?;
            let mut candidate = inner.catalog.clone();
            candidate.live_generation = candidate
                .live_generation
                .checked_add(1)
                .ok_or_else(CloudError::storage)?;
            candidate.live_grant = None;
            for receipt in &mut candidate.live_receipts {
                if receipt.view.state == "selected" {
                    receipt.view.state = "failed".into();
                    receipt.view.stop_requested = true;
                    receipt.view.error.get_or_insert_with(changed);
                    receipt.changed()?;
                }
            }
            self.publish(&mut inner, candidate)?;
            inner.live.auth_nonce = nonce;
            if let Some(cancel) = inner.live.auth_cancel.take() {
                cancel.store(true, Ordering::Release);
            }
            inner.live.authorizing = false;
            for cancel in inner.live.archive_checks.values() { cancel.store(true, Ordering::Release); }
            inner.live.operations.retain(|_, o| o.worker);
            inner
                .live
                .operations
                .values()
                .map(|o| {
                    o.cancel.store(true, Ordering::Release);
                    o.coordinator.clone()
                })
                .collect::<Vec<_>>()
        };
        for owner in owners {
            owner.owner_changed(None);
        }
        self.live_snapshot()
    }

    pub(crate) fn select_live(
        &self,
        app: &AppHandle,
        source: NativeRelaySource,
    ) -> Result<CloudLiveSnapshot, CloudError> {
        if !source.current(app) {
            return Err(changed());
        }
        let facts = source.facts();
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        if inner
            .live
            .operations
            .values()
            .any(|o| o.source.facts() == facts)
        {
            return Ok(snapshot(&inner));
        }
        let previous: Vec<_> = inner
            .catalog
            .live_receipts
            .iter()
            .filter(|r| {
                r.view.provider == facts.provider
                    && r.view.channel_id == facts.channel_id
                    && !live_store::terminal(&r.view.state)
            })
            .map(|r| r.view.selection_id.clone())
            .collect();
        if previous
            .iter()
            .any(|id| inner.live.operations.get(id).is_some_and(|o| o.worker))
        {
            return Err(changed());
        }
        if inner.live.admissions() >= OPERATIONS
            || inner.catalog.live_receipts.len() >= live_store::RECEIPTS
        {
            return Err(error(
                "CLOUD_LIVE_LIMIT",
                "보존 중인 실시간 전송 작업 한도에 도달했습니다.",
            ));
        }
        let coordinator = Arc::new(LiveRelayCoordinator::select_native(source.clone()));
        let id = coordinator.status().selection_id;
        let mut candidate = inner.catalog.clone();
        for receipt in &mut candidate.live_receipts {
            if previous.contains(&receipt.view.selection_id) {
                receipt.view.state = "failed".into();
                receipt.view.stop_requested = true;
                receipt.view.error.get_or_insert_with(changed);
                receipt.changed()?;
            }
        }
        candidate
            .live_receipts
            .push(LiveReceipt::selected(id.clone(), facts));
        // Capacity is checked before any remote effect. Existing Vault limit is unchanged.
        if serde_json::to_vec(&candidate)
            .map_err(|_| CloudError::storage())?
            .len()
            > 4 * 1024 * 1024 - 8192
        {
            return Err(error(
                "CLOUD_LIVE_LIMIT",
                "실시간 전송 기록을 보존할 공간 한도에 도달했습니다.",
            ));
        }
        self.publish(&mut inner, candidate)?;
        for id in previous {
            inner.live.operations.remove(&id);
        }
        inner.live.operations.insert(
            id,
            Operation {
                source,
                coordinator,
                cancel: Arc::new(AtomicBool::new(false)),
                worker: false,
            },
        );
        Ok(snapshot(&inner))
    }

    pub fn start_live(
        self: &Arc<Self>,
        app: AppHandle,
        selection_id: &str,
        expected_revision: u64,
        plan: CloudLivePlan,
    ) -> Result<CloudLiveSnapshot, CloudError> {
        if !plan.valid() {
            return Err(error(
                "CLOUD_LIVE_PLAN",
                "방송 제목·예약 시각·아동용 여부를 확인해 주세요.",
            ));
        }
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        let owner = current_owner(&inner).ok_or_else(auth_required)?;
        let op = inner
            .live
            .operations
            .get(selection_id)
            .ok_or_else(changed)?;
        if op.worker || op.cancel.load(Ordering::Acquire) {
            return Err(changed());
        }
        let (source, coordinator, cancel) =
            (op.source.clone(), op.coordinator.clone(), op.cancel.clone());
        let mut candidate = inner.catalog.clone();
        let receipt = candidate
            .live_receipts
            .iter_mut()
            .find(|r| r.view.selection_id == selection_id)
            .ok_or_else(changed)?;
        if receipt.view.revision != expected_revision || receipt.view.state != "selected" {
            return Err(changed());
        }
        receipt.plan = Some(plan);
        receipt.owner_generation = Some(owner.generation);
        receipt.view.destination_channel_id = Some(owner.channel_id.clone());
        receipt.view.state = "preparing".into();
        receipt.changed()?;
        let request_id = receipt.request_id.clone();
        // Handle publication shares the same closing lock; shutdown cannot miss a newly spawned worker.
        let mut threads = self.threads.lock().map_err(|_| CloudError::storage())?;
        reap_finished(&mut threads);
        self.publish(&mut inner, candidate)?;
        inner
            .live
            .operations
            .get_mut(selection_id)
            .ok_or_else(changed)?
            .worker = true;
        let service = self.clone();
        let id = selection_id.to_owned();
        let handle = std::thread::Builder::new()
            .name("atsumi-live-selected".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    service.prepare_live(
                        &app,
                        &id,
                        &request_id,
                        &source,
                        &coordinator,
                        &owner,
                        &cancel,
                    )
                }))
                .unwrap_or_else(|_| {
                    Err(error(
                        "CLOUD_LIVE_WORKER",
                        "실시간 전송 작업이 중단됐습니다. 생성 기록은 유지됩니다.",
                    ))
                });
                if let Err(e) = result {
                    let _ = service.live_fail(&id, e);
                    coordinator.stop(true);
                }
                // Even unwinding setup cannot discard an installed manager or claim join early.
                loop {
                    let view = coordinator.status();
                    if view.run_id.is_none()
                        || view.manager_joined
                        || matches!(view.state, super::live_relay::SelectionPhase::Failed)
                            && view.relay.is_none()
                    {
                        break;
                    }
                    if let Some(host) = app.try_state::<crate::soop::original::SoopOriginalHost>() {
                        host.poll_relay_join(&app);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let final_view = coordinator.status();
                if service.live_observe(&id, &final_view, true).is_ok() {
                    // Preserve the actual joined writer identity even if sealing
                    // is cancelled by shutdown; reopen can finish that same seal.
                    if let Err(error) = service.finish_live_sidecar(&id, &final_view) {
                        tracing::debug!(code = error.code.as_str(), "live sidecar remains unverified");
                    }
                    // The explicit selected start owns one automatic metadata
                    // window. Its failure must not rewrite the local relay result.
                    service.automatic_archive_check(&id, &request_id, &owner);
                }
                if let Ok(mut inner) = service.lock() {
                    inner.live.operations.remove(&id);
                }
                service.wake.notify_all();
            });
        match handle {
            Ok(handle) => threads.push(handle),
            Err(_) => {
                inner.live.operations.remove(selection_id);
                let mut candidate = inner.catalog.clone();
                if let Some(receipt) = candidate
                    .live_receipts
                    .iter_mut()
                    .find(|r| r.view.selection_id == selection_id)
                {
                    receipt.view.state = "failed".into();
                    receipt.view.error = Some(CloudError::storage());
                    receipt.changed()?;
                }
                self.publish(&mut inner, candidate)?;
                return Err(CloudError::storage());
            }
        }
        Ok(snapshot(&inner))
    }

    fn live_authorization(
        &self,
        owner: &NativeDestinationOwner,
    ) -> Result<LiveAuthorization, CloudError> {
        let (client, grant) = {
            let inner = self.lock()?;
            Self::admit(&inner)?;
            if current_owner(&inner).as_ref() != Some(owner) {
                return Err(changed());
            }
            (
                inner.catalog.client.clone().ok_or_else(auth_required)?,
                inner.catalog.live_grant.clone().ok_or_else(auth_required)?,
            )
        };
        let grant = if grant.grant.expires_at <= now().saturating_add(60_000) {
            let refreshed = oauth::refresh_live(self.transport.as_ref(), &client, &grant)?;
            let mut inner = self.lock()?;
            Self::admit(&inner)?;
            if current_owner(&inner).as_ref() != Some(owner)
                || refreshed.grant.channel_id != owner.channel_id
            {
                return Err(changed());
            }
            let mut candidate = inner.catalog.clone();
            candidate.live_grant = Some(refreshed.clone());
            self.publish(&mut inner, candidate)?;
            refreshed
        } else {
            grant
        };
        LiveAuthorization::from_granted_scopes(
            grant.grant.access_token,
            owner.clone(),
            &grant.scopes.join(" "),
        )
    }

    fn prepare_live(
        &self,
        app: &AppHandle,
        id: &str,
        request_id: &str,
        source: &NativeRelaySource,
        coordinator: &LiveRelayCoordinator,
        owner: &NativeDestinationOwner,
        cancel: &AtomicBool,
    ) -> Result<(), CloudError> {
        if cancel.load(Ordering::Acquire) || !source.current(app) {
            return Err(stopped());
        }
        let auth = self.live_authorization(owner)?;
        let transport = live_api::GoogleLiveTransport::new()?;
        let prepared =
            self.provision_live(&transport, &auth, id, request_id, owner, cancel, || {
                source.current(app)
            })?;
        coordinator
            .start_native(app, owner, prepared.destination, |run| {
                let mut inner = self.lock().map_err(|_| RelaySelectionError::Persistence)?;
                if inner.closing
                    || cancel.load(Ordering::Acquire)
                    || current_owner(&inner).as_ref() != Some(owner)
                {
                    return Err(RelaySelectionError::Cancelled);
                }
                let mut candidate = inner.catalog.clone();
                let receipt = candidate
                    .live_receipts
                    .iter_mut()
                    .find(|r| r.view.selection_id == id)
                    .ok_or(RelaySelectionError::Persistence)?;
                receipt.view.run_id = Some(run.to_string());
                receipt.view.state = "starting".into();
                receipt
                    .changed()
                    .and_then(|_| self.publish(&mut inner, candidate))
                    .map_err(|_| RelaySelectionError::Persistence)
            })
            .map_err(|_| {
                error(
                    "CLOUD_LIVE_START",
                    "선택한 방송의 전송을 시작하지 못했습니다. 생성 기록은 보존됩니다.",
                )
            })?;
        let mut last_state = String::new();
        loop {
            let owner_now = {
                let inner = self.lock()?;
                if let Some(e) = &inner.persistence_error {
                    return Err(e.clone());
                }
                current_owner(&inner)
            };
            coordinator.owner_changed(owner_now.as_ref());
            let view = coordinator.status();
            let state = format!(
                "{:?}:{}:{:?}:{:?}",
                view.state,
                view.manager_joined,
                view.fault,
                view.relay
                    .as_ref()
                    .and_then(|r| r.delivery.as_ref())
                    .map(|d| (d.segment_acks, d.playlist_acks, d.end_list_acked))
            );
            if state != last_state {
                self.live_observe(id, &view, false)?;
                last_state = state;
            }
            if view.manager_joined {
                return Ok(());
            }
            if let Some(host) = app.try_state::<crate::soop::original::SoopOriginalHost>() {
                host.poll_relay_join(app);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    fn provision_live(
        &self,
        transport: &dyn super::upload::Transport,
        auth: &LiveAuthorization,
        id: &str,
        request_id: &str,
        owner: &NativeDestinationOwner,
        cancel: &AtomicBool,
        source_current: impl Fn() -> bool,
    ) -> Result<live_api::PreparationSuccess, CloudError> {
        let plan = self
            .lock()?
            .catalog
            .live_receipts
            .iter()
            .find(|r| r.view.selection_id == id)
            .and_then(|r| r.plan.clone())
            .ok_or_else(changed)?;
        live_api::prepare(
            transport,
            auth,
            &live_api::PreparationPlan {
                request_id: uuid::Uuid::parse_str(request_id).map_err(|_| changed())?,
                title: plan.title,
                scheduled_start_ms: plan.scheduled_start_ms,
                made_for_kids: plan.made_for_kids,
            },
            |progress| {
                // First preserve IDs returned by an already-issued request, even after cancellation.
                self.live_progress(id, progress)?;
                if cancel.load(Ordering::Acquire) || !source_current() {
                    return Err(stopped());
                }
                let inner = self.lock()?;
                if inner.closing || current_owner(&inner).as_ref() != Some(owner) {
                    return Err(changed());
                }
                Ok(())
            },
        )
        .map_err(|failure| failure.error)
    }
    fn live_progress(&self, id: &str, progress: &PreparationProgress) -> Result<(), CloudError> {
        let mut inner = self.lock()?;
        self.update_live_receipt(&mut inner, id, |r| r.progress(progress))
    }
    fn live_fail(&self, id: &str, error: CloudError) -> Result<(), CloudError> {
        let mut inner = self.lock()?;
        self.update_live_receipt(&mut inner, id, |receipt| {
            receipt.view.error.get_or_insert(error);
            receipt.view.interrupted = true;
            receipt.view.stop_requested = true;
            receipt.changed()
        })
    }
    pub(super) fn live_observe(
        &self,
        id: &str,
        observed: &LiveRelaySnapshot,
        finished: bool,
    ) -> Result<(), CloudError> {
        let mut inner = self.lock()?;
        self.update_live_receipt(&mut inner, id, |receipt| {
            let before = (
                receipt.view.state.clone(),
                receipt.view.stop_requested,
                receipt.view.interrupted,
                receipt.view.manager_joined,
                receipt.view.worker_joined,
                receipt.view.sidecar_joined,
                receipt.view.error.clone(),
            );
            receipt.view.run_id = observed
                .run_id
                .clone()
                .or_else(|| receipt.view.run_id.clone());
            receipt.view.stop_requested |= observed.stop_requested;
            receipt.view.interrupted |= observed.interrupted;
            receipt.view.manager_joined = observed.manager_joined;
            if let Some(relay) = &observed.relay {
                receipt.view.worker_joined = relay.worker_joined;
                receipt.view.sidecar_joined = relay.sidecar.as_ref().is_some_and(|s| s.joined);
                receipt.sidecar_id = relay
                    .sidecar
                    .as_ref()
                    .filter(|s| !s.sidecar_id.is_empty())
                    .map(|s| s.sidecar_id.clone());
                if let Some(sidecar) = &relay.sidecar {
                    receipt.view.sidecar_text_state =
                        (!sidecar.text_state.is_empty()).then(|| sidecar.text_state.clone());
                    receipt.view.assets_incomplete |= sidecar.assets_incomplete;
                    receipt.view.timing_incomplete |= sidecar.timing_incomplete;
                    receipt.view.clock_quality =
                        (!sidecar.clock_quality.is_empty()).then(|| sidecar.clock_quality.clone());
                }
                if let Some(delivery) = &relay.delivery {
                    receipt.view.segment_acks = delivery.segment_acks;
                    receipt.view.playlist_acks = delivery.playlist_acks;
                    receipt.view.end_list_acked = delivery.end_list_acked;
                }
            }
            if observed.fault.is_some() {
                receipt.view.error.get_or_insert_with(|| {
                    error(
                        "CLOUD_LIVE_RELAY",
                        "실시간 전송이 중단되었습니다. 전송·채팅 기록은 보존됩니다.",
                    )
                });
            }
            let joined = observed.manager_joined
                && receipt.view.worker_joined
                && receipt.view.end_list_acked;
            if finished && !joined {
                receipt.view.error.get_or_insert_with(stopped);
            }
            receipt.view.state = if finished {
                if receipt.view.error.is_some() || !joined {
                    "failed"
                } else {
                    "joined"
                }
            } else if observed.stop_requested {
                "stopping"
            } else {
                "running"
            }
            .into();
            let after = (
                receipt.view.state.clone(),
                receipt.view.stop_requested,
                receipt.view.interrupted,
                receipt.view.manager_joined,
                receipt.view.worker_joined,
                receipt.view.sidecar_joined,
                receipt.view.error.clone(),
            );
            if before != after {
                receipt.changed()?;
            } else {
                receipt.updated_at = now();
            }
            Ok(())
        })
    }

    pub fn stop_live(
        &self,
        id: &str,
        expected_revision: u64,
    ) -> Result<CloudLiveSnapshot, CloudError> {
        let coordinator = {
            let mut inner = self.lock()?;
            Self::admit(&inner)?;
            let mut candidate = inner.catalog.clone();
            let receipt = candidate
                .live_receipts
                .iter_mut()
                .find(|r| r.view.selection_id == id)
                .ok_or_else(changed)?;
            if receipt.view.revision != expected_revision {
                return Err(changed());
            }
            if live_store::terminal(&receipt.view.state) {
                return Ok(snapshot(&inner));
            }
            let running = inner.live.operations.get(id).is_some_and(|o| o.worker);
            receipt.view.stop_requested = true;
            receipt.view.state = if running { "stopping" } else { "failed" }.into();
            if !running {
                receipt.view.error.get_or_insert_with(stopped);
            }
            receipt.changed()?;
            self.publish(&mut inner, candidate)?;
            let op = inner.live.operations.get(id).ok_or_else(changed)?;
            op.cancel.store(true, Ordering::Release);
            let coordinator = op.coordinator.clone();
            if !running {
                inner.live.operations.remove(id);
            }
            coordinator
        };
        coordinator.stop(false);
        self.live_snapshot()
    }

    /// Called before taking thread handles. Normal stop does not revoke the
    /// destination generation, so the monitor never turns quit into owner loss.
    pub(super) fn request_live_shutdown(&self) -> Result<(), CloudError> {
        let coordinators = {
            let mut inner = self.lock()?;
            inner.closing = true;
            inner.live.auth_nonce = inner.live.auth_nonce.saturating_add(1);
            if let Some(cancel) = inner.live.auth_cancel.take() {
                cancel.store(true, Ordering::Release);
            }
            inner.live.authorizing = false;
            for cancel in inner.live.archive_checks.values() { cancel.store(true, Ordering::Release); }
            for work in inner.live.sidecar_work.values() { work.cancel.store(true, Ordering::Release); }
            let idle = inner
                .live
                .operations
                .iter()
                .filter(|(_, op)| !op.worker)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for id in idle {
                let _ = self.update_live_receipt(&mut inner, &id, |receipt| {
                    receipt.view.state = "failed".into();
                    receipt.view.stop_requested = true;
                    receipt.view.error.get_or_insert_with(stopped);
                    receipt.changed()
                });
                inner.live.operations.remove(&id);
            }
            inner
                .live
                .operations
                .values()
                .map(|op| {
                    op.cancel.store(true, Ordering::Release);
                    op.coordinator.clone()
                })
                .collect::<Vec<_>>()
        };
        for coordinator in coordinators {
            coordinator.stop(false);
        }
        Ok(())
    }
}

impl Runtime {
    pub(super) fn cancel_admission(&self) {
        if let Some(cancel) = &self.auth_cancel {
            cancel.store(true, Ordering::Release);
        }
        for operation in self.operations.values() {
            operation.cancel.store(true, Ordering::Release);
        }
        for cancel in self.archive_checks.values() { cancel.store(true, Ordering::Release); }
        for work in self.sidecar_work.values() { work.cancel.store(true, Ordering::Release); }
    }
    pub(super) fn discard_idle(&mut self) {
        self.operations.retain(|_, op| op.worker);
    }
    pub(super) fn active_ids(&self) -> Vec<String> {
        let mut ids = self
            .operations
            .iter()
            .filter(|(_, o)| o.worker)
            .map(|(id, _)| format!("live:{id}"))
            .collect::<Vec<_>>();
        if self.authorizing || self.auth_workers > 0 {
            ids.push("live:authorization".into());
        }
        for id in self.archive_checks.keys() {
            if !self.operations.contains_key(id) { ids.push(format!("live-archive:{id}")); }
        }
        ids.extend(self.sidecar_work.iter().filter(|(_, w)| w.active()).map(|(id, _)| format!("live-history:{id}")));
        ids.extend(self.replays.active_ids());
        ids
    }
    fn admissions(&self) -> usize {
        self.operations.len() + self.archive_checks.keys().filter(|id| !self.operations.contains_key(*id)).count()
    }
    pub(super) fn busy(&self) -> bool {
        self.authorizing || self.auth_workers > 0 || !self.archive_checks.is_empty() || self.sidecar_work.values().any(super::live_sidecar::WorkState::active) || self.operations.values().any(|o| o.worker) || !self.replays.active_ids().is_empty()
    }
}

#[path = "live_archive_service.rs"]
mod archive;

#[cfg(test)]
#[path = "live_service_tests.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "native_live_authorize_qa.rs"]
mod native_live_authorize_qa;

#[cfg(all(test, windows))]
#[path = "native_live_relay_qa.rs"]
mod native_live_relay_qa;
