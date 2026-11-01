use super::{source_link_parse, Projection};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tauri::{Url, Webview};
const ERROR: &str = "게시물 링크 응답을 확인하지 못했습니다.";
#[derive(Clone, Debug, Default, serde::Serialize)]
pub(super) struct ObserverStats {
    pub responses: usize,
    pub body_reads: usize,
    pub parsed: bool,
    pub stage: &'static str,
}
#[cfg(windows)]
pub(super) fn observe(
    view: &Webview,
    source: Url,
    stop: Arc<AtomicBool>,
    payload: Arc<Mutex<Option<Projection>>>,
) -> Result<Arc<Mutex<ObserverStats>>, String> {
    use webview2_com::{
        CoTaskMemPWSTR, Microsoft::Web::WebView2::Win32::ICoreWebView2_2,
        WebResourceResponseReceivedEventHandler, WebResourceResponseViewGetContentCompletedHandler,
    };
    use windows::{
        core::{Interface, PWSTR},
        Win32::System::Com::{STATFLAG_NONAME, STATSTG},
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let stats = Arc::new(Mutex::new(ObserverStats::default()));
    let observed = stats.clone();
    view.with_webview(move |platform| unsafe {
        let result = (|| -> windows::core::Result<()> {
            let core = platform
                .controller()
                .CoreWebView2()?
                .cast::<ICoreWebView2_2>()?;
            let admitted = Arc::new(AtomicUsize::new(0));
            let mut token = 0;
            core.add_WebResourceResponseReceived(
                &WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
                    if stop.load(Ordering::Acquire)
                        || payload.lock().unwrap_or_else(|p| p.into_inner()).is_some()
                    {
                        return Ok(());
                    }
                    let Some(args) = args else {
                        return Ok(());
                    };
                    let request = args.Request()?;
                    let mut raw = PWSTR::null();
                    request.Uri(&mut raw)?;
                    let uri = CoTaskMemPWSTR::from(raw).to_string();
                    if !Url::parse(&uri).is_ok_and(|url| source_link_parse::detail_response(&url)) {
                        return Ok(());
                    }
                    {
                        let mut stats = observed.lock().unwrap_or_else(|p| p.into_inner());
                        stats.responses += 1;
                        stats.stage = "response";
                    }
                    let response = args.Response()?;
                    let mut status = 0;
                    response.StatusCode(&mut status)?;
                    if status != 200 {
                        return Ok(());
                    }
                    let headers = response.Headers()?;
                    let mut raw = PWSTR::null();
                    headers.GetHeader(&windows::core::HSTRING::from("Content-Type"), &mut raw)?;
                    let kind = CoTaskMemPWSTR::from(raw).to_string().to_ascii_lowercase();
                    if !kind.starts_with("application/json") {
                        return Ok(());
                    }
                    if admitted.fetch_add(1, Ordering::AcqRel) >= 4 {
                        return Ok(());
                    }
                    let source = source.clone();
                    let stop = stop.clone();
                    let payload = payload.clone();
                    let observed = observed.clone();
                    response.GetContent(
                        &WebResourceResponseViewGetContentCompletedHandler::create(Box::new(
                            move |status, stream| {
                                if status.is_err() || stop.load(Ordering::Acquire) {
                                    return Ok(());
                                }
                                {
                                    let mut stats =
                                        observed.lock().unwrap_or_else(|p| p.into_inner());
                                    stats.body_reads += 1;
                                    stats.stage = "body";
                                }
                                let Some(stream) = stream else {
                                    return Ok(());
                                };
                                let mut stat = STATSTG::default();
                                if stream.Stat(&mut stat, STATFLAG_NONAME).is_ok()
                                    && stat.cbSize > 2 * 1024 * 1024
                                {
                                    return Ok(());
                                }
                                let mut bytes = Vec::new();
                                let mut buffer = [0u8; 64 * 1024];
                                for _ in 0..33 {
                                    let mut count = 0;
                                    let result = stream.Read(
                                        buffer.as_mut_ptr().cast(),
                                        buffer.len() as u32,
                                        Some(&mut count),
                                    );
                                    if result.is_err() || count as usize > buffer.len() {
                                        return Ok(());
                                    }
                                    if count == 0 {
                                        break;
                                    }
                                    bytes.extend_from_slice(&buffer[..count as usize]);
                                    if bytes.len() > 2 * 1024 * 1024 {
                                        return Ok(());
                                    }
                                }
                                if stop.load(Ordering::Acquire) {
                                    return Ok(());
                                }
                                tauri::async_runtime::spawn_blocking(move || {
                                    if stop.load(Ordering::Acquire) {
                                        return;
                                    }
                                    let result = source_link_parse::parse(&bytes, &source);
                                    {
                                        let mut stats =
                                            observed.lock().unwrap_or_else(|p| p.into_inner());
                                        stats.parsed = result.is_ok();
                                        stats.stage =
                                            match result.as_ref().err().map(String::as_str) {
                                                None => "parsed",
                                                Some("root") => "root",
                                                Some("structure") => "structure",
                                                Some("json") => "json",
                                                Some(
                                                    "entries" | "instructions" | "module" | "size",
                                                ) => "limit",
                                                _ => "projection",
                                            };
                                    }
                                    if let Ok(projection) = result {
                                        if !stop.load(Ordering::Acquire) {
                                            *payload.lock().unwrap_or_else(|p| p.into_inner()) =
                                                Some(projection);
                                        }
                                    }
                                });
                                Ok(())
                            },
                        )),
                    )?;
                    Ok(())
                })),
                &mut token,
            )?;
            Ok(())
        })();
        let _ = tx.send(result.is_ok());
    })
    .map_err(|_| ERROR)?;
    if rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false) {
        Ok(stats)
    } else {
        Err(ERROR.into())
    }
}
#[cfg(not(windows))]
pub(super) fn observe(
    _: &Webview,
    _: Url,
    _: Arc<AtomicBool>,
    _: Arc<Mutex<Option<Projection>>>,
) -> Result<Arc<Mutex<ObserverStats>>, String> {
    Err(ERROR.into())
}
