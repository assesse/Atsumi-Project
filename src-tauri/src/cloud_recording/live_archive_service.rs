//! One durable read-only verification window owned by an explicit selected start.
use super::super::live_archive::{
    self, ArchiveReason, ArchiveState, Authorization, GoogleArchiveTransport, Identity, Record,
};
use super::super::upload::Transport;
use super::*;

struct Ticket {
    identity: Identity,
    attempt: String,
    revision: u64,
    owner: NativeDestinationOwner,
    cancel: Arc<AtomicBool>,
}
fn archive_error(reason: ArchiveReason) -> CloudError {
    error(
        match reason {
            ArchiveReason::OwnerChanged | ArchiveReason::OwnerMismatch => "CLOUD_LIVE_CHANGED",
            ArchiveReason::SaveFailed => "CLOUD_STORAGE",
            _ => "CLOUD_LIVE_ARCHIVE_CHECK",
        },
        "보관 영상의 확인 상태를 갱신하지 못했습니다. 기존 전송 기록은 유지됩니다.",
    )
}
fn matches_ticket(receipt: &LiveReceipt, ticket: &Ticket) -> bool {
    receipt.archive_identity().as_ref() == Some(&ticket.identity)
        && receipt
            .archive
            .as_ref()
            .is_some_and(|a| a.attempt_id == ticket.attempt && a.check_revision == ticket.revision)
}

