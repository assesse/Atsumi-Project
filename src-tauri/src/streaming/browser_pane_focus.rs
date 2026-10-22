//! Keep a user's pane click and WebView2 keyboard destination in agreement.
//! This is deliberately not activation repair or a periodic focus policy.
//! No keyboard events, DOM input contents, or general remote IPC are intercepted.

pub(super) fn initialization_script(expected_url: &str, nonce: &str) -> String {
    format!("({})({},{});", include_str!("browser_pane_focus.js"), serde_json::json!(expected_url), serde_json::json!(nonce))
}

fn matches_pointer_message(source: &str, body: &str, expected_url: &str, nonce: &str) -> bool {
    source == expected_url
        && body.strip_prefix("ATSUMI_PANE_POINTER:").is_some_and(|value| value == nonce)
}

#[derive(Clone, Copy, Debug)]
struct PointerFocusState {
    foreground: bool,
    visible: bool,
    enabled: bool,
    hit_inside: bool,
    focus_inside: bool,
    reentrant: bool,
}

fn should_repair_pointer_focus(pointer_down: bool, state: PointerFocusState) -> bool {
    pointer_down
        && state.foreground
        && state.visible
        && state.enabled
        && state.hit_inside
        && !state.focus_inside
        && !state.reentrant
}

#[cfg(windows)]
mod windows_pane {
    use super::{matches_pointer_message, should_repair_pointer_focus, PointerFocusState};
    use std::{cell::Cell, rc::Rc};
    use webview2_com::{
        CoTaskMemPWSTR, WebMessageReceivedEventHandler,
        Microsoft::Web::WebView2::Win32::{ICoreWebView2Controller, COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC},
    };
    use windows::{
        core::{BOOL, PWSTR},
        Win32::{
            Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
            UI::{
                Input::KeyboardAndMouse::{GetAsyncKeyState, GetFocus, IsWindowEnabled, VK_LBUTTON},
                Shell::{DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass},
                WindowsAndMessaging::{
                    GetCursorPos, GetForegroundWindow, GetParent, IsChild, IsWindowVisible,
                    WindowFromPoint, WM_LBUTTONDOWN, WM_NCDESTROY, WM_PARENTNOTIFY,
                },
            },
        },
    };

    const SUBCLASS_ID: usize = 0x41545046; // ATPF, scoped to one owned pane HWND.

    struct State {
        hwnd: HWND,
        root: HWND,
        controller: ICoreWebView2Controller,
        repairing: Cell<bool>,
        closed: Cell<bool>,
    }

