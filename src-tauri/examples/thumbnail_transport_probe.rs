//! Isolated native IPC comparison. Synthetic images and temporary profile only;
//! never constructs AppState, opens the real DB, or connects to remote hosts.
#[cfg(not(windows))]
fn main() {}
#[cfg(windows)]
fn main() {
    native::run();
}
#[cfg(windows)]
mod native {
    use atsumi_lib::thumbnail::{
        transport::ThumbnailTransport, ResolvedThumbnail, ThumbnailCacheStatus,
        ThumbnailCompletionEventDto, ThumbnailDeliveryDto, ThumbnailKey, ThumbnailRequestTokenDto,
    };
    use serde_json::{json, Value};
    use std::{
        io::{Read, Write},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };
    use tauri::{Emitter, State};
    struct Probe {
        image: Vec<u8>,
        transport: ThumbnailTransport,
    }
    #[tauri::command]
    fn probe_request(
        view: tauri::WebviewWindow,
        state: State<'_, Probe>,
        id: String,
        mode: String,
    ) -> Result<(), String> {
        let key = ThumbnailKey::gallery_cover(1).unwrap();
        let event = ThumbnailCompletionEventDto::from_result(
            ThumbnailRequestTokenDto {
                request_id: id,
                key: key.clone(),
            },
            Ok(ThumbnailDeliveryDto {
                key,
                cache_status: ThumbnailCacheStatus::Resolved,
                thumbnail: ResolvedThumbnail {
                    content_type: "image/webp".into(),
                    bytes: state.image.clone(),
                    width: 512,
                    height: 512,
                    source_revision: None,
                },
            }),
        );
        if mode == "legacy" {
            view.emit("probe:body", event).map_err(|e| e.to_string())
        } else {
            view.emit(
                "probe:body",
                state
                    .transport
                    .publish(&state.transport.epoch(), event)
                    .unwrap(),
            )
            .map_err(|e| e.to_string())
        }
    }
    #[tauri::command]
    fn probe_read(state: State<'_, Probe>, token: String) -> Result<tauri::ipc::Response, String> {
        state
            .transport
            .read(&token)
            .map(tauri::ipc::Response::new)
            .ok_or("expired".into())
    }
    #[tauri::command]
    fn probe_release(state: State<'_, Probe>, token: String) {
        state.transport.release(&token);
    }

