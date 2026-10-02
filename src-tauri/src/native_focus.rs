//! Restore keyboard delivery when Windows activates only the outer host window.
//! DOM `focus()` cannot repair this: the WebView receives no keyboard events.
//! Only an already-foreground, enabled main window with orphaned native focus
//! is repaired. An editor, chat pane, owned dialog or another app keeps its focus.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusOwner {
    None,
    Host,
    Child,
    Other,
}

fn needs_restore(foreground: bool, available: bool, owner: FocusOwner) -> bool {
    foreground && available && matches!(owner, FocusOwner::None | FocusOwner::Host)
}

#[cfg(windows)]
pub(crate) fn install(view: &tauri::WebviewWindow) -> tauri::Result<()> {
    use std::cell::Cell;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Controller, COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC,
    };
    use windows::{
        core::{w, BOOL},
        Win32::{
            Foundation::{HWND, LPARAM, LRESULT, WPARAM},
            UI::{
                Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled, SetFocus},
                Shell::{
                    DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
                },
                WindowsAndMessaging::{
                    GetForegroundWindow, IsChild, IsIconic, IsWindowVisible, PostMessageW,
                    RegisterWindowMessageW, WM_ACTIVATE, WM_NCDESTROY, WM_SETFOCUS,
                },
            },
        },
    };

    const SUBCLASS_ID: usize = 0x41545346; // ATSF, scoped to the trusted main HWND.
    struct State {
        controller: ICoreWebView2Controller,
        message: u32,
        queued: Cell<bool>,
        last_child: Cell<HWND>,
    }
    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        if message == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(window_proc), id);
            drop(Box::from_raw(data as *mut State));
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }
        let state = &*(data as *const State);
        if message == state.message {
            state.queued.set(false);
            let focused = GetFocus();
            let owner = if focused.0.is_null() {
                FocusOwner::None
            } else if focused == hwnd {
                FocusOwner::Host
            } else if IsChild(hwnd, focused).as_bool() {
                FocusOwner::Child
            } else {
                FocusOwner::Other
            };
            let available = IsWindowVisible(hwnd).as_bool()
                && IsWindowEnabled(hwnd).as_bool()
                && !IsIconic(hwnd).as_bool();
            if !needs_restore(GetForegroundWindow() == hwnd, available, owner) {
                return LRESULT(0);
            }
            // Copy everything before COM/SetFocus: both can reenter window procs.
            // Never hold a lock or mutable borrow across either operation.
            let previous = state.last_child.get();
            let controller = state.controller.clone();
            if !previous.0.is_null()
                && IsChild(hwnd, previous).as_bool()
                && IsWindowVisible(previous).as_bool()
                && IsWindowEnabled(previous).as_bool()
            {
                let _ = SetFocus(Some(previous));
            } else {
                let mut visible = BOOL::default();
                if controller.IsVisible(&mut visible).is_ok() && visible.as_bool() {
                    let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
                }
            }
            tracing::debug!(
                ?owner,
                restored = GetFocus() != hwnd && !GetFocus().0.is_null(),
                "restored orphaned main-window keyboard focus"
            );
            return LRESULT(0);
        }
        if message == WM_ACTIVATE && wparam.0 & 0xffff == 0 {
            let focused = GetFocus();
            if IsChild(hwnd, focused).as_bool() {
                state.last_child.set(focused);
            }
        }
        if (message == WM_SETFOCUS || (message == WM_ACTIVATE && wparam.0 & 0xffff != 0))
            && !state.queued.replace(true)
            && PostMessageW(Some(hwnd), state.message, WPARAM(0), LPARAM(0)).is_err()
        {
            state.queued.set(false);
        }
        // Let Windows/Wry finish normal activation first. The posted message
        // repairs only focus that is STILL on the frame after that completes.
        DefSubclassProc(hwnd, message, wparam, lparam)
    }

    let host = view.hwnd()?.0 as usize;
    view.with_webview(move |platform| unsafe {
        let hwnd = HWND(host as *mut std::ffi::c_void);
        let mut existing = 0;
        if GetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, Some(&mut existing)).as_bool() {
            return;
        }
        let message = RegisterWindowMessageW(w!("Atsumi.RestoreWebViewFocus.v1"));
        if message == 0 {
            tracing::warn!("could not register native keyboard-focus repair message");
            return;
        }
        let state = Box::into_raw(Box::new(State {
            controller: platform.controller(),
            message,
            queued: Cell::new(false),
            last_child: Cell::new(HWND::default()),
        }));
        if !SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, state as usize).as_bool() {
            drop(Box::from_raw(state));
            tracing::warn!("could not install native keyboard-focus repair");
        }
    })
}

#[cfg(not(windows))]
pub(crate) fn install(_: &tauri::WebviewWindow) -> tauri::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_foreground_host_or_missing_focus_is_repaired() {
        for owner in [
            FocusOwner::None,
            FocusOwner::Host,
            FocusOwner::Child,
            FocusOwner::Other,
        ] {
            assert_eq!(
                needs_restore(true, true, owner),
                matches!(owner, FocusOwner::None | FocusOwner::Host)
            );
            assert!(
                !needs_restore(false, true, owner),
                "must not activate a background app"
            );
            assert!(
                !needs_restore(true, false, owner),
                "must not bypass a modal, minimize, or hide"
            );
        }
    }
}
