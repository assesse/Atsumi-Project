//! Account troubleshooting is HTTP-stage/status only. No input, query, body,
//! cookie, username, challenge image/answer, or account identifier is recorded.
fn stage(value: &str) -> Option<&'static str> {
    let url = tauri::Url::parse(value).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("nid.naver.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let path = url.path().to_ascii_lowercase();
    if path.contains("captcha") {
        Some("chzzk_account_challenge_response")
    } else if path == "/nidlogin.login" {
        Some("chzzk_account_login_response")
    } else {
        None
    }
}

#[cfg(windows)]
pub(super) fn install(view: &tauri::Webview) {
    use webview2_com::{
        CoTaskMemPWSTR,
        Microsoft::Web::WebView2::Win32::{
            ICoreWebView2_2, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST,
        },
        WebResourceRequestedEventHandler, WebResourceResponseReceivedEventHandler,
    };
    use windows::core::{Interface, HSTRING, PWSTR};
    let _ = view.with_webview(|platform| unsafe {
        let result = (|| -> windows::core::Result<()> {
            let base = platform.controller().CoreWebView2()?;
            let core = base.cast::<ICoreWebView2_2>()?;
            let requested = std::sync::atomic::AtomicU32::new(0);
            let mut request_token = 0;
            base.add_WebResourceRequested(
                &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let request = args.Request()?;
                    let mut uri = PWSTR::null();
                    request.Uri(&mut uri)?;
                    let Some(response_stage) = stage(&CoTaskMemPWSTR::from(uri).to_string()) else {
                        return Ok(());
                    };
                    let mut method = PWSTR::null();
                    request.Method(&mut method)?;
                    let method = CoTaskMemPWSTR::from(method).to_string();
                    if !matches!(method.as_str(), "GET" | "POST") {
                        return Ok(());
                    }
                    let attempt = requested.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if attempt < 32 {
                        tracing::info!(
                            diag_stage = if response_stage == "chzzk_account_challenge_response" {
                                "chzzk_account_challenge_request"
                            } else {
                                "chzzk_account_login_request"
                            },
                            diag_operation = if method == "POST" {
                                "account_post"
                            } else {
                                "account_get"
                            },
                            attempt = attempt + 1,
                            "official account request started"
                        );
                    }
                    // Observation only. No body, header, credentials or response
                    // is read, modified, cancelled, copied or sent anywhere here.
                    Ok(())
                })),
                &mut request_token,
            )?;
            for context in [
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH,
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST,
            ] {
                base.AddWebResourceRequestedFilter(
                    &HSTRING::from("https://nid.naver.com/*"),
                    context,
                )?;
            }
            let received = std::sync::atomic::AtomicU32::new(0);
            let mut token = 0;
            core.add_WebResourceResponseReceived(
                &WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let request = args.Request()?;
                    let mut uri = PWSTR::null();
                    request.Uri(&mut uri)?;
                    let Some(diag_stage) = stage(&CoTaskMemPWSTR::from(uri).to_string()) else {
                        return Ok(());
                    };
                    let mut method = PWSTR::null();
                    request.Method(&mut method)?;
                    let method = CoTaskMemPWSTR::from(method).to_string();
                    if !matches!(method.as_str(), "GET" | "POST") {
                        return Ok(());
                    }
                    let mut status = 0;
                    args.Response()?.StatusCode(&mut status)?;
                    let attempt = received.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if attempt < 32 {
                        tracing::info!(
                            diag_stage,
                            status,
                            diag_operation = if method == "POST" {
                                "account_post"
                            } else {
                                "account_get"
                            },
                            attempt = attempt + 1,
                            "official account response received"
                        );
                    }
                    Ok(())
                })),
                &mut token,
            )
        })();
        if result.is_err() {
            tracing::warn!(
                diag_stage = "chzzk_account_diagnostics_unavailable",
                "account diagnostics unavailable"
            );
        }
    });
}
#[cfg(not(windows))]
pub(super) fn install(_: &tauri::Webview) {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_and_query_strings_cannot_enter_stages() {
        assert_eq!(
            stage("https://nid.naver.com/login/ext/captcha?answer=private"),
            Some("chzzk_account_challenge_response")
        );
        assert_eq!(
            stage("https://nid.naver.com/nidlogin.login?private=value"),
            Some("chzzk_account_login_response")
        );
        for url in [
            "https://nid.naver.com.evil/login/ext/captcha",
            "https://nid.naver.com/other",
            "https://nid.naver.com:444/nidlogin.login",
            "http://nid.naver.com/nidlogin.login",
            "https://user@nid.naver.com/nidlogin.login",
        ] {
            assert_eq!(stage(url), None);
        }
    }
}
