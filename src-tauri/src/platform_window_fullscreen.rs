//! Ownership of the one physical main-window fullscreen presentation.
//! No provider, account, media, recording or navigation state belongs here.
//! Mutations serialize on the UI thread; callbacks never run under our mutex.
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager, Window, WindowEvent};

const UNAVAILABLE: &str = "전체화면 창을 확인할 수 없습니다.";
const NATIVE_FAILED: &str = "전체화면 전환을 완료하지 못했습니다. 다시 시도해 주세요.";
const INVALIDATED: &str = "전체화면 요청의 표시 대상이 변경됐습니다.";
const REENTRANT: &str = "다른 전체화면 전환을 처리하고 있습니다.";
const QUEUE_TIMEOUT: &str = "전체화면 전환을 시작하지 못했습니다. 다시 시도해 주세요.";
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const QUEUED_WAIT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Grant {
    pub owner: String,
    pub document_epoch: u64,
    pub revision: u64,
    pub window_key: String,
}

#[derive(Clone, Debug)]
pub struct AcquireError {
    pub message: String,
    /// Only this request's retained ownership after a failed native rollback.
    /// Callers must not discard this retry capability or report success.
    pub retained_grant: Option<Grant>,
}
impl From<String> for AcquireError {
    fn from(message: String) -> Self {
        Self {
            message,
            retained_grant: None,
        }
    }
}
impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for AcquireError {}

pub type Validity = Arc<dyn Fn() -> bool + Send + Sync>;
pub type OnAcquired = Arc<dyn Fn(Grant) + Send + Sync>;
pub type OnLost = Arc<dyn Fn(Grant) + Send + Sync>;

#[derive(Clone)]
struct Owner {
    grant: Grant,
    on_lost: OnLost,
}
struct Binding {
    native_id: usize,
    key: String,
}
#[derive(Default)]
struct Machine {
    binding: Option<Binding>,
    owner: Option<Owner>,
    revision: u64,
}
#[derive(Default)]
struct Coordinator {
    machine: Mutex<Machine>,
    operating: AtomicBool,
}
struct ManagedCoordinator(Arc<Coordinator>);

thread_local! { static IN_CALLBACK: Cell<bool> = const { Cell::new(false) }; }

struct OperationGuard<'a>(&'a AtomicBool);
impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
struct CallbackGuard(bool);
impl CallbackGuard {
    fn enter() -> Self {
        Self(IN_CALLBACK.with(|v| v.replace(true)))
    }
}
impl Drop for CallbackGuard {
    fn drop(&mut self) {
        IN_CALLBACK.with(|v| v.set(self.0));
    }
}
fn notify(callback: &OnLost, grant: Grant) -> bool {
    let _scope = CallbackGuard::enter();
    catch_unwind(AssertUnwindSafe(|| callback(grant))).is_ok()
}
fn valid(callback: &Validity) -> bool {
    let _scope = CallbackGuard::enter();
    catch_unwind(AssertUnwindSafe(|| callback())).unwrap_or(false)
}

/// Called only by the dispatcher, on the native main-window thread.
trait PresentationWindow {
    fn native_id(&self) -> usize;
    fn fullscreen(&self) -> Result<bool, String>;
    fn set_fullscreen(&self, enabled: bool) -> Result<(), String>;
    fn on_destroyed(&self, callback: Arc<dyn Fn() + Send + Sync>);
}
struct NativeWindow {
    window: Window,
    native_id: usize,
}
impl PresentationWindow for NativeWindow {
    fn native_id(&self) -> usize {
        self.native_id
    }
    fn fullscreen(&self) -> Result<bool, String> {
        self.window
            .is_fullscreen()
            .map_err(|_| NATIVE_FAILED.into())
    }
    fn set_fullscreen(&self, enabled: bool) -> Result<(), String> {
        self.window
            .set_fullscreen(enabled)
            .map_err(|_| NATIVE_FAILED.into())
    }
    fn on_destroyed(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        self.window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                callback();
            }
        });
    }
}

