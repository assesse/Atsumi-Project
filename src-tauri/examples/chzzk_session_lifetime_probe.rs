//! Offline WebView2 session-lifetime regression. Only synthetic loopback cookies
//! and a fresh temporary profile; never accesses NAVER or the user's app profile.
//! --after-logout exercises the application's real logout before signing in again.
//! --keep-owner retains one blank profile owner while login/check windows close.
//! --app-session uses the application's real shared, lazy profile owner.
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    use atsumi_lib::streaming::browser::OfficialBrowser;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicI32, Ordering},
            Arc,
        },
        time::Duration,
    };
    use tauri::Manager;
    let args: Vec<_> = std::env::args().skip(1).collect();
    let keep_owner = args.iter().any(|arg| arg == "--keep-owner");
    let after_logout = args.iter().any(|arg| arg == "--after-logout");
    let app_session = args.iter().any(|arg| arg == "--app-session");
    if args.iter().any(|arg| {
        !matches!(
            arg.as_str(),
            "--keep-owner" | "--after-logout" | "--app-session"
        )
    }) {
        std::process::exit(2);
    }
    let directory = tempfile::Builder::new()
        .prefix("atsumi-session-lifetime-")
        .tempdir()
        .unwrap();
    let profile = directory.path().join("chzzk-browser-profile");
    let host = OfficialBrowser::new(directory.path().into()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut bytes = [0u8; 16384];
            let Ok(length) = stream.read(&mut bytes) else {
                continue;
            };
            let request = String::from_utf8_lossy(&bytes[..length]);
            let seed = request.starts_with("GET /seed ");
            let status = request.starts_with("GET /status ");
            let cookies = request
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("cookie:"))
                .unwrap_or("");
            let body = if status {
                serde_json::json!({"session":cookies.contains("atsumi_probe_session=present"), "persistent":cookies.contains("atsumi_probe_persist=present")}).to_string()
            } else {
                "<!doctype html><title>Isolated session fixture</title>".into()
            };
            let headers = if seed {
                "Set-Cookie: atsumi_probe_session=present; HttpOnly; SameSite=Lax; Path=/\r\nSet-Cookie: atsumi_probe_persist=present; HttpOnly; SameSite=Lax; Path=/; Max-Age=3600\r\n"
            } else {
                ""
            };
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nCache-Control: no-store\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}", if status { "application/json" } else { "text/html" }, headers, body.len(), body);
        }
    });
    let outcome = Arc::new(AtomicI32::new(2));
    let worker_outcome = outcome.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let runtime = tauri::Builder::default().setup(move |app| {
        let handle = app.handle().clone();
        let deadline = handle.clone();
        std::thread::spawn(move || { std::thread::sleep(Duration::from_secs(45)); deadline.exit(2); });
        std::thread::spawn(move || {
            let result = (|| -> Result<serde_json::Value, String> {
                if !handle.webview_windows().is_empty() { return Err("session was created eagerly".into()); }
                if app_session {
                    let requests: Vec<_> = (0..4).map(|_| {
                        let host = host.clone();
                        let handle = handle.clone();
                        std::thread::spawn(move || host.prepare_profile_session(&handle))
                    }).collect();
                    for request in requests {
                        request.join().map_err(|_| "session thread failed")?.map_err(|_| "app session failed")?;
                    }
                    if handle.webview_windows().len() != 1 { return Err("concurrent owners duplicated".into()); }
                }
                let keeper = if keep_owner { Some(fixture_window(&handle, "fixture-owner", &profile, "about:blank")?) } else { None };
                let logout_result = if after_logout {
                    let previous = fixture_window(&handle, "fixture-previous-login", &profile, &format!("{origin}/seed"))?;
                    let previous_status = fixture_status(&previous)?;
                    if previous_status["session"] != true || previous_status["persistent"] != true { return Err("previous fixture seed failed".into()); }
                    // Only the isolated profile is registered with this host.
                    // No actual account, media or user's settings are involved.
                    host.logout(&handle).map_err(|_| "application fixture logout failed")?;
                    let deadline = std::time::Instant::now() + Duration::from_secs(5);
                    loop {
                        let state = serde_json::to_value(host.snapshot().map_err(|_| "logout fixture state")?).map_err(|_| "logout fixture projection")?;
                        if state["accountBusy"] == false && state["authStatus"] == "signed_out" { break; }
                        if std::time::Instant::now() >= deadline { return Err("logout fixture did not finish".into()); }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    let cleared = fixture_status(&previous)?;
                    if cleared["session"] != false || cleared["persistent"] != false { return Err("application logout left fixture cookies".into()); }
                    previous.destroy().map_err(|_| "destroy previous fixture")?;
                    drop(previous);
                    std::thread::sleep(Duration::from_secs(2));
                    Some(cleared)
                } else { None };
                let seed = fixture_window(&handle, "fixture-login", &profile, &format!("{origin}/seed"))?;
                let before = fixture_status(&seed)?;
                if before["session"] != true || before["persistent"] != true { return Err("fixture seed failed".into()); }
                seed.destroy().map_err(|_| "destroy fixture")?;
                drop(seed);
                std::thread::sleep(Duration::from_secs(2));
                let mut checks = Vec::new();
                for index in 0..2 {
                    let check = fixture_window(&handle, &format!("fixture-check-{index}"), &profile, &format!("{origin}/status"))?;
                    checks.push(fixture_status(&check)?);
                    check.destroy().map_err(|_| "destroy check")?;
                    drop(check);
                    std::thread::sleep(Duration::from_secs(2));
                }
                let retained_by_app = app_session || after_logout;
                if retained_by_app {
                    let owner = handle.get_webview_window("chzzk-profile-session").ok_or("app owner missing")?;
                    if owner.is_visible().map_err(|_| "owner visibility")? || owner.url().map_err(|_| "owner URL")?.as_str() != "about:blank"
                        || !host.active_ids().is_empty() || serde_json::to_value(host.snapshot().map_err(|_| "owner state")?).unwrap()["windowOpen"] != false {
                        return Err("owner was not inert".into());
                    }
                }
                if let Some(keeper) = keeper { keeper.destroy().map_err(|_| "destroy owner")?; }
                host.shutdown_and_wait(&handle);
                std::thread::sleep(Duration::from_secs(2));
                if !handle.webview_windows().is_empty() || host.prepare_profile_session(&handle).is_ok() {
                    return Err("session recreated during shutdown".into());
                }
                let restarted = fixture_window(&handle, "fixture-restart", &profile, &format!("{origin}/status"))?;
                let restart_status = fixture_status(&restarted)?;
                restarted.destroy().map_err(|_| "destroy restarted fixture")?;
                let passed = checks.iter().all(|value| value["persistent"] == true && value["session"] == (keep_owner || retained_by_app))
                    && restart_status["persistent"] == true && restart_status["session"] == false;
                Ok(serde_json::json!({"ok":passed,"isolatedProfile":true,"realAccountAccessed":false,"keepOwner":keep_owner,"appSession":retained_by_app,"afterLogout":after_logout,"logoutCleared":logout_result,"before":before,"after":checks,"afterShutdown":restart_status}))
            })();
            match result {
                Ok(result) => { println!("{result}"); worker_outcome.store(if result["ok"] == true { 0 } else { 1 }, Ordering::Release); }
                Err(error) => { eprintln!("{}", serde_json::json!({"ok":false,"stage":error})); worker_outcome.store(1, Ordering::Release); }
            }
            handle.exit(worker_outcome.load(Ordering::Acquire));
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

#[cfg(windows)]
fn fixture_window(
    app: &tauri::AppHandle,
    label: &str,
    profile: &std::path::Path,
    url: &str,
) -> Result<tauri::WebviewWindow, String> {
    let (send, receive) = std::sync::mpsc::channel();
    let allowed = url.to_owned();
    let window = tauri::WebviewWindowBuilder::new(
        app,
        label,
        tauri::WebviewUrl::External(url.parse().unwrap()),
    )
    .data_directory(profile.into())
    .browser_extensions_enabled(false)
    .visible(false)
    .focused(false)
    .skip_taskbar(true)
    .inner_size(1.0, 1.0)
    .on_navigation(move |url| url.as_str() == allowed)
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .on_page_load(move |_, payload| {
        if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
            let _ = send.send(());
        }
    })
    .build()
    .map_err(|_| "create fixture window")?;
    if url != "about:blank" {
        receive
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "fixture load timeout")?;
    }
    Ok(window)
}

#[cfg(windows)]
fn fixture_status(view: &tauri::WebviewWindow) -> Result<serde_json::Value, String> {
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::HSTRING;
    let (send, receive) = std::sync::mpsc::channel();
    view.with_webview(move |platform| unsafe {
        let callback = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status, json| {
            let _ = send.send(if status.is_ok() { Ok(json) } else { Err("fixture evaluation".to_string()) });
            Ok(())
        }));
        let params = serde_json::json!({"expression":"fetch('/status',{credentials:'include',cache:'no-store'}).then(r=>r.json())", "awaitPromise":true, "returnByValue":true}).to_string();
        if let Ok(core) = platform.controller().CoreWebView2() { let _ = core.CallDevToolsProtocolMethod(&HSTRING::from("Runtime.evaluate"), &HSTRING::from(params), &callback); }
    }).map_err(|_| "dispatch fixture")?;
    let raw = receive
        .recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "fixture result timeout")??;
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| "fixture result shape")?;
    value
        .pointer("/result/value")
        .cloned()
        .ok_or("fixture result missing".into())
}
