//! Recover only the main document, never download workers or the process.
use crate::ui_diagnostics::UiDiagnostics;
use std::{sync::Arc, time::Duration};
use tauri::Manager;

pub(crate) fn request_reload(
    app: &tauri::AppHandle,
    reason: &'static str,
    epoch: String,
    hung: bool,
) {
    let Some(recorder) = app
        .try_state::<Arc<UiDiagnostics>>()
        .map(|s| s.inner().clone())
    else {
        return;
    };
    if !recorder.claim_reload(&epoch, reason) {
        return;
    }
    let app = app.clone();
    // Return from COM callbacks before touching the controller again.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        crate::ui_diagnostics::capture(&app, reason);
        tokio::time::sleep(Duration::from_millis(if hung { 2000 } else { 500 })).await;
        if app
            .try_state::<crate::interface::AppState>()
            .is_some_and(|s| s.is_quitting())
        {
            return;
        }
        if !recorder.may_reload(&epoch, hung) {
            return;
        }
        let Some(view) = app.get_webview("main") else {
            return;
        };
        let window = view.window();
        if hung
            && (!window.is_focused().unwrap_or(false)
                || !window.is_visible().unwrap_or(false)
                || window.is_minimized().unwrap_or(true))
        {
            recorder.cancel_reload(&epoch, "lost_foreground");
            return;
        }
        match view.reload() {
            Ok(()) => recorder.record(
                "reload_requested",
                serde_json::json!({"reason":reason,"epoch":epoch}),
            ),
            Err(error) => {
                recorder.cancel_reload(&epoch, "reload_error");
                tracing::error!(%error, "main renderer recovery reload failed");
            }
        }
    });
}

#[cfg(windows)]
pub(crate) fn install(view: &tauri::WebviewWindow) -> tauri::Result<()> {
    use webview2_com::{
        AcceleratorKeyPressedEventHandler,
        Microsoft::Web::WebView2::Win32::{
            ICoreWebView2ProcessFailedEventArgs2, COREWEBVIEW2_KEY_EVENT_KIND,
            COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
            COREWEBVIEW2_PROCESS_FAILED_KIND,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
            COREWEBVIEW2_PROCESS_FAILED_REASON,
        },
        ProcessFailedEventHandler,
    };
    use windows::core::Interface;
    let app = view.app_handle().clone();
    view.with_webview(move |platform| unsafe {
        let result = (|| -> windows::core::Result<()> {
            let controller = platform.controller();
            let core = controller.CoreWebView2()?;
            let process_app = app.clone();
            let mut token = 0;
            core.add_ProcessFailed(
                &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else {
                        return Ok(());
                    };
                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                    args.ProcessFailedKind(&mut kind)?;
                    let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
                    let mut exit_code = 0;
                    if let Ok(details) = args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
                        let _ = details.Reason(&mut reason);
                        let _ = details.ExitCode(&mut exit_code);
                    }
                    if let Some(recorder) = process_app.try_state::<Arc<UiDiagnostics>>() {
                        recorder.failed(
                            kind.0,
                            reason.0,
                            exit_code,
                            kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
                        );
                        if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED {
                            request_reload(
                                &process_app,
                                "renderer_exited",
                                recorder.epoch(),
                                false,
                            );
                        }
                    }
                    tracing::error!(
                        kind = kind.0,
                        reason = reason.0,
                        exit_code,
                        "main WebView process failed; backend workers remain running"
                    );
                    Ok(())
                })),
                &mut token,
            )?;
            controller.add_AcceleratorKeyPressed(
                &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                    // This synchronous callback must return immediately. Do not do
                    // disk IO, inspect the DOM, or record any text/ordinary keys.
                    let Some(args) = args else {
                        return Ok(());
                    };
                    let mut key = 0;
                    args.VirtualKey(&mut key)?;
                    if key == 0x74 {
                        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
                        args.KeyEventKind(&mut kind)?;
                        if kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                            || kind == COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
                        {
                            if let Some(recorder) = app.try_state::<Arc<UiDiagnostics>>() {
                                recorder.record(
                                    "native_f5",
                                    serde_json::json!({"epoch":recorder.epoch()}),
                                );
                            }
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(%error, "main renderer recovery could not be installed");
        }
    })
}

#[cfg(not(windows))]
pub(crate) fn install(_: &tauri::WebviewWindow) -> tauri::Result<()> {
    Ok(())
}