impl Coordinator {
    fn enter(&self) -> Result<OperationGuard<'_>, String> {
        if IN_CALLBACK.with(Cell::get)
            || self
                .operating
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return Err(REENTRANT.into());
        }
        Ok(OperationGuard(&self.operating))
    }

    fn current(&self, grant: &Grant) -> bool {
        self.machine.lock().ok().is_some_and(|state| {
            state
                .binding
                .as_ref()
                .is_some_and(|v| v.key == grant.window_key)
                && state.owner.as_ref().is_some_and(|v| v.grant == *grant)
        })
    }

    fn destroyed(&self, key: &str) {
        let previous = {
            let Ok(mut state) = self.machine.lock() else {
                return;
            };
            if !state.binding.as_ref().is_some_and(|v| v.key == key) {
                return;
            }
            state.binding = None;
            state.owner.take()
        };
        if let Some(previous) = previous {
            notify(&previous.on_lost, previous.grant);
        }
    }

    fn bind(self: &Arc<Self>, window: &impl PresentationWindow) -> Result<String, String> {
        let (key, previous) = {
            let mut state = self.machine.lock().map_err(|_| UNAVAILABLE)?;
            if let Some(binding) = &state.binding {
                if binding.native_id == window.native_id() {
                    return Ok(binding.key.clone());
                }
            }
            let key = uuid::Uuid::new_v4().simple().to_string();
            let previous = state.owner.take();
            state.binding = Some(Binding {
                native_id: window.native_id(),
                key: key.clone(),
            });
            (key, previous)
        };
        let weak = Arc::downgrade(self);
        let destroyed_key = key.clone();
        window.on_destroyed(Arc::new(move || {
            if let Some(state) = weak.upgrade() {
                state.destroyed(&destroyed_key);
            }
        }));
        if let Some(previous) = previous {
            notify(&previous.on_lost, previous.grant);
        }
        Ok(key)
    }

    fn bound(&self, key: &str) -> bool {
        self.machine
            .lock()
            .ok()
            .is_some_and(|v| v.binding.as_ref().is_some_and(|b| b.key == key))
    }

    fn set_confirmed(window: &impl PresentationWindow, enabled: bool) -> Result<(), String> {
        if window.fullscreen()? != enabled {
            window.set_fullscreen(enabled)?;
        }
        if window.fullscreen()? != enabled {
            return Err(NATIVE_FAILED.into());
        }
        Ok(())
    }

    fn install(&self, owner: Owner) -> Result<Option<Owner>, String> {
        let mut state = self.machine.lock().map_err(|_| UNAVAILABLE)?;
        if !state
            .binding
            .as_ref()
            .is_some_and(|b| b.key == owner.grant.window_key)
        {
            return Err(INVALIDATED.into());
        }
        Ok(state.owner.replace(owner))
    }

    fn acquire(
        self: &Arc<Self>,
        window: &impl PresentationWindow,
        owner: String,
        epoch: u64,
        validity: Validity,
        on_acquired: OnAcquired,
        on_lost: OnLost,
    ) -> Result<Grant, AcquireError> {
        let _operation = self.enter().map_err(AcquireError::from)?;
        if owner.is_empty()
            || owner.len() > 128
            || !owner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || epoch == 0
            || epoch > MAX_SAFE_INTEGER
        {
            return Err(String::from("전체화면 표시 대상이 올바르지 않습니다.").into());
        }
        let key = self.bind(window).map_err(AcquireError::from)?;
        if !valid(&validity) {
            return Err(String::from(INVALIDATED).into());
        }
        let (grant, previous) = {
            let mut state = self
                .machine
                .lock()
                .map_err(|_| AcquireError::from(UNAVAILABLE.to_string()))?;
            let previous = state.owner.clone();
            let same = previous.as_ref().filter(|v| {
                v.grant.owner == owner
                    && v.grant.document_epoch == epoch
                    && v.grant.window_key == key
            });
            let grant = if let Some(same) = same {
                same.grant.clone()
            } else {
                state.revision = state
                    .revision
                    .checked_add(1)
                    .filter(|v| *v <= MAX_SAFE_INTEGER)
                    .ok_or_else(|| AcquireError::from(UNAVAILABLE.to_string()))?;
                Grant {
                    owner,
                    document_epoch: epoch,
                    revision: state.revision,
                    window_key: key.clone(),
                }
            };
            (grant, previous)
        };
        if Self::set_confirmed(window, true).is_err() {
            // A dispatched OS operation may have changed state despite a failed
            // confirmation. Restore the prior presentation before reporting.
            return Err(self.failed_enter(window, &key, grant, previous, on_acquired, on_lost));
        }
        if !self.bound(&key) {
            return Err(String::from(INVALIDATED).into());
        }
        if !valid(&validity) {
            if previous.is_some() {
                return Err(String::from(INVALIDATED).into());
            }
            if Self::set_confirmed(window, false).is_ok() {
                return Err(String::from(INVALIDATED).into());
            }
            return Err(self.retain_failure(grant, on_acquired, on_lost));
        }
        let installed = Owner {
            grant: grant.clone(),
            on_lost,
        };
        let replaced = self.install(installed).map_err(AcquireError::from)?;
        let old_notified = replaced
            .filter(|v| v.grant != grant)
            .is_none_or(|v| notify(&v.on_lost, v.grant));
        let acquired_notified = notify(&on_acquired, grant.clone());
        // Session close may race either native entry or the local-store callback.
        // It must observe the installed grant, or this check revokes it here.
        if !acquired_notified || !valid(&validity) {
            let error = if self.release_inner(window, &grant).is_ok() {
                AcquireError::from(INVALIDATED.to_string())
            } else {
                AcquireError {
                    message: NATIVE_FAILED.into(),
                    retained_grant: Some(grant),
                }
            };
            return Err(error);
        }
        if !self.current(&grant) {
            return Err(String::from(INVALIDATED).into());
        }
        if !old_notified {
            return Err(AcquireError {
                message: "이전 전체화면 표시를 정리하지 못했습니다.".into(),
                retained_grant: Some(grant),
            });
        }
        Ok(grant)
    }

    fn failed_enter(
        &self,
        window: &impl PresentationWindow,
        key: &str,
        grant: Grant,
        previous: Option<Owner>,
        on_acquired: OnAcquired,
        on_lost: OnLost,
    ) -> AcquireError {
        if !self.bound(key) {
            return INVALIDATED.to_string().into();
        }
        // Never hand an existing, different owner's capability to this caller.
        if previous.is_some() {
            let _ = Self::set_confirmed(window, true);
            return NATIVE_FAILED.to_string().into();
        }
        if Self::set_confirmed(window, false).is_ok() {
            return NATIVE_FAILED.to_string().into();
        }
        self.retain_failure(grant, on_acquired, on_lost)
    }

    fn retain_failure(
        &self,
        grant: Grant,
        on_acquired: OnAcquired,
        on_lost: OnLost,
    ) -> AcquireError {
        if self
            .install(Owner {
                grant: grant.clone(),
                on_lost,
            })
            .is_err()
        {
            return INVALIDATED.to_string().into();
        }
        notify(&on_acquired, grant.clone());
        AcquireError {
            message: NATIVE_FAILED.into(),
            retained_grant: Some(grant),
        }
    }

    fn release(
        self: &Arc<Self>,
        window: &impl PresentationWindow,
        grant: &Grant,
    ) -> Result<bool, String> {
        let _operation = self.enter()?;
        self.bind(window)?;
        self.release_inner(window, grant)
    }

    fn release_inner(
        &self,
        window: &impl PresentationWindow,
        grant: &Grant,
    ) -> Result<bool, String> {
        if !self.current(grant) {
            return Ok(false);
        }
        Self::set_confirmed(window, false)?;
        let previous = {
            let mut state = self.machine.lock().map_err(|_| UNAVAILABLE)?;
            if !state.owner.as_ref().is_some_and(|v| v.grant == *grant) {
                return Ok(false);
            }
            state.owner.take()
        };
        if let Some(previous) = previous {
            notify(&previous.on_lost, previous.grant);
        }
        Ok(true)
    }
}

