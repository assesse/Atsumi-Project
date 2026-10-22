//! A short-lived, JSON-only browser document. The browser retains session cookies;
//! only bounded public content metadata leaves this SOOP-owned profile.
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const URL: &str = "https://live.sooplive.com/api/myplus/preferbjLiveVodController.php";
const ERROR: &str = "로그인 기반 추천을 불러오지 못했습니다. 인기 콘텐츠를 표시합니다.";
const MAX_RESULT: usize = 256 * 1024;

fn allowed(url: &tauri::Url) -> bool {
    url.as_str() == URL || url.as_str() == "about:blank"
}

fn projection(nonce: &str) -> String {
    include_str!("discovery_session.js").replace(
        "__ATSUMI_SOOP_DISCOVERY_NONCE__",
        &serde_json::json!(nonce).to_string(),
    )
}

/// Blocking worker only. Never creates a player, reads cookies or sends account actions.
pub(super) fn preferred(app: &AppHandle) -> Result<Value, String> {
    let nonce = uuid::Uuid::new_v4().to_string();
    let label = format!("soop-discovery-{nonce}");
    let path = super::original::profile(app)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel_create = cancelled.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let owned_app = app.clone();
    let owned_label = label.clone();
    app.run_on_main_thread(move || {
        if cancel_create.load(Ordering::Acquire) {
            return;
        }
        let created = WebviewWindowBuilder::new(
            &owned_app,
            &owned_label,
            WebviewUrl::External(URL.parse().unwrap()),
        )
        .title("SOOP 탐색 조회")
        .visible(false)
        .focused(false)
        .skip_taskbar(true)
        .data_directory(path)
        .browser_extensions_enabled(false)
        .on_navigation(allowed)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .build();
        if cancel_create.load(Ordering::Acquire) {
            if let Ok(window) = created {
                let _ = window.close();
            }
            return;
        }
        let _ = tx.send(created.map(|_| ()).map_err(|_| ERROR.to_string()));
    })
    .map_err(|_| ERROR)?;
    let result = (|| {
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| ERROR)??;
        let window = app.get_webview_window(&label).ok_or(ERROR)?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(12) {
            if window.url().map_err(|_| ERROR)?.as_str() != URL {
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            window
                .eval_with_callback(projection(&nonce), move |raw| {
                    let _ = tx.send(raw);
                })
                .map_err(|_| ERROR)?;
            let raw = rx.recv_timeout(Duration::from_secs(2)).map_err(|_| ERROR)?;
            if window.url().map_err(|_| ERROR)?.as_str() != URL {
                return Err(ERROR.into());
            }
            if raw.len() > MAX_RESULT {
                return Err(ERROR.into());
            }
            let value: Value = serde_json::from_str(&raw).map_err(|_| ERROR)?;
            if value["nonce"] != nonce {
                return Err(ERROR.into());
            }
            if value["state"] == "ready" {
                return Ok(value);
            }
            if value["state"] == "unavailable" {
                return Err(ERROR.into());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(ERROR.into())
    })();
    cancelled.store(true, Ordering::Release);
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.close();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_json_document_can_navigate() {
        assert!(allowed(&URL.parse().unwrap()));
        for u in [
            "https://live.sooplive.com/",
            "https://live.sooplive.com/api/myplus/preferbjLiveVodController.php?token=x",
            "http://live.sooplive.com/api/myplus/preferbjLiveVodController.php",
            "https://evil.test/",
        ] {
            assert!(!allowed(&u.parse().unwrap()));
        }
    }
    #[test]
    fn projection_has_no_cookie_or_account_read() {
        let script = projection("bounded-test");
        for forbidden in [
            "document.cookie",
            "localStorage",
            "sessionStorage",
            "fetch(",
            "XMLHttpRequest",
            "document.querySelector",
            "document.querySelectorAll",
            "document.forms",
            "getElementById",
            "HTMLInputElement",
            "outerHTML",
        ] {
            assert!(!script.contains(forbidden), "{forbidden}");
        }
        assert!(script.contains("bounded-test"));
        assert!(script.contains("slice(0, 40)"));
        // This public content gate is not an account password read. It must
        // remain projected so protected broadcasts cannot appear unrestricted.
        assert!(script.contains("clean(\"is_password\""));
    }
}