    struct RepairGuard<'a>(&'a Cell<bool>);
    impl Drop for RepairGuard<'_> {
        fn drop(&mut self) { self.0.set(false); }
    }

    fn is_pointer_down(message: u32, wparam: usize) -> bool {
        message == WM_LBUTTONDOWN
            || (message == WM_PARENTNOTIFY && (wparam & 0xffff) as u32 == WM_LBUTTONDOWN)
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
            drop(state);
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }
        if !is_pointer_down(message, wparam.0) {
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }

        // MoveFocus can reenter a window procedure, including teardown. Keep a
        // separate reference alive, never a mutable borrow or mutex across COM.
        Rc::increment_strong_count(data as *const State);
        let state = Rc::from_raw(data as *const State);
        repair(&state, "native_pointer");
        // Do not consume the click, change capture, or replace the official
        // input handler. A click on an already-focused pane is entirely inert.
        DefSubclassProc(hwnd, message, wparam, lparam)
    }

    unsafe fn repair(state: &State, operation: &'static str) {
        if state.closed.get() || state.repairing.replace(true) { return; }
        let _guard = RepairGuard(&state.repairing);
        let hwnd = state.hwnd;
        let mut visible = BOOL::default();
        let controller_visible = state.controller.IsVisible(&mut visible).is_ok() && visible.as_bool();
        // COM can pump teardown or another input event. Do not use a pre-COM
        // snapshot or a released mouse button to redirect a later keystroke.
        if state.closed.get() || GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 == 0 { return; }
        let focused = GetFocus();
        let mut point = POINT::default();
        let hit = if GetCursorPos(&mut point).is_ok() {
            WindowFromPoint(point)
        } else {
            HWND::default()
        };
        let policy = PointerFocusState {
                foreground: GetForegroundWindow() == state.root,
                visible: controller_visible && IsWindowVisible(hwnd).as_bool(),
                enabled: IsWindowEnabled(state.root).as_bool() && IsWindowEnabled(hwnd).as_bool(),
                hit_inside: hit == hwnd || IsChild(hwnd, hit).as_bool(),
                focus_inside: focused == hwnd || IsChild(hwnd, focused).as_bool(),
                reentrant: false, // This invocation owns the guard across COM.
            };
        if should_repair_pointer_focus(true, policy) {
            // Programmatic preserves the page's existing focused element. The
            // normal click below still determines its final caret/selection.
            let result = state.controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
            let actual = GetFocus();
            if result.is_ok() && !state.closed.get() && (actual == hwnd || IsChild(hwnd, actual).as_bool()) {
                tracing::info!(diag_stage = "chzzk_pane_pointer_focus_repaired", diag_operation = operation, success = true);
            } else {
                tracing::warn!(diag_stage = "chzzk_pane_pointer_focus_failed", diag_operation = operation, success = false);
            }
        } else if policy.focus_inside {
            tracing::info!(diag_stage = "chzzk_pane_pointer_focus_already_owned", diag_operation = operation);
        } else {
            tracing::info!(diag_stage = "chzzk_pane_pointer_focus_guarded", diag_operation = operation);
        }
    }

    pub(super) fn install(view: &tauri::Webview, expected_url: String, nonce: String) -> tauri::Result<()> {
        let root = view.window().hwnd()?.0 as usize;
        view.with_webview(move |platform| unsafe {
            let root = HWND(root as *mut std::ffi::c_void);
            let controller = platform.controller();
            let mut hwnd = HWND::default();
            // Only the child container returned by this exact controller is
            // eligible. Never subclass the root, another app, or login popup.
            if controller.ParentWindow(&mut hwnd).is_err()
                || hwnd == root
                || GetParent(hwnd).ok() != Some(root)
            {
                tracing::warn!(diag_stage = "chzzk_pane_pointer_focus_unavailable");
                return;
            }
            let mut existing = 0;
            if GetWindowSubclass(hwnd, Some(pane_proc), SUBCLASS_ID, Some(&mut existing)).as_bool() {
                return;
            }
            let state = Rc::new(State {
                hwnd,
                root,
                controller,
                repairing: Cell::new(false),
                closed: Cell::new(false),
            });
            let raw = Rc::into_raw(state.clone());
            if !SetWindowSubclass(hwnd, Some(pane_proc), SUBCLASS_ID, raw as usize).as_bool() {
                drop(Rc::from_raw(raw));
                tracing::warn!(diag_stage = "chzzk_pane_pointer_focus_unavailable");
                return;
            }
            let weak = Rc::downgrade(&state);
            let attached = (|| -> windows::core::Result<()> {
                let core = state.controller.CoreWebView2()?;
                let mut registration = 0;
                core.add_WebMessageReceived(&WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()); };
                    let Some(state) = weak.upgrade() else { return Ok(()); };
                    if state.closed.get() || GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 == 0 {
                        return Ok(());
                    }
                    let mut source = PWSTR::null();
                    args.Source(&mut source)?;
                    let source = CoTaskMemPWSTR::from(source).to_string();
                    if source != expected_url { return Ok(()); }
                    let mut body = PWSTR::null();
                    if args.TryGetWebMessageAsString(&mut body).is_err() { return Ok(()); }
                    let body = CoTaskMemPWSTR::from(body).to_string();
                    if matches_pointer_message(&source, &body, &expected_url, &nonce) {
                        repair(&state, "pointer_bridge");
                    }
                    Ok(())
                })), &mut registration)
            })();
            if attached.is_err() {
                tracing::warn!(diag_stage = "chzzk_pane_pointer_bridge_unavailable");
            }
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::UI::WindowsAndMessaging::{
            WM_ACTIVATE, WM_CREATE, WM_KEYDOWN, WM_MOUSEMOVE, WM_SETFOCUS,
        };

        #[test]
        fn only_native_pointer_press_triggers_the_repair_path() {
            assert!(is_pointer_down(WM_LBUTTONDOWN, 0));
            assert!(is_pointer_down(WM_PARENTNOTIFY, WM_LBUTTONDOWN as usize));
            assert!(is_pointer_down(WM_PARENTNOTIFY, (123 << 16) | WM_LBUTTONDOWN as usize));
            for message in [WM_ACTIVATE, WM_SETFOCUS, WM_KEYDOWN, WM_MOUSEMOVE, WM_CREATE] {
                assert!(!is_pointer_down(message, WM_LBUTTONDOWN as usize));
                assert!(!is_pointer_down(WM_PARENTNOTIFY, message as usize));
            }
        }
    }
}

pub(super) fn install(view: &tauri::Webview, expected_url: String, nonce: String) -> tauri::Result<()> {
    #[cfg(windows)]
    return windows_pane::install(view, expected_url, nonce);
    #[cfg(not(windows))]
    {
        let _ = (view, expected_url, nonce);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eligible() -> PointerFocusState {
        PointerFocusState {
            foreground: true,
            visible: true,
            enabled: true,
            hit_inside: true,
            focus_inside: false,
            reentrant: false,
        }
    }

    #[test]
    fn a_user_click_can_move_keyboard_focus_from_another_pane() {
        assert!(should_repair_pointer_focus(true, eligible()));
        assert!(!should_repair_pointer_focus(false, eligible()));
    }

    #[test]
    fn every_safety_guard_prevents_background_or_repeated_focus_changes() {
        for state in [
            PointerFocusState { foreground: false, ..eligible() },
            PointerFocusState { visible: false, ..eligible() },
            PointerFocusState { enabled: false, ..eligible() },
            PointerFocusState { hit_inside: false, ..eligible() },
            PointerFocusState { focus_inside: true, ..eligible() },
            PointerFocusState { reentrant: true, ..eligible() },
        ] {
            assert!(!should_repair_pointer_focus(true, state), "{state:?}");
        }
    }

    #[test]
    fn pointer_message_rejects_other_pages_panes_and_capture_payloads() {
        let url = "https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/chat";
        let nonce = "per_pane_nonce";
        let body = "ATSUMI_PANE_POINTER:per_pane_nonce";
        assert!(matches_pointer_message(url, body, url, nonce));
        for other in ["about:blank", "https://nid.naver.com/", "https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/chat?other=1"] {
            assert!(!matches_pointer_message(other, body, url, nonce));
        }
        for poisoned in ["ATSUMI_PANE_POINTER:other_nonce", "ATSUMI_PANE_POINTER:per_pane_nonce\n", "ATSUMI_BROWSER_CAPTURE:{}", ""] {
            assert!(!matches_pointer_message(url, poisoned, url, nonce));
        }
    }
}