fn managed(app: &AppHandle) -> Arc<Coordinator> {
    if let Some(state) = app.try_state::<ManagedCoordinator>() {
        return state.inner().0.clone();
    }
    app.manage(ManagedCoordinator(Arc::new(Coordinator::default())));
    app.state::<ManagedCoordinator>().inner().0.clone()
}

/// Callbacks store/revoke the source-local exact grant inside this operation;
/// storing only after this function returns can resurrect a superseded grant.
pub fn acquire(
    app: &AppHandle,
    owner: String,
    document_epoch: u64,
    validity: Validity,
    on_acquired: OnAcquired,
    on_lost: OnLost,
) -> Result<Grant, AcquireError> {
    let coordinator = managed(app);
    dispatch(app, move |window| {
        coordinator.acquire(
            &window,
            owner,
            document_epoch,
            validity,
            on_acquired,
            on_lost,
        )
    })
    .map_err(AcquireError::from)?
}

pub fn release(app: &AppHandle, grant: Grant) -> Result<bool, String> {
    let coordinator = managed(app);
    dispatch(app, move |window| coordinator.release(&window, &grant))?
}

pub fn is_current(app: &AppHandle, grant: &Grant) -> bool {
    app.try_state::<ManagedCoordinator>()
        .is_some_and(|state| state.inner().0.current(grant))
}

