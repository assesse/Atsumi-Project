//! Explicit Windows-only Native Messaging diagnostic for the installed NAVER extension.
//! Uses a fresh temporary WebView2 profile and about:blank; never opens a live
//! channel, logs in, reads browser cookies, starts a recording, or changes the
//! installed extension/registry. The only native request is NAVER's initial `{}`
//! one-shot connector ping, issued inside its own unmodified service worker.
//! cargo run --offline --example chzzk_extension_probe

#[cfg(not(windows))]
fn main() {
    eprintln!("chzzk_extension_probe requires Windows/WebView2");
}

#[cfg(windows)]
fn main() {
    if std::env::args_os().len() != 1 {
        eprintln!("This isolated probe accepts no URLs, paths, or account arguments");
        std::process::exit(2);
    }
    if let Err(code) = windows_probe::run() {
        eprintln!(
            "{}",
            serde_json::json!({"kind":"extension_probe_failed","code":code})
        );
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows_probe {
    use atsumi_lib::streaming::{browser::WINDOW_LABEL, browser_extension};
    use serde_json::{json, Value};
    use std::{sync::mpsc, time::Duration};
    use tauri::{Webview, WebviewUrl, WebviewWindowBuilder};

    // No response data, local paths, or diagnostic strings leave the extension
    // context. Only boolean capabilities and fixed error classifications return.
    const NATIVE_CHECK: &str = r#"(() => new Promise(resolve => {
      const runtime=globalThis.chrome && chrome.runtime;
      const api=!!runtime && typeof runtime.sendNativeMessage==='function';
      if(!api){resolve({apiAvailable:false,responded:false,code:'api_unavailable'});return;}
      let finished=false;
      const done=result=>{if(!finished){finished=true;clearTimeout(timer);resolve(result);}};
      const timer=setTimeout(()=>done({apiAvailable:true,responded:false,code:'timeout'}),3000);
      try{runtime.sendNativeMessage('nliveconnector_1.0.0.0',{},response=>{
        const error=runtime.lastError;
        const message=error && String(error.message).toLowerCase();
        let code='no_response';
        if(message){
          if(message.includes('not found'))code='host_not_found';
          else if(message.includes('forbidden') || message.includes('not allowed'))code='host_not_allowed';
          else if(message.includes('not supported'))code='unsupported';
          else if(message.includes('exited') || message.includes('disconnected'))code='host_disconnected';
          else code='native_error';
        }else if(response!==undefined){code='responded';}
        done({apiAvailable:true,responded:!error && response!==undefined,code});
      });}catch{done({apiAvailable:true,responded:false,code:'api_threw'});}
    }))()"#;

    /// Called only on the probe worker, never the WebView/UI thread.
    fn cdp(
        view: &Webview,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, &'static str> {
        use webview2_com::{
            CallDevToolsProtocolMethodCompletedHandler,
            Microsoft::Web::WebView2::Win32::ICoreWebView2_11,
        };
        use windows::core::{Interface, HSTRING};

        let (sender, receiver) = mpsc::sync_channel(1);
        let method = method.to_string();
        let params = params.to_string();
        let session = session.map(str::to_string);
        view.with_webview(move |platform| unsafe {
            let callback = sender.clone();
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                move |status, body| {
                    let result = if status.is_err() {
                        Err("cdp_method_failed")
                    } else if body.len() > 64 * 1024 {
                        Err("cdp_response_limit")
                    } else {
                        serde_json::from_str::<Value>(&body).map_err(|_| "cdp_invalid_json")
                    };
                    let _ = callback.try_send(result);
                    Ok(())
                },
            ));
            let result = (|| -> windows::core::Result<()> {
                let core = platform.controller().CoreWebView2()?;
                if let Some(session) = session {
                    core.cast::<ICoreWebView2_11>()?
                        .CallDevToolsProtocolMethodForSession(
                            &HSTRING::from(session),
                            &HSTRING::from(method),
                            &HSTRING::from(params),
                            &handler,
                        )
                } else {
                    core.CallDevToolsProtocolMethod(
                        &HSTRING::from(method),
                        &HSTRING::from(params),
                        &handler,
                    )
                }
            })();
            if result.is_err() {
                let _ = sender.try_send(Err("cdp_dispatch_failed"));
            }
        })
        .map_err(|_| "webview_dispatch_failed")?;
        receiver
            .recv_timeout(Duration::from_secs(6))
            .map_err(|_| "cdp_timeout")?
    }

    fn native_check(view: &Webview, id: &str) -> Value {
        // Target inspection is confined to this newly created, empty profile.
        // Only the exact installed extension service worker can be attached.
        let expected_url = format!("chrome-extension://{id}/background.js");
        let target = (0..6).find_map(|attempt| {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(250));
            }
            let targets = cdp(view, None, "Target.getTargets", json!({})).ok()?;
            targets
                .get("targetInfos")?
                .as_array()?
                .iter()
                .find(|target| {
                    target.get("type").and_then(Value::as_str) == Some("service_worker")
                        && target.get("url").and_then(Value::as_str) == Some(expected_url.as_str())
                })?
                .get("targetId")?
                .as_str()
                .map(str::to_string)
        });
        let Some(target) = target else {
            return json!({"id":id,"stage":"service_worker","code":"target_unavailable","nativeReady":false});
        };
        let attached = match cdp(
            view,
            None,
            "Target.attachToTarget",
            json!({"targetId":target,"flatten":true}),
        ) {
            Ok(result) => result,
            Err(code) => return json!({"id":id,"stage":"attach","code":code,"nativeReady":false}),
        };
        let Some(session) = attached.get("sessionId").and_then(Value::as_str) else {
            return json!({"id":id,"stage":"attach","code":"session_missing","nativeReady":false});
        };
        let result = cdp(
            view,
            Some(session),
            "Runtime.evaluate",
            json!({
                "expression":NATIVE_CHECK,"awaitPromise":true,"returnByValue":true
            }),
        );
        let _ = cdp(
            view,
            None,
            "Target.detachFromTarget",
            json!({"sessionId":session}),
        );
        match result {
            Ok(result) if result.get("exceptionDetails").is_none() => {
                let value = &result["result"]["value"];
                let code = value["code"].as_str().unwrap_or("invalid_result");
                let code = match code {
                    "api_unavailable" | "timeout" | "host_not_found" | "host_not_allowed"
                    | "unsupported" | "host_disconnected" | "native_error" | "responded"
                    | "no_response" | "api_threw" => code,
                    _ => "invalid_result",
                };
                json!({"id":id,"stage":"native_ping","code":code,
                    "apiAvailable":value["apiAvailable"].as_bool().unwrap_or(false),
                    "nativeReady":code=="responded" && value["responded"]==true})
            }
            Ok(_) => {
                json!({"id":id,"stage":"native_ping","code":"evaluation_exception","nativeReady":false})
            }
            Err(code) => json!({"id":id,"stage":"native_ping","code":code,"nativeReady":false}),
        }
    }

    pub fn run() -> Result<(), &'static str> {
        let directory = tempfile::Builder::new()
            .prefix("atsumi-extension-probe-")
            .tempdir()
            .map_err(|_| "temporary_profile_failed")?;
        let profile = directory.path().join("webview-profile");
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        context.config_mut().app.tray_icon = None;
        tauri::Builder::default().setup(move |app| {
            let deadline_app = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(60));
                eprintln!("{}", json!({"kind":"extension_probe_failed","code":"overall_deadline"}));
                deadline_app.exit(2);
            });
            let window = WebviewWindowBuilder::new(app, WINDOW_LABEL,
                WebviewUrl::External("about:blank".parse().unwrap()))
                .title("Atsumi isolated extension diagnostic")
                .visible(false)
                .data_directory(profile)
                .browser_extensions_enabled(true)
                .on_navigation(|url| url.as_str() == "about:blank")
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            let view: Webview = window.as_ref().clone();
            let (sender, receiver) = mpsc::sync_channel(1);
            browser_extension::connect(&view, move |result| { let _ = sender.try_send(result); })?;
            let app = app.handle().clone();
            std::thread::spawn(move || {
                let result = receiver.recv_timeout(Duration::from_secs(20));
                match result {
                    Ok(Ok(report)) => {
                        println!("{}", json!({"kind":"extension_probe_loaded","report":report,
                            "isolatedProfile":true,"page":"about:blank","recording":false}));
                        for id in &report.loaded_ids {
                            println!("{}", json!({"kind":"extension_native_probe","result":native_check(&view,id)}));
                        }
                    }
                    Ok(Err(error)) => println!("{}", json!({"kind":"extension_probe_failed","code":error.code})),
                    Err(_) => println!("{}", json!({"kind":"extension_probe_failed","code":"load_timeout"})),
                }
                app.exit(0);
            });
            Ok(())
        }).run(context).map_err(|_| "probe_runtime_failed")?;
        // This owns only the newly generated temporary profile, never user data.
        drop(directory);
        Ok(())
    }
}