impl CloudService {
    // Archive-only persistence deliberately does not call live_fail or change
    // the locally joined/error/interrupted facts when a GET or save fails.
    fn save_archive(
        &self,
        inner: &mut Inner,
        id: &str,
        record: Record,
    ) -> Result<(), ArchiveReason> {
        let mut candidate = inner.catalog.clone();
        let receipt = candidate
            .live_receipts
            .iter_mut()
            .find(|r| r.view.selection_id == id)
            .ok_or(ArchiveReason::IdentityMismatch)?;
        if receipt.archive_identity().as_ref() != Some(&record.identity) || !record.valid() {
            return Err(ArchiveReason::IdentityMismatch);
        }
        let old = receipt
            .archive
            .as_ref()
            .map(|a| (a.state, a.reason, a.attempt_id.as_str(), a.check_revision));
        let changed = old
            != Some((
                record.state,
                record.reason,
                record.attempt_id.as_str(),
                record.check_revision,
            ));
        receipt.view.archive_verification = Some(record.snapshot());
        receipt.archive = Some(record);
        if changed {
            receipt.changed().map_err(|_| ArchiveReason::SaveFailed)?;
        } else {
            receipt.updated_at = now();
        }
        if !receipt.valid() {
            return Err(ArchiveReason::InvalidResponse);
        }
        let mut pending = receipt.clone();
        match self.publish(inner, candidate) {
            Ok(()) => {
                inner.live.pending_receipts.remove(id);
                Ok(())
            }
            Err(_) => {
                pending.view.receipt_persisted = false;
                if let Some(a) = pending.archive.as_mut() {
                    a.stop(ArchiveReason::SaveFailed);
                    pending.view.archive_verification = Some(a.snapshot());
                }
                inner.live.pending_receipts.insert(id.into(), pending);
                Err(ArchiveReason::SaveFailed)
            }
        }
    }
    fn reserve_archive(
        &self,
        inner: &mut Inner,
        id: &str,
        owner: NativeDestinationOwner,
        explicit: bool,
        at: u64,
    ) -> Result<Option<Ticket>, ArchiveReason> {
        if inner.live.archive_checks.contains_key(id) {
            return Ok(None);
        }
        if !inner.live.operations.contains_key(id) && inner.live.admissions() >= OPERATIONS {
            return Err(ArchiveReason::RequestLimit);
        }
        let receipt = inner
            .catalog
            .live_receipts
            .iter()
            .find(|r| r.view.selection_id == id)
            .ok_or(ArchiveReason::IdentityMismatch)?;
        let identity = receipt
            .archive_identity()
            .ok_or(ArchiveReason::IdentityMismatch)?;
        if identity.owner_channel_id != owner.channel_id {
            return Err(ArchiveReason::OwnerMismatch);
        }
        if !explicit && (receipt.archive.is_some() || receipt.view.state != "joined") {
            return Ok(None);
        }
        let mut record = match receipt.archive.as_ref() {
            Some(previous) if at < previous.deadline_at => {
                let mut record = previous.clone();
                record.resume(owner.generation, at)?;
                record
            }
            prior => Record::begin(identity.clone(), owner.generation, at, prior)?,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        if inner.closing {
            record.stop(ArchiveReason::Cancelled);
        } else if current_owner(inner).as_ref() != Some(&owner) {
            record.stop(ArchiveReason::OwnerChanged);
        }
        let ticket = Ticket {
            identity,
            attempt: record.attempt_id.clone(),
            revision: record.check_revision,
            owner,
            cancel: cancel.clone(),
        };
        self.save_archive(inner, id, record)?;
        inner.live.archive_checks.insert(id.into(), cancel);
        Ok(Some(ticket))
    }
    fn archive_fence(&self, inner: &Inner, ticket: &Ticket) -> Result<Record, ArchiveReason> {
        let id = &ticket.identity.selection_id;
        if inner.persistence_error.is_some() {
            return Err(ArchiveReason::SaveFailed);
        }
        if !inner
            .live
            .archive_checks
            .get(id)
            .is_some_and(|c| Arc::ptr_eq(c, &ticket.cancel))
        {
            return Err(ArchiveReason::OwnerChanged);
        }
        let receipt = inner
            .catalog
            .live_receipts
            .iter()
            .find(|r| &r.view.selection_id == id)
            .ok_or(ArchiveReason::IdentityMismatch)?;
        if !matches_ticket(receipt, ticket) {
            return Err(ArchiveReason::IdentityMismatch);
        }
        if current_owner(inner).as_ref() != Some(&ticket.owner) {
            return Err(ArchiveReason::OwnerChanged);
        }
        if inner.closing || ticket.cancel.load(Ordering::Acquire) {
            return Err(ArchiveReason::Cancelled);
        }
        Ok(receipt.archive.as_ref().unwrap().clone())
    }
    fn stop_archive_ticket(&self, ticket: &Ticket, reason: ArchiveReason) {
        if let Ok(mut inner) = self.lock() {
            if inner.persistence_error.is_some() {
                return;
            }
            let id = &ticket.identity.selection_id;
            if !inner
                .live
                .archive_checks
                .get(id)
                .is_some_and(|c| Arc::ptr_eq(c, &ticket.cancel))
            {
                return;
            }
            let record = inner
                .catalog
                .live_receipts
                .iter()
                .find(|r| &r.view.selection_id == id && matches_ticket(r, ticket))
                .and_then(|r| r.archive.clone());
            if let Some(mut record) = record {
                record.stop(reason);
                let _ = self.save_archive(&mut inner, id, record);
            }
        }
    }
    fn release_archive_ticket(&self, ticket: &Ticket) {
        if let Ok(mut inner) = self.lock() {
            let id = &ticket.identity.selection_id;
            if inner
                .live
                .archive_checks
                .get(id)
                .is_some_and(|c| Arc::ptr_eq(c, &ticket.cancel))
            {
                inner.live.archive_checks.remove(id);
            }
        }
        self.wake.notify_all();
    }
    fn run_archive_check(
        &self,
        ticket: &Ticket,
        transport: &dyn Transport,
        mut authorize: impl FnMut() -> Result<Authorization, ArchiveReason>,
        mut clock: impl FnMut() -> u64,
        mut wait: impl FnMut(Duration),
    ) -> Result<(), ArchiveReason> {
        loop {
            let at = clock();
            let mut inner = self.lock().map_err(|_| ArchiveReason::SaveFailed)?;
            let mut record = self.archive_fence(&inner, ticket)?;
            if !matches!(record.state, ArchiveState::Pending | ArchiveState::Checking) {
                return Ok(());
            }
            record.budget(at)?;
            if let Some(due) = record.retry_at.filter(|due| *due > at) {
                drop(inner);
                wait(Duration::from_millis((due - at).min(1_000)));
                continue;
            }
            record.start_round(at)?;
            self.save_archive(&mut inner, &ticket.identity.selection_id, record)?;
            drop(inner);
            // Refresh uses the separate verified Live grant, never the upload grant.
            let auth = authorize()?;
            if auth.generation != ticket.owner.generation {
                return Err(ArchiveReason::OwnerChanged);
            }
            let round = live_archive::verify_round(transport, &auth, &ticket.identity, || {
                let at = clock();
                let mut inner = self.lock().map_err(|_| ArchiveReason::SaveFailed)?;
                let mut record = self.archive_fence(&inner, ticket)?;
                record.request(at)?;
                self.save_archive(&mut inner, &ticket.identity.selection_id, record)
            });
            let at = clock();
            let mut inner = self.lock().map_err(|_| ArchiveReason::SaveFailed)?;
            let mut record = self.archive_fence(&inner, ticket)?;
            record.apply(round, at)?;
            let pending = record.state == ArchiveState::Pending;
            self.save_archive(&mut inner, &ticket.identity.selection_id, record)?;
            if !pending {
                return Ok(());
            }
        }
    }
    fn execute_archive_ticket(&self, ticket: &Ticket) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Check before client construction as well as before every GET.
            {
                let inner = self.lock().map_err(|_| ArchiveReason::SaveFailed)?;
                self.archive_fence(&inner, ticket)?;
            }
            let transport = GoogleArchiveTransport::new().map_err(|_| ArchiveReason::Network)?;
            self.run_archive_check(
                ticket,
                &transport,
                || {
                    self.live_authorization(&ticket.owner)
                        .map_err(|_| ArchiveReason::Authorization)?
                        .archive_authorization()
                },
                now,
                |duration| {
                    if let Ok(inner) = self.lock() {
                        let _ = self.wake.wait_timeout(inner, duration);
                    }
                },
            )
        }))
        .unwrap_or(Err(ArchiveReason::WorkerPanic));
        if let Err(reason) = result {
            self.stop_archive_ticket(ticket, reason);
        }
        self.release_archive_ticket(ticket);
    }
    pub(super) fn automatic_archive_check(
        &self,
        id: &str,
        request: &str,
        owner: &NativeDestinationOwner,
    ) {
        let ticket = self.lock().ok().and_then(|mut inner| {
            if !inner
                .catalog
                .live_receipts
                .iter()
                .any(|r| r.view.selection_id == id && r.request_id == request)
            {
                return None;
            }
            self.reserve_archive(&mut inner, id, owner.clone(), false, now())
                .ok()
                .flatten()
        });
        if let Some(ticket) = ticket {
            self.execute_archive_ticket(&ticket);
        }
    }
    pub fn verify_live_archive(
        self: &Arc<Self>,
        id: &str,
        expected_revision: u64,
    ) -> Result<CloudLiveSnapshot, CloudError> {
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        let receipt = inner
            .catalog
            .live_receipts
            .iter()
            .find(|r| r.view.selection_id == id)
            .ok_or_else(changed)?;
        if receipt.view.revision != expected_revision {
            return Err(changed());
        }
        if inner.live.archive_checks.contains_key(id) {
            return Ok(snapshot(&inner));
        }
        let owner = current_owner(&inner).ok_or_else(auth_required)?;
        // Reservation and handle publication share shutdown's closing lock.
        let mut threads = self.threads.lock().map_err(|_| CloudError::storage())?;
        reap_finished(&mut threads);
        let Some(ticket) = self
            .reserve_archive(&mut inner, id, owner, true, now())
            .map_err(archive_error)?
        else {
            return Ok(snapshot(&inner));
        };
        let cancel = ticket.cancel.clone();
        let service = self.clone();
        match std::thread::Builder::new()
            .name("atsumi-live-archive".into())
            .spawn(move || service.execute_archive_ticket(&ticket))
        {
            Ok(handle) => threads.push(handle),
            Err(_) => {
                cancel.store(true, Ordering::Release);
                inner.live.archive_checks.remove(id);
                if let Some(mut record) = inner
                    .catalog
                    .live_receipts
                    .iter()
                    .find(|r| r.view.selection_id == id)
                    .and_then(|r| r.archive.clone())
                {
                    record.stop(ArchiveReason::WorkerPanic);
                    self.save_archive(&mut inner, id, record)
                        .map_err(archive_error)?;
                }
                return Err(CloudError::storage());
            }
        }
        Ok(snapshot(&inner))
    }
}

#[cfg(test)]
#[path = "live_archive_service_tests.rs"]
mod tests;