const PENDING: u8 = 0;
const RUNNING: u8 = 1;
const CANCELLED: u8 = 2;
const FINISHED: u8 = 3;
#[derive(Default)]
struct Ticket(AtomicU8);
impl Ticket {
    fn start(&self) -> bool {
        self.0
            .compare_exchange(PENDING, RUNNING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    fn cancel_queued(&self) -> bool {
        self.0
            .compare_exchange(PENDING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    fn finish(&self) {
        self.0.store(FINISHED, Ordering::Release);
    }
}

fn wait_for_result<T>(
    ticket: &Ticket,
    receiver: Receiver<Result<T, String>>,
    timeout: Duration,
) -> Result<T, String> {
    match receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(RecvTimeoutError::Disconnected) => Err(UNAVAILABLE.into()),
        Err(RecvTimeoutError::Timeout) if ticket.cancel_queued() => Err(QUEUE_TIMEOUT.into()),
        // The operation already began. Its synchronous native call has no hard
        // runtime deadline: wait for truth, never return an orphan late success.
        Err(RecvTimeoutError::Timeout) => receiver.recv().map_err(|_| UNAVAILABLE.to_string())?,
    }
}

#[cfg(windows)]
fn window_thread(window: &Window) -> Result<(usize, bool), String> {
    use windows::Win32::{
        Foundation::HWND, System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::GetWindowThreadProcessId,
    };
    let id = window.hwnd().map_err(|_| UNAVAILABLE)?.0 as usize;
    let owner_thread = unsafe { GetWindowThreadProcessId(HWND(id as *mut std::ffi::c_void), None) };
    if id == 0 || owner_thread == 0 {
        return Err(UNAVAILABLE.into());
    }
    Ok((id, owner_thread == unsafe { GetCurrentThreadId() }))
}
#[cfg(not(windows))]
fn window_thread(_window: &Window) -> Result<(usize, bool), String> {
    // The supported desktop target is Windows. Do not queue+wait with unknown
    // UI-thread identity on another platform.
    Err("이 환경의 전체화면 전환은 아직 지원되지 않습니다.".into())
}

fn dispatch<T: Send + 'static>(
    app: &AppHandle,
    action: impl FnOnce(NativeWindow) -> T + Send + 'static,
) -> Result<T, String> {
    if IN_CALLBACK.with(Cell::get) {
        return Err(REENTRANT.into());
    }
    let window = app.get_window("main").ok_or(UNAVAILABLE)?;
    let (native_id, inline) = window_thread(&window)?;
    if inline {
        return catch_unwind(AssertUnwindSafe(|| {
            action(NativeWindow { window, native_id })
        }))
        .map_err(|_| NATIVE_FAILED.into());
    }
    let ticket = Arc::new(Ticket::default());
    let queued_ticket = ticket.clone();
    let (sender, receiver) = mpsc::channel();
    let queued_app = app.clone();
    let queued = app.run_on_main_thread(move || {
        if !queued_ticket.start() {
            return;
        }
        // Resolve again after queueing. A closed/recreated main window must not
        // let an old Window handle rebind and clear its replacement's owner.
        let current = window_thread(&window)
            .and_then(|(original_id, original_inline)| {
                if !original_inline || original_id != native_id {
                    return Err(INVALIDATED.into());
                }
                queued_app
                    .get_window("main")
                    .ok_or_else(|| UNAVAILABLE.to_string())
            })
            .and_then(|window| {
                window_thread(&window).and_then(|(id, inline)| {
                    if !inline || id != native_id {
                        return Err(INVALIDATED.into());
                    }
                    Ok(NativeWindow {
                        window,
                        native_id: id,
                    })
                })
            });
        let result = current.and_then(|native| {
            catch_unwind(AssertUnwindSafe(|| action(native))).map_err(|_| NATIVE_FAILED.to_string())
        });
        let _ = sender.send(result);
        queued_ticket.finish();
    });
    if queued.is_err() && ticket.cancel_queued() {
        return Err(UNAVAILABLE.into());
    }
    wait_for_result(&ticket, receiver, QUEUED_WAIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct FakeWindow {
        id: usize,
        full: AtomicBool,
        fail_enter: AtomicBool,
        fail_exit: AtomicBool,
        fail_read: AtomicBool,
        writes: AtomicUsize,
        destroyed: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
    }
    impl FakeWindow {
        fn new(id: usize) -> Self {
            Self {
                id,
                full: AtomicBool::new(false),
                fail_enter: AtomicBool::new(false),
                fail_exit: AtomicBool::new(false),
                fail_read: AtomicBool::new(false),
                writes: AtomicUsize::new(0),
                destroyed: Mutex::new(Vec::new()),
            }
        }
        fn destroy(&self) {
            let callbacks = self.destroyed.lock().unwrap().clone();
            for callback in callbacks {
                callback();
            }
        }
    }
    impl PresentationWindow for FakeWindow {
        fn native_id(&self) -> usize {
            self.id
        }
        fn fullscreen(&self) -> Result<bool, String> {
            if self.fail_read.load(Ordering::Acquire) {
                return Err(NATIVE_FAILED.into());
            }
            Ok(self.full.load(Ordering::Acquire))
        }
        fn set_fullscreen(&self, enabled: bool) -> Result<(), String> {
            self.writes.fetch_add(1, Ordering::AcqRel);
            if (if enabled {
                &self.fail_enter
            } else {
                &self.fail_exit
            })
            .load(Ordering::Acquire)
            {
                return Err(NATIVE_FAILED.into());
            }
            self.full.store(enabled, Ordering::Release);
            Ok(())
        }
        fn on_destroyed(&self, callback: Arc<dyn Fn() + Send + Sync>) {
            self.destroyed.lock().unwrap().push(callback);
        }
    }
    fn acquire_for(
        coordinator: &Arc<Coordinator>,
        window: &FakeWindow,
        owner: &str,
        epoch: u64,
        lost: Arc<AtomicUsize>,
    ) -> Grant {
        coordinator
            .acquire(
                window,
                owner.into(),
                epoch,
                Arc::new(|| true),
                Arc::new(|_| {}),
                Arc::new(move |_| {
                    lost.fetch_add(1, Ordering::AcqRel);
                }),
            )
            .unwrap()
    }
    fn counter() -> Arc<AtomicUsize> {
        Arc::new(AtomicUsize::new(0))
    }

    #[test]
    fn supersession_and_stale_release_never_exit_new_owner() {
        let coordinator = Arc::new(Coordinator::default());
        let window = FakeWindow::new(1);
        let lost_a = counter();
        let a = acquire_for(&coordinator, &window, "a", 1, lost_a.clone());
        let b = acquire_for(&coordinator, &window, "b", 1, counter());
        assert_eq!(lost_a.load(Ordering::Acquire), 1);
        assert!(coordinator.current(&b));
        assert!(!coordinator.release(&window, &a).unwrap());
        assert!(window.full.load(Ordering::Acquire));
        assert_eq!(window.writes.load(Ordering::Acquire), 1);
        assert!(coordinator.release(&window, &b).unwrap());
        assert!(!coordinator.release(&window, &b).unwrap());
    }

    #[test]
    fn same_epoch_is_idempotent_but_new_epoch_fences_old_grant() {
        let coordinator = Arc::new(Coordinator::default());
        let window = FakeWindow::new(1);
        let a = acquire_for(&coordinator, &window, "a", 1, counter());
        let again = acquire_for(&coordinator, &window, "a", 1, counter());
        assert_eq!(a, again);
        let next = acquire_for(&coordinator, &window, "a", 2, counter());
        assert_ne!(a, next);
        assert!(!coordinator.release(&window, &a).unwrap());
        assert!(coordinator.current(&next));
    }

    #[test]
    fn native_failures_preserve_retryable_current_ownership() {
        let coordinator = Arc::new(Coordinator::default());
        let window = FakeWindow::new(1);
        window.fail_enter.store(true, Ordering::Release);
        assert!(coordinator
            .acquire(
                &window,
                "a".into(),
                1,
                Arc::new(|| true),
                Arc::new(|_| {}),
                Arc::new(|_| {})
            )
            .is_err());
        assert!(coordinator.machine.lock().unwrap().owner.is_none());
        window.fail_enter.store(false, Ordering::Release);
        let lost = counter();
        let a = acquire_for(&coordinator, &window, "a", 1, lost.clone());
        window.fail_exit.store(true, Ordering::Release);
        assert!(coordinator.release(&window, &a).is_err());
        assert!(coordinator.current(&a));
        assert_eq!(lost.load(Ordering::Acquire), 0);
        window.fail_exit.store(false, Ordering::Release);
        assert!(coordinator.release(&window, &a).unwrap());
        assert_eq!(lost.load(Ordering::Acquire), 1);
    }

    #[test]
    fn forged_grant_and_destroyed_window_cannot_release_replacement() {
        let coordinator = Arc::new(Coordinator::default());
        let first = FakeWindow::new(1);
        let a = acquire_for(&coordinator, &first, "a", 1, counter());
        let mut forged = a.clone();
        forged.revision += 1;
        assert!(!coordinator.release(&first, &forged).unwrap());
        first.destroy();
        let second = FakeWindow::new(1); // HWND may be reused after Destroyed.
        let b = acquire_for(&coordinator, &second, "a", 1, counter());
        assert_ne!(a.window_key, b.window_key);
        first.destroy(); // An old incarnation's repeated callback is harmless.
        assert!(coordinator.current(&b));
        assert!(!coordinator.release(&second, &a).unwrap());
        assert!(second.full.load(Ordering::Acquire));
    }

    #[test]
    fn callbacks_run_outside_lock_and_cannot_reenter_mutation() {
        let coordinator = Arc::new(Coordinator::default());
        let window = Arc::new(FakeWindow::new(1));
        let seen = counter();
        let callback_coordinator = coordinator.clone();
        let callback_window = window.clone();
        let callback_seen = seen.clone();
        let a = coordinator
            .acquire(
                window.as_ref(),
                "a".into(),
                1,
                Arc::new(|| true),
                Arc::new(move |grant| {
                    assert!(callback_coordinator.machine.try_lock().is_ok());
                    assert!(callback_coordinator.current(&grant));
                    assert_eq!(
                        callback_coordinator
                            .release(callback_window.as_ref(), &grant)
                            .unwrap_err(),
                        REENTRANT
                    );
                    callback_seen.fetch_add(1, Ordering::AcqRel);
                }),
                Arc::new(|_| {}),
            )
            .unwrap();
        assert_eq!(seen.load(Ordering::Acquire), 1);
        assert!(coordinator.current(&a));
    }

    #[test]
    fn acquired_callback_precedes_later_supersession_without_late_store() {
        let coordinator = Arc::new(Coordinator::default());
        let window = FakeWindow::new(1);
        let local = Arc::new(Mutex::new(None::<Grant>));
        let store = local.clone();
        let clear = local.clone();
        let a = coordinator
            .acquire(
                &window,
                "a".into(),
                1,
                Arc::new(|| true),
                Arc::new(move |g| {
                    *store.lock().unwrap() = Some(g);
                }),
                Arc::new(move |g| {
                    let mut current = clear.lock().unwrap();
                    if current.as_ref() == Some(&g) {
                        *current = None;
                    }
                }),
            )
            .unwrap();
        assert_eq!(*local.lock().unwrap(), Some(a));
        acquire_for(&coordinator, &window, "b", 1, counter());
        assert!(local.lock().unwrap().is_none());
    }

    #[test]
    fn close_during_commit_revokes_or_returns_explicit_retained_capability() {
        for failed_exit in [false, true] {
            let coordinator = Arc::new(Coordinator::default());
            let window = FakeWindow::new(1);
            window.fail_exit.store(failed_exit, Ordering::Release);
            let alive = Arc::new(AtomicBool::new(true));
            let check = alive.clone();
            let close = alive.clone();
            let error = coordinator
                .acquire(
                    &window,
                    "a".into(),
                    1,
                    Arc::new(move || check.load(Ordering::Acquire)),
                    Arc::new(move |_| close.store(false, Ordering::Release)),
                    Arc::new(|_| {}),
                )
                .unwrap_err();
            assert_eq!(error.retained_grant.is_some(), failed_exit);
            if let Some(grant) = error.retained_grant {
                assert!(coordinator.current(&grant));
                window.fail_exit.store(false, Ordering::Release);
                assert!(coordinator.release(&window, &grant).unwrap());
            } else {
                assert!(!window.full.load(Ordering::Acquire));
            }
        }
    }

    #[test]
    fn invalid_intent_never_mutates_and_new_failure_does_not_expose_previous_owner() {
        let coordinator = Arc::new(Coordinator::default());
        let window = FakeWindow::new(1);
        assert!(coordinator
            .acquire(
                &window,
                "a".into(),
                1,
                Arc::new(|| false),
                Arc::new(|_| {}),
                Arc::new(|_| {})
            )
            .is_err());
        assert_eq!(window.writes.load(Ordering::Acquire), 0);
        let a = acquire_for(&coordinator, &window, "a", 1, counter());
        let checks = counter();
        let error = coordinator
            .acquire(
                &window,
                "b".into(),
                1,
                Arc::new(move || checks.fetch_add(1, Ordering::AcqRel) == 0),
                Arc::new(|_| {}),
                Arc::new(|_| {}),
            )
            .unwrap_err();
        assert!(error.retained_grant.is_none());
        assert!(coordinator.current(&a));
    }

    #[test]
    fn queued_timeout_cancels_before_any_late_side_effect() {
        let ticket = Ticket::default();
        let (_sender, receiver) = mpsc::channel::<Result<(), String>>();
        assert_eq!(
            wait_for_result(&ticket, receiver, Duration::from_millis(1)).unwrap_err(),
            QUEUE_TIMEOUT
        );
        assert!(!ticket.start());
        assert_eq!(ticket.0.load(Ordering::Acquire), CANCELLED);
    }

    #[test]
    fn already_running_waits_for_definitive_result_not_timeout() {
        let ticket = Arc::new(Ticket::default());
        assert!(ticket.start());
        let worker_ticket = ticket.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(15));
            sender.send(Ok(73)).unwrap();
            worker_ticket.finish();
        });
        assert_eq!(
            wait_for_result(&ticket, receiver, Duration::from_millis(1)).unwrap(),
            73
        );
        worker.join().unwrap();
        assert_eq!(ticket.0.load(Ordering::Acquire), FINISHED);
    }

