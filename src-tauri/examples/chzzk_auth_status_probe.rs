//! Native account observation with a disposable anonymous WebView2 profile.
//! --unavailable-profile checks failure handling. Never uses the user's app profile.
#[cfg(not(windows))]
fn main() {}

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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let unavailable_profile = args == ["--unavailable-profile"];
    if !args.is_empty() && !unavailable_profile {
        std::process::exit(2);
    }
    let directory = tempfile::Builder::new()
        .prefix("atsumi-auth-status-probe-")
        .tempdir()
        .unwrap();
    let host = OfficialBrowser::new(directory.path().into()).unwrap();
    if unavailable_profile {
        std::fs::write(
            directory.path().join("chzzk-browser-profile"),
            b"isolated failure fixture",
        )
        .unwrap();
    }
    let outcome = Arc::new(AtomicI32::new(2));
    let worker_outcome = outcome.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let runtime = tauri::Builder::default().setup(move |app| {
        let handle = app.handle().clone();
        let deadline = handle.clone();
        std::thread::spawn(move || { std::thread::sleep(Duration::from_secs(40)); deadline.exit(2); });
        tauri::async_runtime::spawn(async move {
            let mut checks = Vec::new();
            for _ in 0..2 {
                let started = Instant::now();
                let snapshot = match host.refresh_login_status(&handle, true).await {
                    Ok(value) => serde_json::to_value(value).unwrap(),
                    Err(error) => { eprintln!("{}", json!({"ok":false,"code":error.code})); handle.exit(1); return; }
                };
                let cleanup = Instant::now();
                while !handle.webview_windows().is_empty() && cleanup.elapsed() < Duration::from_secs(3) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                let expected = if unavailable_profile { "unknown" } else { "signed_out" };
                let ok = snapshot["authStatus"] == expected && snapshot["authChecking"] == false
                    && snapshot["authError"].is_string() == unavailable_profile && snapshot["windowOpen"] == false
                    && snapshot["accountBusy"] == false && handle.webview_windows().is_empty();
                checks.push(json!({"ok":ok,"authStatus":snapshot["authStatus"],"authChecking":snapshot["authChecking"],"hasError":snapshot["authError"].is_string(),"elapsedMs":started.elapsed().as_millis(),"remainingWindows":handle.webview_windows().len()}));
                if !ok { eprintln!("{}", json!({"unavailableProfile":unavailable_profile,"checks":checks})); handle.exit(1); return; }
            }
            println!("{}", json!({"ok":true,"unavailableProfile":unavailable_profile,"anonymousProfile":true,"noPlayerOpened":true,"checks":checks}));
            worker_outcome.store(0, Ordering::Release);
            handle.exit(0);
        });
        Ok(())
    }).build(context).unwrap().run_return(|_, event| {
        if let tauri::RunEvent::ExitRequested { api, code: None, .. } = event { api.prevent_exit(); }
    });
    drop(directory);
    std::process::exit(if runtime == 0 {
        outcome.load(Ordering::Acquire)
    } else {
        runtime
    });
}
