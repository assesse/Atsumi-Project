//! One owned SOOP VOD mouse press, not a general input grace period.
use super::Session;
use crate::{
    platform_native_geometry::{InputGate, NativeViewport, Rect},
    platform_window_fullscreen::Grant,
};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const MAX_AGE: Duration = Duration::from_millis(500);
/// Diagnostics only: these values never participate in permission decisions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Reason {
    #[default]
    NoEvent,
    Observing,
    Ready,
    BindingUnavailable,
    NativeWindow,
    NativePointer,
    ForwardBinding,
    ForwardWindow,
    ForwardFocus,
    ForwardPointer,
    ForwardDpi,
    Invalidated,
    Expired,
    BindingMismatch,
    TransitionMismatch,
    Missing,
    StateBusy,
    ConsumeWindow,
    ConsumeFocus,
    ConsumePointer,
    ConsumeDpi,
    Accepted,
}
impl Reason {
    pub(super) fn diagnostic(self) -> &'static str {
        match self {
            Self::NoEvent => "no_native_event",
            Self::BindingUnavailable => "native_binding_unavailable",
            Self::NativeWindow => "native_window_unavailable",
            Self::NativePointer => "native_pointer_outside",
            Self::ForwardBinding => "forward_binding_changed",
            Self::ForwardWindow => "forward_window_unavailable",
            Self::ForwardFocus => "forward_focus_mismatch",
            Self::ForwardPointer => "forward_pointer_outside",
            Self::ForwardDpi => "forward_dpi_changed",
            Self::Invalidated => "native_invalidated",
            Self::Expired => "receipt_expired",
            Self::BindingMismatch => "receipt_binding_mismatch",
            Self::TransitionMismatch => "receipt_transition_mismatch",
            Self::Missing | Self::Observing | Self::Ready => "receipt_missing",
            Self::StateBusy => "receipt_state_busy",
            Self::ConsumeWindow => "consume_window_unavailable",
            Self::ConsumeFocus => "consume_focus_mismatch",
            Self::ConsumePointer => "consume_pointer_outside",
            Self::ConsumeDpi => "consume_dpi_changed",
            Self::Accepted => "receipt_accepted",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
struct Geometry {
    player: Rect,
    chat: Option<Rect>,
    holes: Vec<[f64; 8]>,
}
impl Geometry {
    fn from_viewport(v: &NativeViewport) -> Option<Self> {
        if !v.visible || v.validate().is_err() {
            return None;
        }
        let player = v.player.clone()?;
        if player.width <= 0. || player.height <= 0. {
            return None;
        }
        Some(Self {
            player,
            chat: v.chat.clone(),
            holes: v
                .occlusions
                .iter()
                .map(|h| {
                    [
                        h.x, h.y, h.width, h.height, h.radii[0], h.radii[1], h.radii[2], h.radii[3],
                    ]
                })
                .collect(),
        })
    }
    fn contains_player(&self, x: f64, y: f64) -> bool {
        let inside = |r: &Rect| x >= r.x && y >= r.y && x < r.x + r.width && y < r.y + r.height;
        x.is_finite() && y.is_finite() && inside(&self.player)
            && !self.chat.as_ref().is_some_and(inside)
            // Conservatively reject the entire overlay rectangle, including its
            // rounded corners. WindowFromPoint additionally checks the real region.
            && !self.holes.iter().any(|h| inside(&Rect{x:h[0],y:h[1],width:h[2],height:h[3]}))
    }
    fn contains_physical(&self, x: i32, y: i32, dpi: u32) -> bool {
        if !(24..=768).contains(&dpi) {
            return false;
        }
        let scale = f64::from(dpi) / 96.;
        self.contains_player(f64::from(x) / scale, f64::from(y) / scale)
    }
}
pub(super) fn same_geometry(a: Option<&NativeViewport>, b: Option<&NativeViewport>) -> bool {
    a.and_then(Geometry::from_viewport) == b.and_then(Geometry::from_viewport)
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Binding {
    pub epoch: u64,
    pub presenter: String,
    pub grant: Option<Grant>,
    pub generation: u64,
    geometry: Geometry,
}
impl Binding {
    pub(super) fn enabled(&self) -> bool {
        self.grant.is_none()
    }
    pub(super) fn after_acquire(&self, current: &Self, accepted: &(Grant, u64)) -> bool {
        current.epoch == self.epoch
            && current.presenter == self.presenter
            && current.generation == accepted.1
            && current.grant.as_ref() == Some(&accepted.0)
            && accepted.0.owner == self.presenter
            && accepted.0.document_epoch == self.epoch
    }
}

/// Never wait in a native window procedure and never hold session locks while
/// forwarding it. Generation brackets the independently locked source fields.
pub(super) fn current(session: &Session) -> Option<Binding> {
    let generation = session.mouse_intent.generation();
    if session.closed.load(Ordering::Acquire) {
        return None;
    }
    let epoch = session.epoch.load(Ordering::Acquire);
    if epoch == 0 {
        return None;
    }
    let viewers = session.viewers.try_lock().ok()?;
    let presenter = viewers.last()?.clone();
    let grant = session.fullscreen_grant.try_lock().ok()?.clone();
    if grant
        .as_ref()
        .is_some_and(|g| g.owner != presenter || g.document_epoch != epoch)
    {
        return None;
    }
    let geometry = Geometry::from_viewport(session.viewport.try_lock().ok()?.as_ref()?)?;
    if generation != session.mouse_intent.generation()
        || session.closed.load(Ordering::Acquire)
        || epoch != session.epoch.load(Ordering::Acquire)
    {
        return None;
    }
    Some(Binding {
        epoch,
        presenter,
        grant,
        generation,
        geometry,
    })
}
struct Receipt {
    binding: Binding,
    created: Instant,
    dpi: u32,
}
#[derive(Default)]
struct Pending {
    dispatch: u64,
    committed: bool,
    receipt: Option<Receipt>,
    reason: Reason,
    observed_at: Option<Instant>,
}
#[derive(Default)]
pub(super) struct Intent {
    generation: AtomicU64,
    native_seen: AtomicBool,
    active: AtomicBool,
    pending: Mutex<Pending>,
}
/// A native callback lifetime, not a physical-click identifier. Keep this alive
/// through DefSubclassProc; neither consumption nor invalidation rearms it.
struct Dispatch<'a> {
    intent: &'a Intent,
    id: (u64, u64),
}
impl Drop for Dispatch<'_> {
    fn drop(&mut self) {
        self.intent.active.store(false, Ordering::Release);
    }
}
impl Intent {
    pub(super) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    pub(super) fn clear(&self) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        // An in-flight holder observes generation mismatch; never block the UI.
        if let Ok(mut pending) = self.pending.try_lock() {
            if pending.receipt.is_some()
                || matches!(pending.reason, Reason::Observing | Reason::Ready)
            {
                pending.reason = Reason::Invalidated;
            }
            pending.receipt = None;
        }
        generation
    }
    fn begin_at(&self, now: Instant) -> Option<Dispatch<'_>> {
        self.native_seen.store(true, Ordering::Relaxed);
        self.active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        let mut dispatch = Dispatch {
            intent: self,
            id: (0, 0),
        };
        let mut p = self.pending.try_lock().ok()?;
        p.observed_at = Some(now);
        p.receipt = None;
        p.reason = Reason::Observing;
        p.dispatch = p.dispatch.checked_add(1)?;
        p.committed = false;
        dispatch.id = (p.dispatch, self.generation());
        Some(dispatch)
    }
    fn finish(&self, dispatch: &Dispatch<'_>, binding: Binding, dpi: u32, created: Instant) {
        let Ok(mut p) = self.pending.try_lock() else {
            return;
        };
        if std::ptr::eq(dispatch.intent, self)
            && p.dispatch == dispatch.id.0
            && !p.committed
            && self.generation() == dispatch.id.1
            && binding.generation == dispatch.id.1
        {
            p.committed = true;
            p.receipt = Some(Receipt {
                binding,
                created,
                dpi,
            });
            p.reason = Reason::Ready;
        }
    }
    fn note(&self, dispatch: &Dispatch<'_>, reason: Reason) {
        if let Ok(mut p) = self.pending.try_lock() {
            if std::ptr::eq(dispatch.intent, self)
                && p.dispatch == dispatch.id.0
                && self.generation() == dispatch.id.1
            {
                p.reason = reason;
            }
        }
    }
    /// Consume on any canonical mouse attempt, including a malformed transition.
    #[cfg(test)]
    pub(super) fn take(
        &self,
        binding: Option<&Binding>,
        enabled: Option<bool>,
        now: Instant,
    ) -> Option<(Binding, u32)> {
        self.take_observed(binding, enabled, now).0
    }
    fn take_observed(
        &self,
        binding: Option<&Binding>,
        enabled: Option<bool>,
        now: Instant,
    ) -> (Option<(Binding, u32)>, Reason) {
        let Ok(mut p) = self.pending.try_lock() else {
            return (None, Reason::StateBusy);
        };
        let Some(receipt) = p.receipt.take() else {
            let reason = if self.native_seen.load(Ordering::Relaxed)
                && (p.reason == Reason::NoEvent
                    || p.observed_at.is_none_or(|at| {
                        now.checked_duration_since(at)
                            .is_none_or(|age| age > MAX_AGE)
                    })) {
                Reason::Missing
            } else {
                p.reason
            };
            return (None, reason);
        };
        p.reason = Reason::Missing;
        let Some(binding) = binding else {
            return (None, Reason::BindingUnavailable);
        };
        if receipt.binding != *binding || binding.generation != self.generation() {
            return (None, Reason::BindingMismatch);
        }
        if enabled != Some(binding.enabled()) {
            return (None, Reason::TransitionMismatch);
        }
        if !now
            .checked_duration_since(receipt.created)
            .is_some_and(|age| age <= MAX_AGE)
        {
            return (None, Reason::Expired);
        }
        (Some((receipt.binding, receipt.dpi)), Reason::Accepted)
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{cell::Cell, rc::Rc, sync::Weak};
    use webview2_com::{
        FocusChangedEventHandler, Microsoft::Web::WebView2::Win32::ICoreWebView2Controller,
    };
    use windows::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        Graphics::Gdi::ScreenToClient,
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::IsWindowEnabled,
            Shell::{DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                GetCursorPos, GetForegroundWindow, GetGUIThreadInfo, GetParent, IsChild,
                IsWindowVisible, WindowFromPoint, GUITHREADINFO, WM_KILLFOCUS, WM_LBUTTONDOWN,
                WM_NCDESTROY, WM_PARENTNOTIFY, WM_SHOWWINDOW,
            },
        },
    };
    const SUBCLASS_ID: usize = 0x4153534d; // ASSM: this receiver only.
    struct State {
        root: HWND,
        pane: HWND,
        session: Weak<Session>,
        intent: Arc<Intent>,
        closed: Cell<bool>,
    }
    fn down(message: u32, wparam: usize) -> bool {
        message == WM_PARENTNOTIFY && (wparam & 0xffff) as u32 == WM_LBUTTONDOWN
    }
    /// Only a canonical owned native down notification creates evidence. This
    /// context cannot independently authorize a JavaScript message.
    pub(super) unsafe fn context(
        root: usize,
        pane: usize,
        binding: &Binding,
        focus: bool,
    ) -> Result<u32, InputGate> {
        if root == 0 || pane == 0 || root == pane {
            return Err(InputGate::InvalidInput);
        }
        let root = HWND(root as *mut std::ffi::c_void);
        let pane = HWND(pane as *mut std::ffi::c_void);
        if GetForegroundWindow() != root
            || GetParent(pane).ok() != Some(root)
            || !IsWindowVisible(pane).as_bool()
            || !IsWindowVisible(root).as_bool()
            || !IsWindowEnabled(pane).as_bool()
            || !IsWindowEnabled(root).as_bool()
        {
            return Err(InputGate::WindowUnavailable);
        }
        if focus {
            let mut info = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            if GetGUIThreadInfo(0, &mut info).is_err()
                || !(info.hwndFocus == pane || IsChild(pane, info.hwndFocus).as_bool())
            {
                return Err(InputGate::FocusMismatch);
            }
        }
        let mut point = POINT::default();
        if GetCursorPos(&mut point).is_err() {
            return Err(InputGate::PointerOutside);
        }
        let hit = WindowFromPoint(point);
        if hit != pane && !IsChild(pane, hit).as_bool() {
            return Err(InputGate::PointerOutside);
        }
        let dpi = GetDpiForWindow(root);
        if !(24..=768).contains(&dpi) || !ScreenToClient(root, &mut point).as_bool() {
            return Err(InputGate::WindowUnavailable);
        }
        if !binding.geometry.contains_physical(point.x, point.y, dpi) {
            return Err(InputGate::PointerOutside);
        }
        Ok(dpi)
    }
    unsafe extern "system" fn pane_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        if message == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(pane_proc), id);
            let state = Rc::from_raw(data as *const State);
            state.closed.set(true);
            state.intent.clear();
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }
        Rc::increment_strong_count(data as *const State);
        let state = Rc::from_raw(data as *const State);
        if message == WM_KILLFOCUS || message == WM_SHOWWINDOW && wparam.0 == 0 {
            state.intent.clear();
        }
        // WM_PARENTNOTIFY/LEFTDOWN is the native event evidence; the button
        // may already be released. The host queue's last message timestamp is
        // unrelated to this sent notification. No page message mints a receipt.
        let created = Instant::now();
        let dispatch = if down(message, wparam.0) && !state.closed.get() {
            state.intent.begin_at(created)
        } else {
            None
        };
        let observed = dispatch.as_ref().and_then(|dispatch| {
            let Some(binding) = state.session.upgrade().and_then(|s| current(&s)) else {
                state.intent.note(dispatch, Reason::BindingUnavailable);
                return None;
            };
            match context(
                state.root.0 as usize,
                state.pane.0 as usize,
                &binding,
                false,
            ) {
                Ok(dpi) => Some((binding, dpi)),
                Err(gate) => {
                    state.intent.note(
                        dispatch,
                        if gate == InputGate::PointerOutside {
                            Reason::NativePointer
                        } else {
                            Reason::NativeWindow
                        },
                    );
                    None
                }
            }
        });
        // No mutex/RefCell borrow survives into the next subclass; reentrancy
        // can destroy the pane or reserve a newer press and must win.
        let result = DefSubclassProc(hwnd, message, wparam, lparam);
        if let (Some(dispatch), Some((binding, dpi))) = (dispatch.as_ref(), observed) {
            if state.closed.get()
                || state.session.upgrade().and_then(|s| current(&s)).as_ref() != Some(&binding)
            {
                state.intent.note(dispatch, Reason::ForwardBinding);
            } else {
                match context(state.root.0 as usize, state.pane.0 as usize, &binding, true) {
                    Ok(current_dpi) if current_dpi == dpi => {
                        state.intent.finish(dispatch, binding, dpi, created)
                    }
                    Ok(_) => state.intent.note(dispatch, Reason::ForwardDpi),
                    Err(gate) => state.intent.note(
                        dispatch,
                        match gate {
                            InputGate::FocusMismatch => Reason::ForwardFocus,
                            InputGate::PointerOutside => Reason::ForwardPointer,
                            _ => Reason::ForwardWindow,
                        },
                    ),
                }
            }
        }
        result
    }
    pub(crate) unsafe fn install(
        controller: &ICoreWebView2Controller,
        root: usize,
        pane: usize,
        session: &Arc<Session>,
    ) -> windows::core::Result<()> {
        let root = HWND(root as *mut std::ffi::c_void);
        let pane = HWND(pane as *mut std::ffi::c_void);
        if pane == root || pane.0.is_null() || GetParent(pane).ok() != Some(root) {
            return Err(windows::core::Error::from_win32());
        }
        let mut existing = 0;
        if GetWindowSubclass(pane, Some(pane_proc), SUBCLASS_ID, Some(&mut existing)).as_bool() {
            return Err(windows::core::Error::from_win32());
        }
        let state = Rc::new(State {
            root,
            pane,
            session: Arc::downgrade(session),
            intent: session.mouse_intent.clone(),
            closed: Cell::new(false),
        });
        let raw = Rc::into_raw(state.clone());
        if !SetWindowSubclass(pane, Some(pane_proc), SUBCLASS_ID, raw as usize).as_bool() {
            drop(Rc::from_raw(raw));
            return Err(windows::core::Error::from_win32());
        }
        let lost = session.mouse_intent.clone();
        let mut token = 0;
        if let Err(error) = controller.add_LostFocus(
            &FocusChangedEventHandler::create(Box::new(move |_, _| {
                lost.clear();
                Ok(())
            })),
            &mut token,
        ) {
            state.closed.set(true);
            state.intent.clear();
            if RemoveWindowSubclass(pane, Some(pane_proc), SUBCLASS_ID).as_bool() {
                drop(Rc::from_raw(raw));
            }
            return Err(error);
        }
        Ok(())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn only_owned_parent_left_down_is_a_candidate() {
            assert!(!down(WM_LBUTTONDOWN, 0));
            assert!(down(WM_PARENTNOTIFY, WM_LBUTTONDOWN as usize));
            assert!(down(WM_PARENTNOTIFY, (123 << 16) | WM_LBUTTONDOWN as usize));
            for message in [
                0x200,
                0x202,
                0x100,
                0x101,
                WM_KILLFOCUS,
                WM_SHOWWINDOW,
                WM_NCDESTROY,
            ] {
                assert!(!down(message, 0));
                assert!(!down(WM_PARENTNOTIFY, message as usize));
            }
        }
    }
}
#[cfg(windows)]
pub(super) use native::install;
#[cfg(windows)]
pub(super) fn consume(
    session: &Session,
    root: usize,
    pane: usize,
    enabled: Option<bool>,
) -> (InputGate, Option<Binding>, Reason) {
    let binding = current(session);
    let (receipt, reason) =
        session
            .mouse_intent
            .take_observed(binding.as_ref(), enabled, Instant::now());
    let Some((binding, dpi)) = receipt else {
        return (InputGate::InvalidInput, None, reason);
    };
    match unsafe { native::context(root, pane, &binding, true) } {
        Ok(current_dpi) if current_dpi == dpi => {
            (InputGate::Accepted, Some(binding), Reason::Accepted)
        }
        Ok(_) => (InputGate::WindowUnavailable, None, Reason::ConsumeDpi),
        Err(gate) => (
            gate,
            None,
            match gate {
                InputGate::FocusMismatch => Reason::ConsumeFocus,
                InputGate::PointerOutside => Reason::ConsumePointer,
                _ => Reason::ConsumeWindow,
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding(intent: &Intent) -> Binding {
        Binding {
            epoch: 2,
            presenter: "viewer-a".into(),
            grant: None,
            generation: intent.generation(),
            geometry: Geometry {
                player: Rect {
                    x: 10.,
                    y: 10.,
                    width: 500.,
                    height: 300.,
                },
                chat: Some(Rect {
                    x: 510.,
                    y: 10.,
                    width: 200.,
                    height: 300.,
                }),
                holes: vec![],
            },
        }
    }
    fn mint(intent: &Intent, _stamp: i32, now: Instant) -> Binding {
        let b = binding(intent);
        let dispatch = intent.begin_at(now).unwrap();
        intent.finish(&dispatch, b.clone(), 96, now);
        b
    }
    #[test]
    fn one_receipt_and_nested_dispatch_or_second_commit_never_refills() {
        let i = Intent::default();
        let now = Instant::now();
        let b = binding(&i);
        let dispatch = i.begin_at(now).unwrap();
        i.finish(&dispatch, b.clone(), 96, now);
        assert!(i.take(Some(&b), Some(true), now).is_some());
        assert!(i.begin_at(now).is_none());
        i.finish(&dispatch, b.clone(), 96, now);
        assert!(i.take(Some(&b), Some(true), now).is_none());
        i.clear();
        assert!(i.begin_at(now).is_none());
        drop(dispatch);
        let b = mint(&i, 9, now);
        assert!(i.take(Some(&b), Some(true), now).is_some());
    }
    #[test]
    fn expiry_direction_and_failed_attempt_are_consuming() {
        let i = Intent::default();
        let now = Instant::now();
        for (stamp, enabled, at) in [
            (1, Some(true), now + MAX_AGE + Duration::from_nanos(1)),
            (2, Some(false), now),
            (3, None, now),
        ] {
            let b = mint(&i, stamp, now);
            assert!(i.take(Some(&b), enabled, at).is_none());
            assert!(i.take(Some(&b), Some(true), now).is_none());
        }
        let b = mint(&i, 4, now);
        assert!(i.take(Some(&b), Some(true), now + MAX_AGE).is_some());
    }
    #[test]
    fn reentrant_down_and_focus_hide_aba_cannot_mint() {
        let i = Intent::default();
        let now = Instant::now();
        let old = binding(&i);
        let outer = i.begin_at(now).unwrap();
        assert!(i.begin_at(now).is_none());
        i.clear();
        i.finish(&outer, old, 96, now);
        assert!(i.take(Some(&binding(&i)), Some(true), now).is_none());
        assert!(i.begin_at(now).is_none());
        drop(outer);
        let newer = mint(&i, 8, now);
        assert_eq!(i.take(Some(&newer), Some(true), now).unwrap().0, newer);
    }
    #[test]
    fn epoch_alias_grant_geometry_and_independent_receivers_fail_closed() {
        let i = Intent::default();
        let other = Intent::default();
        let now = Instant::now();
        for (n, change) in [0, 1, 2, 3].into_iter().enumerate() {
            let original = mint(&i, n as i32, now);
            let mut b = original.clone();
            match change {
                0 => b.epoch += 1,
                1 => b.presenter = "viewer-b".into(),
                2 => {
                    b.grant = Some(Grant {
                        owner: b.presenter.clone(),
                        document_epoch: 2,
                        revision: 3,
                        window_key: "root".into(),
                    })
                }
                _ => b.geometry.player.width += 1.,
            }
            assert!(i.take(Some(&b), Some(b.enabled()), now).is_none());
            assert!(i.take(Some(&original), Some(true), now).is_none());
        }
        let b = mint(&i, 10, now);
        assert!(other.take(Some(&b), Some(true), now).is_none());
        i.clear();
        assert!(i.take(Some(&b), Some(true), now).is_none());
    }
    #[test]
    fn player_only_geometry_rejects_chat_holes_and_nonfinite_points() {
        let mut g = binding(&Intent::default()).geometry;
        assert!(g.contains_player(20., 20.));
        assert!(!g.contains_player(520., 20.));
        assert!(!g.contains_player(f64::NAN, 20.));
        g.holes.push([15., 15., 50., 50., 10., 10., 10., 10.]);
        assert!(!g.contains_player(20., 20.));
    }
    #[test]
    fn physical_points_follow_current_main_dpi_without_accepting_chat() {
        let g = binding(&Intent::default()).geometry;
        for dpi in [96, 144, 192] {
            let scale = dpi as f64 / 96.;
            assert!(g.contains_physical((20. * scale) as i32, (20. * scale) as i32, dpi));
            assert!(!g.contains_physical((520. * scale) as i32, (20. * scale) as i32, dpi));
        }
        assert!(!g.contains_physical(20, 20, 0));
        assert!(!g.contains_physical(20, 20, 769));
    }
    #[test]
    fn exit_receipt_releases_only_original_grant() {
        let i = Intent::default();
        let now = Instant::now();
        let mut b = binding(&i);
        b.grant = Some(Grant {
            owner: b.presenter.clone(),
            document_epoch: b.epoch,
            revision: 9,
            window_key: "first-root".into(),
        });
        for field in 0..3 {
            let dispatch = i.begin_at(now).unwrap();
            i.finish(&dispatch, b.clone(), 96, now);
            let mut other = b.clone();
            let grant = other.grant.as_mut().unwrap();
            match field {
                0 => grant.revision += 1,
                1 => grant.window_key = "replacement-root".into(),
                _ => grant.owner = "replacement-alias".into(),
            }
            assert!(i.take(Some(&other), Some(false), now).is_none());
            assert!(i.take(Some(&b), Some(false), now).is_none());
        }
        let dispatch = i.begin_at(now).unwrap();
        i.finish(&dispatch, b.clone(), 96, now);
        assert_eq!(i.take(Some(&b), Some(false), now).unwrap().0.grant, b.grant);
    }
    #[test]
    fn accepted_entry_can_match_only_its_exact_grant_and_generation() {
        let i = Intent::default();
        let b = binding(&i);
        let grant = Grant {
            owner: b.presenter.clone(),
            document_epoch: b.epoch,
            revision: 4,
            window_key: "root".into(),
        };
        let mut current = b.clone();
        current.grant = Some(grant.clone());
        current.generation += 1;
        assert!(b.after_acquire(&current, &(grant.clone(), current.generation)));
        let mut foreign = grant.clone();
        foreign.revision += 1;
        assert!(!b.after_acquire(&current, &(foreign, current.generation)));
        assert!(!b.after_acquire(&current, &(grant, current.generation + 1)));
    }
    #[test]
    fn dispatch_has_no_queue_timestamp_dependency_and_never_waits_on_busy_state() {
        let i = Intent::default();
        let now = Instant::now();
        let busy = i.pending.lock().unwrap();
        assert!(i.begin_at(now).is_none());
        assert!(!i.active.load(Ordering::Acquire));
        drop(busy);
        let first = i.begin_at(now).unwrap();
        let revision = first.id.0;
        drop(first);
        let second = i.begin_at(now).unwrap();
        assert_eq!(second.id.0, revision + 1);
    }
    #[test]
    fn independent_receivers_cannot_commit_each_others_dispatch() {
        let i = Intent::default();
        let other = Intent::default();
        let now = Instant::now();
        let first = i.begin_at(now).unwrap();
        let second = other.begin_at(now).unwrap();
        let b = binding(&other);
        other.finish(&first, b.clone(), 96, now);
        assert!(other.take(Some(&b), Some(true), now).is_none());
        other.finish(&second, b.clone(), 96, now);
        assert!(other.take(Some(&b), Some(true), now).is_some());
    }
    #[test]
    fn observations_distinguish_absent_event_context_rejection_and_stale_evidence() {
        let i = Intent::default();
        let now = Instant::now();
        assert_eq!(i.take_observed(None, Some(true), now).1, Reason::NoEvent);
        let dispatch = i.begin_at(now).unwrap();
        i.note(&dispatch, Reason::NativePointer);
        assert_eq!(
            i.take_observed(None, Some(true), now).1,
            Reason::NativePointer
        );
        i.clear();
        assert_eq!(
            i.take_observed(None, Some(true), now).1,
            Reason::NativePointer
        );
        assert_eq!(
            i.take_observed(None, Some(true), now + MAX_AGE + Duration::from_nanos(1))
                .1,
            Reason::Missing
        );
    }
    #[test]
    fn late_observation_cannot_replace_newer_reason_or_create_permission() {
        let i = Intent::default();
        let now = Instant::now();
        let old = i.begin_at(now).unwrap();
        i.clear();
        i.note(&old, Reason::ForwardFocus);
        assert_eq!(
            i.take_observed(None, Some(true), now).1,
            Reason::Invalidated
        );
        drop(old);
        let newer = i.begin_at(now).unwrap();
        i.note(&newer, Reason::NativePointer);
        assert_eq!(
            i.take_observed(Some(&binding(&i)), Some(true), now),
            (None, Reason::NativePointer)
        );
    }
    #[test]
    fn consuming_diagnostics_preserve_exact_deadline_direction_and_binding_decisions() {
        let i = Intent::default();
        let now = Instant::now();
        let b = mint(&i, 1, now);
        assert_eq!(
            i.take_observed(Some(&b), Some(false), now).1,
            Reason::TransitionMismatch
        );
        let b = mint(&i, 2, now);
        assert_eq!(
            i.take_observed(
                Some(&b),
                Some(true),
                now + MAX_AGE + Duration::from_nanos(1)
            )
            .1,
            Reason::Expired
        );
        let mut b = mint(&i, 3, now);
        b.epoch += 1;
        assert_eq!(
            i.take_observed(Some(&b), Some(true), now).1,
            Reason::BindingMismatch
        );
        let b = mint(&i, 4, now);
        assert_eq!(
            i.take_observed(Some(&b), Some(true), now).1,
            Reason::Accepted
        );
        assert_eq!(i.take_observed(Some(&b), Some(true), now).0, None);
    }
}
