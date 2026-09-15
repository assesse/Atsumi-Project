//! Isolated native Tao keyboard-message reentry regression. No WebView, network,
//! AppState, account, recording, user database, or real desktop input is used.
//! cargo run --offline --example windows_input_probe
//!
//! The extra subclass pauses a synthetic key down/up before forwarding to Tao.
//! A second thread sends WM_KILLFOCUS to this exact disposable HWND. The queue's
//! QS_SENDMESSAGE high word establishes the rendezvous without pumping messages.
//! Tao's own PeekMessage then reenters the focus path. Before upstream c704261c,
//! that path tries to reacquire KEY_EVENT_BUILDERS while the outer key owns it.
//! https://github.com/tauri-apps/tao/pull/1215
//! https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-getqueuestatus
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows native window required");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = probe::run() {
        eprintln!("WINDOWS_INPUT_PROBE_FAILED: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod probe {
    use serde_json::{json, Value};
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering},
            Arc,
        },
        thread,
        time::{Duration, Instant},
    };
    use windows::{
        core::BOOL,
        Win32::{
            Foundation::{HWND, LPARAM, LRESULT, WPARAM},
            UI::WindowsAndMessaging::{
                GetQueueStatus, GetWindowThreadProcessId, SendMessageTimeoutW, QS_SENDMESSAGE,
                SMTO_ABORTIFHUNG, SMTO_BLOCK, SMTO_ERRORONEXIT, WM_CHAR, WM_KEYDOWN, WM_KEYUP,
                WM_KILLFOCUS, WM_NCDESTROY, WM_SETFOCUS,
            },
        },
    };

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    type SubclassProc =
        unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT;
    const SUBCLASS_ID: usize = 0x4154_4b50;
    const CYCLES: usize = 4;
    const KEY_A: usize = 0x41;
    const SCAN_A: isize = 0x001e_0001;
    static EXIT_CODE: AtomicI32 = AtomicI32::new(2);

    #[link(name = "comctl32")]
    unsafe extern "system" {
        fn SetWindowSubclass(hwnd: HWND, callback: SubclassProc, id: usize, data: usize) -> BOOL;
        fn RemoveWindowSubclass(hwnd: HWND, callback: SubclassProc, id: usize) -> BOOL;
        fn DefSubclassProc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    }

    #[derive(Default)]
    struct Shared {
        hwnd: AtomicUsize,
        alive: AtomicBool,
        done: AtomicBool,
        stop_sender: AtomicBool,
        armed: AtomicBool,
        failed: AtomicBool,
        depth: AtomicUsize,
        forwarding_key: AtomicBool,
        requested: AtomicUsize,
        send_started: AtomicUsize,
        send_finished: AtomicUsize,
        queued_rendezvous: AtomicUsize,
        focus_entered: AtomicUsize,
        focus_returned: AtomicUsize,
        premature_focus: AtomicUsize,
        key_returned: AtomicUsize,
        chars_returned: AtomicUsize,
    }
    impl Shared {
        fn summary(&self, success: bool) -> Value {
            json!({"kind":"windows_input_probe","success":success,"cycles":CYCLES,
                "keyCallsReturned":self.key_returned.load(Ordering::Acquire),
                "characterCallsReturned":self.chars_returned.load(Ordering::Acquire),
                "queuedRendezvous":self.queued_rendezvous.load(Ordering::Acquire),
                "focusEnteredDuringKeyForward":self.focus_entered.load(Ordering::Acquire),
                "focusReturnedDuringKeyForward":self.focus_returned.load(Ordering::Acquire),
                "focusReenteredBeforeForward":self.premature_focus.load(Ordering::Acquire),
                "sendRequests":self.requested.load(Ordering::Acquire),
                "sendRequestsFinished":self.send_finished.load(Ordering::Acquire),
                "failed":self.failed.load(Ordering::Acquire),"hiddenNativeWindowOnly":true,
                "webview":false,"realDesktopInput":false,"userDatabase":false})
        }
    }

    unsafe extern "system" fn input_subclass(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _: usize,
        data: usize,
    ) -> LRESULT {
        // `run` and both worker threads retain Arc owners until the subclass is
        // removed on this UI thread or its one owned window is destroyed.
        let shared = &*(data as *const Shared);
        let depth = shared.depth.fetch_add(1, Ordering::AcqRel) + 1;
        let controlled_key = matches!(message, WM_KEYDOWN | WM_KEYUP)
            && wparam.0 == KEY_A
            && shared.armed.swap(false, Ordering::AcqRel);
        if controlled_key {
            let sequence = shared.requested.fetch_add(1, Ordering::AcqRel) + 1;
            let begin = Instant::now();
            let mut queued = false;
            while begin.elapsed() < Duration::from_secs(1) {
                if shared.send_started.load(Ordering::Acquire) >= sequence
                    && (GetQueueStatus(QS_SENDMESSAGE) >> 16) & QS_SENDMESSAGE.0 != 0
                {
                    queued = true;
                    break;
                }
                // Sleep does not pump this UI thread's sent-message queue.
                thread::sleep(Duration::from_millis(1));
            }
            if queued {
                shared.queued_rendezvous.fetch_add(1, Ordering::AcqRel);
            } else {
                shared.failed.store(true, Ordering::Release);
            }
            shared.forwarding_key.store(true, Ordering::Release);
        }
        let reentered_focus =
            message == WM_KILLFOCUS && depth >= 2 && shared.forwarding_key.load(Ordering::Acquire);
        if reentered_focus {
            shared.focus_entered.fetch_add(1, Ordering::AcqRel);
        } else if message == WM_KILLFOCUS && depth >= 2 {
            // If GetQueueStatus itself or another pre-forward operation pumped
            // the message, this is not evidence of the intended Tao path.
            shared.premature_focus.fetch_add(1, Ordering::AcqRel);
        }
        if message == WM_NCDESTROY {
            shared.alive.store(false, Ordering::Release);
            let _ = RemoveWindowSubclass(hwnd, input_subclass, SUBCLASS_ID);
        }
        let result = DefSubclassProc(hwnd, message, wparam, lparam);
        if reentered_focus {
            shared.focus_returned.fetch_add(1, Ordering::AcqRel);
        }
        if controlled_key {
            shared.forwarding_key.store(false, Ordering::Release);
            shared.key_returned.fetch_add(1, Ordering::AcqRel);
        }
        if message == WM_CHAR {
            shared.chars_returned.fetch_add(1, Ordering::AcqRel);
        }
        shared.depth.fetch_sub(1, Ordering::AcqRel);
        result
    }

    fn send_owned(shared: &Shared, message: u32, wparam: usize, lparam: isize) -> bool {
        if !shared.alive.load(Ordering::Acquire) {
            return false;
        }
        let hwnd = HWND(shared.hwnd.load(Ordering::Acquire) as *mut _);
        let mut process = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut process));
            if process != std::process::id() {
                return false;
            }
            let mut result = 0;
            SendMessageTimeoutW(
                hwnd,
                message,
                WPARAM(wparam),
                LPARAM(lparam),
                SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
                3000,
                Some(&mut result),
            )
            .0 != 0
        }
    }

    pub fn run() -> Result<()> {
        let shared = Arc::new(Shared::default());
        let watchdog = shared.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(40);
            while Instant::now() < deadline {
                if watchdog.done.load(Ordering::Acquire) {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
            // A deliberately reproduced UI deadlock cannot service app.exit.
            // Terminate only this isolated probe process, never a user process.
            eprintln!("WINDOWS_INPUT_PROBE_DEADLINE: {}", watchdog.summary(false));
            std::process::exit(3);
        });
        let setup_shared = shared.clone();
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        context.config_mut().app.tray_icon = None;
        let app = tauri::Builder::default()
            .setup(move |app| {
                let label = format!("native-input-probe-{}", uuid::Uuid::new_v4().simple());
                let window = tauri::window::WindowBuilder::new(app, label)
                    .title("Isolated native input regression")
                    .inner_size(160.0, 100.0)
                    .position(-16000.0, -16000.0)
                    .skip_taskbar(true)
                    .focused(false)
                    .visible(false)
                    .build()?;
                let hwnd = window.hwnd()?;
                setup_shared.hwnd.store(hwnd.0 as usize, Ordering::Release);
                unsafe {
                    if !SetWindowSubclass(
                        hwnd,
                        input_subclass,
                        SUBCLASS_ID,
                        Arc::as_ptr(&setup_shared) as usize,
                    )
                    .as_bool()
                    {
                        return Err("could not subclass the isolated native window".into());
                    }
                }
                setup_shared.alive.store(true, Ordering::Release);
                let sender_shared = setup_shared.clone();
                let sender = thread::spawn(move || {
                    let mut previous = 0;
                    while !sender_shared.stop_sender.load(Ordering::Acquire) {
                        let requested = sender_shared.requested.load(Ordering::Acquire);
                        if requested > previous {
                            previous = requested;
                            sender_shared
                                .send_started
                                .store(requested, Ordering::Release);
                            if !send_owned(&sender_shared, WM_KILLFOCUS, 0, 0) {
                                sender_shared.failed.store(true, Ordering::Release);
                            }
                            sender_shared
                                .send_finished
                                .store(requested, Ordering::Release);
                        } else {
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                });
                let app = app.handle().clone();
                thread::spawn(move || {
                    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        for _ in 0..CYCLES {
                            // These are notifications to our hidden HWND only;
                            // no SetFocus/SendInput/global hook or foreground change.
                            if !send_owned(&setup_shared, WM_SETFOCUS, 0, 0) {
                                return false;
                            }
                            for (message, lparam) in
                                [(WM_KEYDOWN, SCAN_A), (WM_KEYUP, SCAN_A | 0xc000_0000)]
                            {
                                setup_shared.armed.store(true, Ordering::Release);
                                if !send_owned(&setup_shared, message, KEY_A, lparam) {
                                    return false;
                                }
                                let requested = setup_shared.requested.load(Ordering::Acquire);
                                let deadline = Instant::now() + Duration::from_secs(3);
                                while setup_shared.send_finished.load(Ordering::Acquire) < requested
                                {
                                    if Instant::now() >= deadline {
                                        return false;
                                    }
                                    thread::sleep(Duration::from_millis(1));
                                }
                                if setup_shared.failed.load(Ordering::Acquire) {
                                    return false;
                                }
                                if message == WM_KEYDOWN
                                    && !send_owned(&setup_shared, WM_CHAR, KEY_A, SCAN_A)
                                {
                                    return false;
                                }
                            }
                            if !send_owned(&setup_shared, WM_KILLFOCUS, 0, 0) {
                                return false;
                            }
                        }
                        let expected = CYCLES * 2;
                        setup_shared.key_returned.load(Ordering::Acquire) == expected
                            && setup_shared.queued_rendezvous.load(Ordering::Acquire) == expected
                            && setup_shared.focus_entered.load(Ordering::Acquire) == expected
                            && setup_shared.focus_returned.load(Ordering::Acquire) == expected
                            && setup_shared.premature_focus.load(Ordering::Acquire) == 0
                            && setup_shared.chars_returned.load(Ordering::Acquire) >= CYCLES
                            && !setup_shared.failed.load(Ordering::Acquire)
                    }));
                    let success = matches!(run, Ok(true));
                    println!("{}", setup_shared.summary(success));
                    setup_shared.stop_sender.store(true, Ordering::Release);
                    // A timed-out call may leave the UI in the old deadlock. Keep
                    // the watchdog armed until the UI confirms subclass removal.
                    if !success {
                        EXIT_CODE.store(2, Ordering::Release);
                        app.exit(2);
                        return;
                    }
                    let _ = sender.join();
                    let exit_app = app.clone();
                    let cleanup_shared = setup_shared.clone();
                    if app
                        .run_on_main_thread(move || unsafe {
                            let hwnd = HWND(cleanup_shared.hwnd.load(Ordering::Acquire) as *mut _);
                            let removed =
                                RemoveWindowSubclass(hwnd, input_subclass, SUBCLASS_ID).as_bool();
                            cleanup_shared.alive.store(false, Ordering::Release);
                            let code = if removed { 0 } else { 2 };
                            EXIT_CODE.store(code, Ordering::Release);
                            exit_app.exit(code);
                        })
                        .is_err()
                    {
                        EXIT_CODE.store(2, Ordering::Release);
                        app.exit(2);
                    }
                });
                Ok(())
            })
            .build(context)?;
        let runtime_code = app.run_return(|_, _| {});
        shared.stop_sender.store(true, Ordering::Release);
        shared.done.store(true, Ordering::Release);
        let code = EXIT_CODE.load(Ordering::Acquire);
        if code == 0 && runtime_code == 0 {
            Ok(())
        } else {
            Err(format!("native input probe exit {code}").into())
        }
    }
}
