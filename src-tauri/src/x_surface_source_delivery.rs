//! Pass-through resource observation plus trusted native public-script compilation.
//! No response override: that path crashes the current WebView2 renderer.
use super::*;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    io::Read,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};
use tauri::Manager;
use webview2_com::{
    CoTaskMemPWSTR, Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT,
    NavigationStartingEventHandler, WebResourceRequestedEventHandler,
};
use windows::core::{HSTRING, PWSTR};

type Cache = HashMap<String, Arc<String>>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
thread_local! {static PENDING:RefCell<HashMap<u64,webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Deferral>>=RefCell::new(HashMap::new());}
static NEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Default)]
struct LoadState {
    generation: u64,
    seen: HashSet<String>,
    pending: HashSet<u64>,
}
fn complete(id: u64) {
    let deferred = PENDING.with(|p| p.borrow_mut().remove(&id));
    if let Some(deferral) = deferred {
        unsafe {
            let _ = deferral.Complete();
        }
    }
}
fn fetch(url: &str) -> Option<Arc<String>> {
    if !public_asset(url) {
        return None;
    }
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(value) = cache
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(url)
        .cloned()
    {
        return Some(value);
    }
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .build()
        .ok()?;
    let response = client
        .get(url)
        .header("Accept-Encoding", "identity")
        .send()
        .ok()?;
    if response.status() != reqwest::StatusCode::OK
        || response.url().as_str() != url
        || response
            .content_length()
            .is_some_and(|n| n > MAX_SCRIPT as u64)
        || response
            .headers()
            .get("content-encoding")
            .is_some_and(|v| v != "identity")
        || !response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.starts_with("application/javascript") || v.starts_with("text/javascript")
            })
    {
        return None;
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_SCRIPT as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    let source = std::str::from_utf8(&bytes).ok()?;
    let fixed = repair_source(source)?;
    let fixed = Arc::new(fixed);
    let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
    if cache.len() >= 4 {
        cache.clear();
    }
    cache.insert(url.into(), fixed.clone());
    Some(fixed)
}
pub(super) fn install(
    view: &tauri::Webview,
    session: super::super::XSession,
    pane: Arc<super::super::Pane>,
) {
    if view.label() != pane.label || view.window().label() != "main" {
        return;
    }
    let app = view.app_handle().clone();
    let load = Arc::new(Mutex::new(LoadState::default()));
    let _=view.with_webview(move|platform|unsafe{
        let result=(||->windows::core::Result<()>{
            let core=platform.controller().CoreWebView2()?;
            let mut token=0;
            let navigation_load=load.clone();
            core.add_NavigationStarting(&NavigationStartingEventHandler::create(Box::new(move|_,_|{
                let pending={
                    let mut load=navigation_load.lock().unwrap_or_else(|p|p.into_inner());
                    load.generation=load.generation.wrapping_add(1);load.seen.clear();
                    load.pending.drain().collect::<Vec<_>>()
                };
                for id in pending {complete(id);}
                Ok(())
            })),&mut token)?;
            core.add_WebResourceRequested(&WebResourceRequestedEventHandler::create(Box::new(move|_,args|{
                let observe=(||->windows::core::Result<()>{
                    let Some(args)=args else{return Ok(());};
                    let mut context=COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT;args.ResourceContext(&mut context)?;
                    if context!=COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT{return Ok(());}
                    let request=args.Request()?;
                    let mut raw=PWSTR::null();request.Uri(&mut raw)?;let uri=CoTaskMemPWSTR::from(raw).to_string();
                    let mut raw=PWSTR::null();request.Method(&mut raw)?;
                    if CoTaskMemPWSTR::from(raw).to_string()!="GET"||!public_asset(&uri){return Ok(());}
                    let generation={
                        let mut load=load.lock().unwrap_or_else(|p|p.into_inner());
                        if load.seen.len()>=4||!load.seen.insert(uri.clone()){return Ok(());}
                        load.generation
                    };
                    #[cfg(debug_assertions)]
                    super::super::super::debug_shape("official-surface","source-request",&serde_json::json!({"kind":asset_kind(&uri)}));
                    let id=NEXT.fetch_add(1,Ordering::Relaxed);
                    let deferral=args.GetDeferral()?;
                    PENDING.with(|p|p.borrow_mut().insert(id,deferral));
                    load.lock().unwrap_or_else(|p|p.into_inner()).pending.insert(id);
                    let deadline_app=app.clone();
                    std::thread::spawn(move||{std::thread::sleep(Duration::from_millis(4500));let _=deadline_app.run_on_main_thread(move||complete(id));});
                    let app=app.clone();let session=session.clone();let pane=pane.clone();let load=load.clone();
                    tauri::async_runtime::spawn_blocking(move||{
                        let fixed=std::panic::catch_unwind(||fetch(&uri)).ok().flatten();
                        #[cfg(debug_assertions)]
                        super::super::super::debug_shape("official-surface","source-fetch",&serde_json::json!({"kind":asset_kind(&uri),"success":fixed.is_some()}));
                        let owned=app.clone();
                        let _=app.run_on_main_thread(move||{
                            let pending=PENDING.with(|p|p.borrow().contains_key(&id));
                            let same_load=load.lock().unwrap_or_else(|p|p.into_inner()).generation==generation;
                            let current=super::super::is_current(&session,&pane);
                            if !pending || !same_load || !current {
                                #[cfg(debug_assertions)]
                                super::super::super::debug_shape("official-surface","source-skipped",&serde_json::json!({"kind":asset_kind(&uri),"pending":pending,"sameLoad":same_load,"currentPane":current}));
                                complete(id);return;
                            }
                            let Some(view)=owned.get_webview(&pane.label)else{complete(id);return;};
                            if !view.url().is_ok_and(|url|super::super::allowed(&url)){complete(id);return;}
                            let nonce=serde_json::to_string(&pane.nonce).unwrap();
                            let Some(source)=fixed else{complete(id);return;};
                            let kind=serde_json::to_string(&asset_kind(&uri).unwrap()).unwrap();
                            let source_entry=source.contains("sourceMediaViewerAvailable");
                            let script=format!("(()=>{{const api=window.__atsumiXSourceCompat;const success=api?.deliver({nonce},()=>{{\n{source}\n}},{kind})===true;return JSON.stringify({{success,kind:{kind},entryAdapter:{source_entry},viewerReady:window.__atsumiXSurface?.sourceMediaViewerAvailable?.()===true,status:api?.status(),complete:document.readyState==='complete',nodes:document.body?.querySelectorAll('*').length||0}})}})()");
                            let _=view.with_webview(move|platform|{
                                let Ok(core)=platform.controller().CoreWebView2()else{complete(id);return;};
                                let dispatched=core.ExecuteScript(&HSTRING::from(script),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(move|status,value|{
                                    complete(id);
                                    let parsed=serde_json::from_str::<serde_json::Value>(&value).ok().and_then(|v|v.as_str().and_then(|s|serde_json::from_str::<serde_json::Value>(s).ok()));
                                    let success=status.is_ok()&&parsed.as_ref().is_some_and(|p|p["success"]==true);
                                    tracing::info!(diag_stage="x_source_autoplay_compat",diag_operation="public_script_compilation",success);
                                    #[cfg(debug_assertions)]
                                    super::super::super::debug_shape("official-surface","source-native-compilation",&serde_json::json!({"success":success,"shape":parsed}));
                                    Ok(())
                                })));
                                if dispatched.is_err(){complete(id);}
                            });
                        });
                    });Ok(())
                })();
                // SDK/property errors never fail the original HTTP request.
                // A bounded deferral orders compilation before network load;
                // neither headers nor response bytes are ever substituted.
                let _=observe;Ok(())
            })),&mut token)?;
            for pattern in ASSET_PATTERNS {core.AddWebResourceRequestedFilter(&HSTRING::from(pattern),COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT)?;}
            Ok(())
        })();
        if result.is_err(){tracing::warn!(diag_stage="x_source_autoplay_compat",diag_operation="install",success=false);}
    });
}