    #[cfg(windows)]
    fn cdp(view: &tauri::WebviewWindow, method: &str) -> Value {
        use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
        use windows::core::HSTRING;
        let method = method.to_owned();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        view.with_webview(move |platform| unsafe {
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                move |status, text| {
                    let _ = tx.send(if status.is_ok() { text } else { "null".into() });
                    Ok(())
                },
            ));
            let _ = platform
                .controller()
                .CoreWebView2()
                .unwrap()
                .CallDevToolsProtocolMethod(&HSTRING::from(method), &HSTRING::from("{}"), &handler);
        })
        .unwrap();
        serde_json::from_str(
            &rx.recv_timeout(Duration::from_secs(5))
                .unwrap_or("null".into()),
        )
        .unwrap_or(Value::Null)
    }
    #[cfg(windows)]
    fn process_memory(view: &tauri::WebviewWindow) -> Value {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2Environment8, ICoreWebView2_2, COREWEBVIEW2_PROCESS_KIND,
        };
        use windows::{
            core::Interface,
            Win32::{
                Foundation::CloseHandle,
                System::{
                    ProcessStatus::{
                        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
                    },
                    Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
                },
            },
        };
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        view.with_webview(move |platform| unsafe {
            let core = platform
                .controller()
                .CoreWebView2()
                .unwrap()
                .cast::<ICoreWebView2_2>()
                .unwrap();
            let infos = core
                .Environment()
                .unwrap()
                .cast::<ICoreWebView2Environment8>()
                .unwrap()
                .GetProcessInfos()
                .unwrap();
            let mut count = 0;
            infos.Count(&mut count).unwrap();
            let mut rows = Vec::new();
            for i in 0..count {
                let info = infos.GetValueAtIndex(i).unwrap();
                let mut pid = 0;
                let mut kind = COREWEBVIEW2_PROCESS_KIND::default();
                info.ProcessId(&mut pid).unwrap();
                info.Kind(&mut kind).unwrap();
                if let Ok(handle) = OpenProcess(
                    PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
                    false,
                    pid as u32,
                ) {
                    let mut memory = PROCESS_MEMORY_COUNTERS_EX::default();
                    memory.cb = std::mem::size_of_val(&memory) as u32;
                    if GetProcessMemoryInfo(
                        handle,
                        &mut memory as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
                        memory.cb,
                    )
                    .is_ok()
                    {
                        rows.push(json!({"kind":kind.0,"privateBytes":memory.PrivateUsage}));
                    }
                    let _ = CloseHandle(handle);
                }
            }
            let _ = tx.send(json!(rows));
        })
        .unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .unwrap_or(Value::Null)
    }
    #[tauri::command]
    async fn probe_sample(
        view: tauri::WebviewWindow,
        phase: String,
        elapsed_ms: f64,
    ) -> Result<(), String> {
        tauri::async_runtime::spawn_blocking(move||{
        let before=cdp(&view,"Runtime.getHeapUsage"); let memory=process_memory(&view);
        cdp(&view,"HeapProfiler.collectGarbage");
        let after=cdp(&view,"Runtime.getHeapUsage");
        println!("PROBE_SAMPLE {}",json!({"phase":phase,"elapsedMs":elapsed_ms,"heapBeforeGc":before,"heapAfterGc":after,"processes":memory}));
    }).await.map_err(|e|e.to_string())
    }
    #[tauri::command]
    fn probe_done(app: tauri::AppHandle, state: State<'_, Probe>, ok: bool) {
        println!(
            "PROBE_DONE {}",
            json!({"ok":ok,"transport":state.transport.stats()})
        );
        app.exit(if ok { 0 } else { 1 });
    }
    pub fn run() {
        use image::ImageEncoder;
        let mut seed = 17_u32;
        let pixels: Vec<u8> = (0..512 * 512 * 3)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as u8
            })
            .collect();
        let mut bytes = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
            .write_image(&pixels, 512, 512, image::ExtendedColorType::Rgb8)
            .unwrap();
        println!(
            "PROBE_PAYLOAD {}",
            json!({"binaryBytes":bytes.len(),"numericArrayJsonBytes":serde_json::to_vec(&bytes).unwrap().len()})
        );
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        server.set_nonblocking(true).unwrap();
        let url = format!("http://127.0.0.1:{}/", server.local_addr().unwrap().port());
        let finished = Arc::new(AtomicBool::new(false));
        let serving = finished.clone();
        std::thread::spawn(move || {
            while !serving.load(Ordering::Relaxed) {
                match server.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                        let mut request = [0; 2048];
                        let _ = stream.read(&mut request);
                        let body="<!doctype html><html><head><title>Isolated transport test</title></head><body></body></html>";
                        let _=write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        let watchdog = finished.clone();
        std::thread::spawn(move || {
            for _ in 0..90 {
                if watchdog.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            eprintln!("PROBE_TIMEOUT");
            std::process::exit(3);
        });
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("probe-profile");
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        context.config_mut().app.tray_icon = None;
        context.config_mut().app.with_global_tauri = true;
        context.config_mut().build.dev_url = Some(url.parse().unwrap());
        tauri::Builder::default()
            .manage(Probe {
                image: bytes,
                transport: ThumbnailTransport::default(),
            })
            .invoke_handler(tauri::generate_handler![
                probe_request,
                probe_read,
                probe_release,
                probe_sample,
                probe_done
            ])
            .setup(move |app| {
                tauri::WebviewWindowBuilder::new(
                    app,
                    "main",
                    tauri::WebviewUrl::External(url.parse().unwrap()),
                )
                .title("Isolated image transport probe")
                .visible(false)
                .focused(false)
                .skip_taskbar(true)
                .data_directory(profile)
                .initialization_script(include_str!("thumbnail_transport_probe.js"))
                .build()?;
                Ok(())
            })
            .run(context)
            .expect("probe runtime");
        finished.store(true, Ordering::Relaxed);
    }
}