    #[test]
    fn ticket_start_and_cancel_have_exactly_one_winner() {
        for _ in 0..64 {
            let ticket = Arc::new(Ticket::default());
            let other = ticket.clone();
            let worker = std::thread::spawn(move || other.start());
            let cancelled = ticket.cancel_queued();
            let started = worker.join().unwrap();
            assert_ne!(cancelled, started);
        }
    }

    #[test]
    fn grant_serialization_is_capability_only_and_rejects_unknown_fields() {
        let grant = Grant {
            owner: "opaque-id".into(),
            document_epoch: 2,
            revision: 3,
            window_key: "incarnation".into(),
        };
        let mut value = serde_json::to_value(&grant).unwrap();
        assert_eq!(value["documentEpoch"], 2);
        assert!(value.get("document_epoch").is_none());
        value["nativeHandle"] = serde_json::json!(1234);
        assert!(serde_json::from_value::<Grant>(value).is_err());
    }

    #[test]
    fn unavailable_readback_has_explicit_retry_capability_not_success() {
        let coordinator = Arc::new(Coordinator::default());
        let window = FakeWindow::new(1);
        window.full.store(true, Ordering::Release);
        window.fail_read.store(true, Ordering::Release);
        let local = Arc::new(Mutex::new(None::<Grant>));
        let store = local.clone();
        let error = coordinator
            .acquire(
                &window,
                "a".into(),
                1,
                Arc::new(|| true),
                Arc::new(move |g| {
                    *store.lock().unwrap() = Some(g);
                }),
                Arc::new(|_| {}),
            )
            .unwrap_err();
        let grant = error
            .retained_grant
            .expect("unconfirmed rollback retains an exact capability");
        assert_eq!(*local.lock().unwrap(), Some(grant.clone()));
        assert!(coordinator.current(&grant));
        window.fail_read.store(false, Ordering::Release);
        assert!(coordinator.release(&window, &grant).unwrap());
        assert!(!window.full.load(Ordering::Acquire));
    }

