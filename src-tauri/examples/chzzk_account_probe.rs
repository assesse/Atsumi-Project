//! Offline native logout regression. Fresh temporary profile, hidden blank
//! windows only; never opens NAVER, reads real cookies, or uses the app DB.
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows/WebView2 required");
}

#[cfg(windows)]
fn main() {
    use atsumi_lib::streaming::browser::OfficialBrowser;
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicI32, Ordering},
        Arc,
    };
    use std::time::{Duration, Instant};
    use tauri::Manager;
    if std::env::args_os().len() != 1 {
        std::process::exit(2);
    }
    let directory = tempfile::Builder::new()
        .prefix("atsumi-account-probe-")
        .tempdir()
        .expect("isolated profile");
    let host = OfficialBrowser::new(directory.path().into()).expect("isolated host");
    let outcome = Arc::new(AtomicI32::new(2));
    let worker_outcome = outcome.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let runtime_code = tauri::Builder::default().setup(move |app| {
        let handle = app.handle().clone();
        let deadline = handle.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(45));
            eprintln!("account probe deadline exceeded");
            deadline.exit(2);
        });
        std::thread::spawn(move || {
            for _ in 0..2 {
                if let Err(error) = host.logout(&handle) {
                    eprintln!("{}", json!({"ok":false,"code":error.code}));
                    handle.exit(1); return;
                }
                // destroy() queues native window removal. Let that event finish
                // before asserting cleanup or starting the next operation.
                let started = Instant::now();
                while !handle.webview_windows().is_empty() && started.elapsed() < Duration::from_secs(3) {
                    std::thread::sleep(Duration::from_millis(20));
                }
                let snapshot = serde_json::to_value(host.snapshot().expect("snapshot")).expect("json");
                if snapshot["accountBusy"] != false || snapshot["authStatus"] != "signed_out" || snapshot["windowOpen"] != false
                    || !handle.webview_windows().is_empty() {
                    eprintln!("{}", json!({"ok":false,"stage":"logout_state_or_window_cleanup","accountBusy":snapshot["accountBusy"],"authStatus":snapshot["authStatus"],"windowOpen":snapshot["windowOpen"],"remainingWindows":handle.webview_windows().len()}));
                    handle.exit(1); return;
                }
            }
            println!("{}", json!({"ok":true,"isolatedProfile":true,"logoutWithoutPlayback":true,"repeatedLogout":true,"temporaryWindowsClosed":true,"realAccountAccessed":false}));
            worker_outcome.store(0, Ordering::Release);
            handle.exit(0);
        });
        Ok(())
    }).build(context).expect("probe runtime").run_return(|_, event| {
        // No main window exists in this headless probe. Closing the temporary
        // profile owner must not terminate the event loop before verification.
        if let tauri::RunEvent::ExitRequested { api, code: None, .. } = event {
            api.prevent_exit();
        }
    });
    drop(directory);
    std::process::exit(if runtime_code == 0 {
        outcome.load(Ordering::Acquire)
    } else {
        runtime_code
    });
}