    #[test]
    fn destruction_during_callback_never_returns_live_grant() {
        let coordinator = Arc::new(Coordinator::default());
        let window = Arc::new(FakeWindow::new(1));
        let destroy = window.clone();
        let losses = counter();
        let lost = losses.clone();
        let error = coordinator
            .acquire(
                window.as_ref(),
                "a".into(),
                1,
                Arc::new(|| true),
                Arc::new(move |_| destroy.destroy()),
                Arc::new(move |_| {
                    lost.fetch_add(1, Ordering::AcqRel);
                }),
            )
            .unwrap_err();
        assert!(error.retained_grant.is_none());
        assert!(coordinator.machine.lock().unwrap().owner.is_none());
        assert_eq!(losses.load(Ordering::Acquire), 1);
    }

    #[test]
    fn acquired_callback_panic_revokes_and_does_not_poison_owner_lock() {
        let coordinator = Arc::new(Coordinator::default());
        let window = FakeWindow::new(1);
        assert!(coordinator
            .acquire(
                &window,
                "a".into(),
                1,
                Arc::new(|| true),
                Arc::new(|_| panic!("test callback failure")),
                Arc::new(|_| {})
            )
            .is_err());
        assert!(!window.full.load(Ordering::Acquire));
        assert!(coordinator.machine.lock().unwrap().owner.is_none());
        let next = acquire_for(&coordinator, &window, "b", 1, counter());
        assert!(coordinator.current(&next));
    }
}
