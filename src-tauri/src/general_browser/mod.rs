//! An independent browser process: no AppState, application DB, recording or download supervisor.
#[cfg(debug_assertions)]
mod fixture;
mod library;
mod proxy;
mod vpn;
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{
    webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder},
    Emitter, Manager, WebviewUrl, WebviewWindowBuilder,
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tab {
    id: String,
    url: String,
    title: String,
    loading: bool,
    #[serde(default)]
    can_go_back: bool,
    #[serde(default)]
    can_go_forward: bool,
}
#[derive(Serialize, Deserialize, Default)]
struct Session {
    tabs: Vec<Tab>,
    active: Option<usize>,
    #[serde(default)]
    closed: Vec<Tab>,
}
fn restore_session(root: &std::path::Path) -> Session {
    let Some(bytes) = std::fs::read(root.join("tabs.json"))
        .ok()
        .filter(|b| b.len() < 512 * 1024)
    else {
        return Session::default();
    };
    serde_json::from_slice::<Session>(&bytes)
        .or_else(|_| {
            serde_json::from_slice::<Vec<Tab>>(&bytes).map(|tabs| Session {
                active: tabs.len().checked_sub(1),
                tabs,
                closed: Vec::new(),
            })
        })
        .unwrap_or_default()
}
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Network {
    enabled: bool,
    fragment: bool,
    phase: String,
    ip: Option<String>,
    country: Option<String>,
    profile: Option<String>,
    #[serde(default)]
    profile_id: Option<String>,
    error: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    tabs: Vec<Tab>,
    active: Option<String>,
    network: Network,
    profiles: Vec<vpn::ProfileSummary>,
    downloads: usize,
    notice: Option<String>,
    preferences: library::Preferences,
    password_saving_available: bool,
    maximized: bool,
    recently_closed: usize,
}
struct Browser {
    gateway: Arc<proxy::Gateway>,
    root: PathBuf,
    library: Option<library::Store>,
    preferences: Mutex<library::Preferences>,
    password_saving_available: AtomicBool,
    maximized: AtomicBool,
    closed: Mutex<Vec<Tab>>,
    engine: Mutex<Option<vpn::Engine>>,
    network: Mutex<Network>,
    profiles: Mutex<Vec<vpn::ProfileSummary>>,
    tabs: Mutex<Vec<Tab>>,
    active: Mutex<Option<String>>,
    busy: AtomicBool,
    downloads: Mutex<std::collections::HashMap<String, usize>>,
    notice: Mutex<Option<String>>,
    bounds: Mutex<(f64, f64, f64, f64)>,
}
impl Browser {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            tabs: self.tabs.lock().unwrap().clone(),
            active: self.active.lock().unwrap().clone(),
            network: self.network.lock().unwrap().clone(),
            profiles: self.profiles.lock().unwrap().clone(),
            downloads: self.downloads.lock().unwrap().values().sum(),
            notice: self.notice.lock().unwrap().clone(),
            preferences: self.preferences.lock().unwrap().clone(),
            password_saving_available: self.password_saving_available.load(Ordering::Acquire),
            maximized: self.maximized.load(Ordering::Acquire),
            recently_closed: self.closed.lock().unwrap().len(),
        }
    }
    fn emit(&self, app: &tauri::AppHandle) {
        let _ = app.emit_to("main", "browser:state", self.snapshot());
    }
    fn save_tabs(&self) {
        let tabs = self.tabs.lock().unwrap().clone();
        let selected = self.active.lock().unwrap().clone();
        let active = tabs.iter().position(|t| Some(&t.id) == selected.as_ref());
        let data = Session {
            tabs,
            active,
            closed: self.closed.lock().unwrap().clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&data) {
            let pending = self.root.join("tabs.pending");
            if std::fs::write(&pending, bytes).is_ok() {
                let _ = vpn::replace(&pending, &self.root.join("tabs.json"));
            }
        }
    }
    fn layout(&self, app: &tauri::AppHandle) {
        let active = self.active.lock().unwrap().clone();
        let (x, y, w, h) = *self.bounds.lock().unwrap();
        let ids: Vec<_> = self
            .tabs
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.id.clone())
            .collect();
        for id in ids {
            if let Some(view) = app.get_webview(&id) {
                if Some(&id) == active.as_ref() && w > 0.0 && h > 0.0 {
                    let _ = view.set_position(tauri::LogicalPosition::new(x, y));
                    let _ = view.set_size(tauri::LogicalSize::new(w.max(1.0), h.max(1.0)));
                    let _ = view.show();
                } else {
                    let _ = view.hide();
                }
            }
        }
    }
}
fn trusted(view: &tauri::Webview) -> Result<(), String> {
    let url = view
        .url()
        .map_err(|_| "브라우저 요청을 확인하지 못했습니다.")?;
    if crate::frontend_origin::trusted_main(
        view.label(),
        &url,
        cfg!(debug_assertions),
        view.app_handle().config().build.dev_url.as_ref(),
    ) {
        Ok(())
    } else {
        Err("허용되지 않은 브라우저 요청입니다.".into())
    }
}
fn address(input: &str) -> Result<tauri::Url, String> {
    let input = input.trim();
    if input.is_empty() || input == "about:blank" {
        return Ok("about:blank".parse().unwrap());
    }
    let value = if input.contains("://") {
        input.to_string()
    } else if !input.chars().any(char::is_whitespace) && input.contains('.') {
        format!("https://{input}")
    } else {
        let mut url = tauri::Url::parse("https://www.google.com/search").unwrap();
        url.query_pairs_mut().append_pair("q", input);
        url.to_string()
    };
    let url = tauri::Url::parse(&value).map_err(|_| "주소를 확인해 주세요.")?;
    if !allowed(&url) {
        return Err("공개 웹사이트의 http 또는 https 주소를 입력해 주세요.".into());
    }
    Ok(url)
}
fn allowed(url: &tauri::Url) -> bool {
    #[cfg(debug_assertions)]
    if let Some(qa) = fixture::current() {
        return url.as_str() == "about:blank"
            || url.origin().ascii_serialization() == qa.origin
                && url.username().is_empty()
                && url.password().is_none();
    }
    url.as_str() == "about:blank"
        || matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some_and(proxy::public_host)
}
const WEB_ARGS:&str="--disable-quic --force-webrtc-ip-handling-policy=disable_non_proxied_udp --proxy-bypass-list=<-loopback> --disable-features=DnsOverHttpsUpgrade --disable-background-networking";
fn new_tab(app: &tauri::AppHandle, input: &str, activate: bool) -> Result<String, String> {
    let url = address(input)?;
    let state = app.state::<Arc<Browser>>().inner().clone();
    if state.tabs.lock().unwrap().len() >= 30 {
        return Err("열린 탭이 30개입니다. 사용하지 않는 탭을 닫아 주세요.".into());
    }
    let id = format!("web-{}", uuid::Uuid::new_v4().simple());
    state.tabs.lock().unwrap().push(Tab {
        id: id.clone(),
        url: url.to_string(),
        title: "새 탭".into(),
        loading: false,
        can_go_back: false,
        can_go_forward: false,
    });
    let nav_state = state.clone();
    let nav_app = app.clone();
    let nav_id = id.clone();
    let title_state = state.clone();
    let title_app = app.clone();
    let title_id = id.clone();
    let load_state = state.clone();
    let load_app = app.clone();
    let load_id = id.clone();
    let popup_app = app.clone();
    let dl_state = state.clone();
    let dl_app = app.clone();
    let builder = WebviewBuilder::new(&id, WebviewUrl::External(url))
        .data_directory(state.root.join("web-profile"))
        .browser_extensions_enabled(false)
        .additional_browser_args(WEB_ARGS);
    #[cfg(debug_assertions)]
    let fixture_only = fixture::current().is_some();
    #[cfg(not(debug_assertions))]
    let fixture_only = false;
    let builder = if fixture_only {
        builder
    } else {
        builder.proxy_url(
            format!("http://127.0.0.1:{}", state.gateway.port)
                .parse()
                .unwrap(),
        )
    };
    let builder = builder
        .on_navigation(move |url| {
            if !allowed(url) {
                return false;
            }
            if let Some(tab) = nav_state
                .tabs
                .lock()
                .unwrap()
                .iter_mut()
                .find(|t| t.id == nav_id)
            {
                tab.url = url.to_string();
            }
            nav_state.save_tabs();
            nav_state.emit(&nav_app);
            true
        })
        .on_document_title_changed(move |_, title| {
            if let Some(tab) = title_state
                .tabs
                .lock()
                .unwrap()
                .iter_mut()
                .find(|t| t.id == title_id)
            {
                tab.title = title.chars().take(160).collect();
            }
            if let Some(store) = &title_state.library {
                let url = title_state
                    .tabs
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|t| t.id == title_id)
                    .map(|t| t.url.clone());
                if let Some(url) = url {
                    store.title(&url, &title);
                }
            }
            title_state.save_tabs();
            title_state.emit(&title_app);
        })
        .on_page_load(move |_, payload| {
            if let Some(tab) = load_state
                .tabs
                .lock()
                .unwrap()
                .iter_mut()
                .find(|t| t.id == load_id)
            {
                tab.loading = payload.event() == PageLoadEvent::Started;
            }
            load_state.emit(&load_app);
        })
        .on_new_window(move |url, _| {
            if allowed(&url) {
                let app = popup_app.clone();
                let other = app.clone();
                let _ = app.run_on_main_thread(move || {
                    let _ = new_tab(&other, url.as_str(), true);
                });
            }
            NewWindowResponse::Deny
        })
        .on_download(move |_, event| {
            match event {
                DownloadEvent::Requested { url, .. } => {
                    let mut downloads = dl_state.downloads.lock().unwrap();
                    if dl_state.busy.load(Ordering::Acquire) {
                        return false;
                    }
                    *downloads.entry(url.to_string()).or_default() += 1;
                    drop(downloads);
                    *dl_state.notice.lock().unwrap() =
                        Some("브라우저에서 파일을 받는 중입니다.".into());
                }
                DownloadEvent::Finished { url, success, .. } => {
                    {
                        let mut downloads = dl_state.downloads.lock().unwrap();
                        if let Some(count) = downloads.get_mut(url.as_str()) {
                            *count -= 1;
                            if *count == 0 {
                                downloads.remove(url.as_str());
                            }
                        }
                    }
                    *dl_state.notice.lock().unwrap() = Some(
                        if success {
                            "다운로드 완료"
                        } else {
                            "다운로드가 중단됐습니다. 다시 시도해 주세요."
                        }
                        .into(),
                    );
                }
                _ => {}
            }
            dl_state.emit(&dl_app);
            true
        });
    let result = app
        .get_window("main")
        .ok_or("브라우저 창을 찾지 못했습니다.")?
        .add_child(
            builder,
            tauri::LogicalPosition::new(0.0, 130.0),
            tauri::LogicalSize::new(900.0, 600.0),
        );
    let view = match result {
        Ok(view) => view,
        Err(_) => {
            state.tabs.lock().unwrap().retain(|t| t.id != id);
            return Err("탭을 열지 못했습니다.".into());
        }
    };
    install_shortcuts(&view);
    install_browser_features(&view);
    if activate {
        *state.active.lock().unwrap() = Some(id.clone());
    }
    state.layout(app);
    state.save_tabs();
    state.emit(app);
    Ok(id)
}
#[tauri::command]
pub async fn general_browser_open(
    app: tauri::AppHandle,
    webview: tauri::Webview,
) -> Result<(), String> {
    trusted(&webview)?;
    let source = std::env::current_exe().map_err(|_| "브라우저 실행 파일을 찾지 못했습니다.")?;
    let root = app
        .path()
        .data_dir()
        .map_err(|_| "브라우저 경로를 찾지 못했습니다.")?
        .join("local.atsumi.browser/runtime");
    let resources = app
        .path()
        .resource_dir()
        .map_err(|_| "브라우저 도구 경로를 찾지 못했습니다.")?;
    tauri::async_runtime::spawn_blocking(move || {
        // A private copy avoids locking the main executable during app updates/development builds.
        std::fs::create_dir_all(&root).map_err(|_| "브라우저 실행 폴더를 준비하지 못했습니다.")?;
        let exe = root.join("atsumi-browser.exe");
        copy_runtime_file(&source, &exe)?;
        if !cfg!(debug_assertions) {
            let tools = root.join("media-tools/browser");
            std::fs::create_dir_all(&tools)
                .map_err(|_| "브라우저 도구 폴더를 준비하지 못했습니다.")?;
            for name in ["wireproxy.exe", "wireproxy-LICENSE.txt"] {
                copy_runtime_file(
                    &resources.join("media-tools/browser").join(name),
                    &tools.join(name),
                )?;
            }
        }
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--atsumi-browser");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd.spawn().map_err(|_| "브라우저를 열지 못했습니다.")?;
        Ok(())
    })
    .await
    .map_err(|_| "브라우저를 준비하지 못했습니다.")?
}
fn copy_runtime_file(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    fn digest(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
        let mut file = std::fs::File::open(path)?;
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash)?;
        Ok(hash.finalize().to_vec())
    }
    let expected = digest(source).map_err(|_| "브라우저 실행 파일을 읽지 못했습니다.")?;
    if digest(destination).ok().as_ref() == Some(&expected) {
        return Ok(());
    }
    let temporary = destination.with_extension("pending");
    std::fs::copy(source, &temporary)
        .map_err(|_| "브라우저 실행 파일을 준비하지 못했습니다. 저장 공간을 확인해 주세요.")?;
    if digest(&temporary).ok().as_ref() != Some(&expected) {
        return Err("브라우저 실행 파일 확인에 실패했습니다.".into());
    }
    vpn::replace(&temporary, destination).map_err(|_| {
        "열린 브라우저를 닫고 다시 열어 주세요. Atsumi 녹화와 다운로드는 계속됩니다.".into()
    })
}
#[tauri::command]
fn browser_snapshot(
    webview: tauri::Webview,
    state: tauri::State<'_, Arc<Browser>>,
) -> Result<Snapshot, String> {
    trusted(&webview)?;
    Ok(state.snapshot())
}
#[tauri::command]
async fn browser_tab(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    action: String,
    id: Option<String>,
    url: Option<String>,
) -> Result<(), String> {
    trusted(&webview)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let other = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| {
            let state = other.state::<Arc<Browser>>();
            match action.as_str() {
                "new" => {
                    new_tab(&other, url.as_deref().unwrap_or("about:blank"), true)?;
                }
                "select" => {
                    let id = id.ok_or("탭을 찾지 못했습니다.")?;
                    if state.tabs.lock().unwrap().iter().any(|t| t.id == id) {
                        *state.active.lock().unwrap() = Some(id);
                        state.layout(&other);
                    }
                }
                "restore" => {
                    let tab = state.closed.lock().unwrap().pop();
                    if let Some(tab) = tab {
                        if let Err(e) = new_tab(&other, &tab.url, true) {
                            state.closed.lock().unwrap().push(tab);
                            return Err(e);
                        }
                    }
                }
                "close" => {
                    let id = id.ok_or("탭을 찾지 못했습니다.")?;
                    let closing = state
                        .tabs
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|t| t.id == id)
                        .cloned();
                    if let Some(v) = other.get_webview(&id) {
                        v.close().map_err(|_| "탭을 닫지 못했습니다.")?;
                    }
                    if let Some(tab) = closing.filter(|t| t.url != "about:blank") {
                        let mut closed = state.closed.lock().unwrap();
                        closed.push(tab);
                        if closed.len() > 20 {
                            closed.remove(0);
                        }
                    }
                    state.tabs.lock().unwrap().retain(|t| t.id != id);
                    if state.active.lock().unwrap().as_ref() == Some(&id) {
                        *state.active.lock().unwrap() =
                            state.tabs.lock().unwrap().last().map(|t| t.id.clone());
                    }
                    if state.tabs.lock().unwrap().is_empty() {
                        new_tab(&other, "about:blank", true)?;
                    }
                    state.layout(&other);
                }
                "go" | "back" | "forward" | "reload" => {
                    let current = state
                        .active
                        .lock()
                        .unwrap()
                        .clone()
                        .ok_or("열린 탭이 없습니다.")?;
                    let view = other.get_webview(&current).ok_or("탭을 찾지 못했습니다.")?;
                    match action.as_str() {
                        "go" => view.navigate(address(url.as_deref().unwrap_or(""))?),
                        "back" => view.eval("history.back()"),
                        "forward" => view.eval("history.forward()"),
                        _ => view.reload(),
                    }
                    .map_err(|_| "페이지를 열지 못했습니다.")?;
                }
                _ => return Err("알 수 없는 브라우저 동작입니다.".into()),
            }
            state.save_tabs();
            state.emit(&other);
            Ok(())
        })();
        let _ = sender.send(result);
    })
    .map_err(|_| "브라우저가 응답하지 않습니다.")?;
    receiver
        .await
        .map_err(|_| "브라우저가 응답하지 않습니다.")?
}
#[tauri::command]
fn browser_bounds(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: tauri::State<'_, Arc<Browser>>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    trusted(&webview)?;
    if [x, y, width, height]
        .iter()
        .any(|v| !v.is_finite() || *v < 0.0 || *v > 20000.0)
    {
        return Err("잘못된 브라우저 크기입니다.".into());
    }
    *state.bounds.lock().unwrap() = (x, y, width, height);
    state.layout(&app);
    Ok(())
}
#[tauri::command]
async fn browser_import(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: tauri::State<'_, Arc<Browser>>,
) -> Result<(), String> {
    trusted(&webview)?;
    let state = state.inner().clone();
    if state.busy.swap(true, Ordering::AcqRel) {
        return Err("연결 설정을 변경하는 중입니다.".into());
    }
    if !state.downloads.lock().unwrap().is_empty() {
        state.busy.store(false, Ordering::Release);
        return Err("브라우저 다운로드가 끝난 뒤 국가를 추가해 주세요.".into());
    }
    let worker = state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = vpn::pick()? else {
            return Ok(());
        };
        if path
            .extension()
            .and_then(|s| s.to_str())
            .is_none_or(|s| !s.eq_ignore_ascii_case("conf"))
        {
            return Err("WireGuard .conf 파일을 선택해 주세요.".into());
        }
        let file = std::fs::File::open(&path).map_err(|_| "설정 파일을 열지 못했습니다.")?;
        let mut text = String::new();
        file.take(32769)
            .read_to_string(&mut text)
            .map_err(|_| "설정 파일을 읽지 못했습니다.")?;
        let config = vpn::sanitize(&text)?;
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .chars()
            .filter(|c| !c.is_control())
            .take(80)
            .collect();
        let mut profiles = vpn::load_profiles(&worker.root)?;
        let id = profiles.add(vpn::Profile { name, config })?;
        vpn::save_profiles(&worker.root, &profiles)?;
        // Adding a country must not replace or disconnect the selected profile.
        *worker.profiles.lock().unwrap() = profiles.summaries();
        let mut network = worker.network.lock().unwrap();
        if network.profile_id.is_none() {
            let chosen = profiles.selected(Some(&id))?;
            let mut selected = network.clone();
            selected.profile_id = Some(chosen.id.clone());
            selected.profile = Some(chosen.profile.name.clone());
            save_network(&worker.root, &selected)?;
            *network = selected;
        }
        Ok(())
    })
    .await
    .unwrap_or_else(|_| Err("설정 파일을 가져오지 못했습니다.".into()));
    state.busy.store(false, Ordering::Release);
    state.emit(&app);
    result
}
fn exit_location(port: u16) -> Result<(String, String), String> {
    let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))
        .map_err(|_| "연결 확인을 준비하지 못했습니다.")?;
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .proxy(proxy)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "연결 확인을 준비하지 못했습니다.")?;
    let response = client
        .get("https://www.cloudflare.com/cdn-cgi/trace")
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|_| {
            "VPN 연결을 확인하지 못했습니다. 설정과 인터넷 연결을 확인하고 다시 연결해 주세요."
        })?;
    let mut text = String::new();
    response
        .take(8192)
        .read_to_string(&mut text)
        .map_err(|_| "VPN 응답을 읽지 못했습니다.")?;
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .map(str::to_owned)
    };
    let ip = field("ip=")
        .filter(|s| s.parse::<std::net::IpAddr>().is_ok())
        .ok_or("VPN IP를 확인하지 못했습니다.")?;
    let country = field("loc=")
        .filter(|s| s.len() == 2 && s.chars().all(|c| c.is_ascii_uppercase()))
        .ok_or("접속 국가를 확인하지 못했습니다.")?;
    Ok((ip, country))
}
#[tauri::command]
async fn browser_network(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: tauri::State<'_, Arc<Browser>>,
    enabled: bool,
    fragment: bool,
    profile_id: Option<String>,
) -> Result<(), String> {
    trusted(&webview)?;
    let state = state.inner().clone();
    if state.busy.swap(true, Ordering::AcqRel) {
        return Err("연결 설정을 변경하는 중입니다.".into());
    }
    if !state.downloads.lock().unwrap().is_empty() {
        state.busy.store(false, Ordering::Release);
        return Err("브라우저 다운로드가 끝난 뒤 연결 방식을 변경해 주세요.".into());
    }
    {
        let mut network = state.network.lock().unwrap().clone();
        let selection_only = profile_id.is_some()
            && !network.enabled
            && !enabled
            && network.fragment == fragment
            && network.phase == "direct";
        if let Some(id) = profile_id {
            let summaries = state.profiles.lock().unwrap();
            let Some(profile) = summaries.iter().find(|p| p.id == id) else {
                state.busy.store(false, Ordering::Release);
                return Err("선택한 국가 설정을 찾지 못했습니다.".into());
            };
            network.profile_id = Some(profile.id.clone());
            network.profile = Some(profile.name.clone());
        }
        if selection_only {
            let result = save_network(&state.root, &network);
            if result.is_ok() {
                *state.network.lock().unwrap() = network;
            }
            state.busy.store(false, Ordering::Release);
            state.emit(&app);
            return result;
        }
        network.enabled = enabled;
        network.fragment = fragment;
        network.phase = if enabled { "connecting" } else { "direct" }.into();
        network.ip = None;
        network.country = None;
        network.error = None;
        if let Err(error) = save_network(&state.root, &network) {
            state.busy.store(false, Ordering::Release);
            return Err(error);
        }
        state.gateway.change(proxy::Route::Blocked, fragment);
        *state.network.lock().unwrap() = network;
    }
    state.emit(&app);
    let recovery = state.clone();
    let recovery_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let result: Result<(), String> = (|| {
            state.engine.lock().unwrap().take();
            if !enabled {
                state.gateway.change(proxy::Route::Direct, fragment);
                return Ok(());
            }
            let profiles = vpn::load_profiles(&state.root)?;
            let selected = state.network.lock().unwrap().profile_id.clone();
            let profile = &profiles.selected(selected.as_deref())?.profile;
            let exe = if cfg!(debug_assertions) {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../.runtime/browser-tools/wireproxy.exe")
            } else {
                app.path()
                    .resource_dir()
                    .map_err(|_| "VPN 도구 경로를 찾지 못했습니다.")?
                    .join("media-tools/browser/wireproxy.exe")
            };
            let engine = vpn::Engine::start(&exe, &profile.config)?;
            let port = engine.port;
            *state.engine.lock().unwrap() = Some(engine);
            state.gateway.change(proxy::Route::Vpn(port), fragment);
            let (ip, country) = exit_location(state.gateway.port)?;
            let mut network = state.network.lock().unwrap();
            network.phase = "connected".into();
            network.ip = Some(ip);
            network.country = Some(country);
            Ok(())
        })();
        if let Err(error) = &result {
            state.gateway.change(proxy::Route::Blocked, fragment);
            state.engine.lock().unwrap().take();
            let mut network = state.network.lock().unwrap();
            network.phase = "error".into();
            network.error = Some(error.clone());
        }
        state.busy.store(false, Ordering::Release);
        state.emit(&app);
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            let state = app2.state::<Arc<Browser>>();
            let ids: Vec<_> = state
                .tabs
                .lock()
                .unwrap()
                .iter()
                .map(|t| t.id.clone())
                .collect();
            for id in ids {
                if let Some(view) = app2.get_webview(&id) {
                    let _ = view.reload();
                }
            }
        });
        result
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => {
            let error = "네트워크 설정을 변경하지 못했습니다.".to_string();
            recovery.gateway.change(proxy::Route::Blocked, fragment);
            {
                let mut network = recovery.network.lock().unwrap();
                network.phase = "error".into();
                network.error = Some(error.clone());
            }
            recovery.busy.store(false, Ordering::Release);
            recovery.emit(&recovery_app);
            Err(error)
        }
    }
}
fn save_network(root: &std::path::Path, network: &Network) -> Result<(), String> {
    let bytes = serde_json::to_vec(network).map_err(|_| "연결 설정을 저장하지 못했습니다.")?;
    let temporary = root.join("network.pending");
    std::fs::write(&temporary, bytes)
        .map_err(|_| "연결 설정을 저장하지 못했습니다. 저장 공간을 확인해 주세요.")?;
    vpn::replace(&temporary, &root.join("network.json"))
}
fn restore_network(root: &std::path::Path) -> Network {
    match std::fs::read(root.join("network.json")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Network::default(),
        Ok(bytes) if bytes.len() < 4096 => {
            serde_json::from_slice(&bytes).unwrap_or_else(|_| Network {
                enabled: true,
                ..Network::default()
            })
        }
        _ => Network {
            enabled: true,
            ..Network::default()
        },
    }
}
pub fn run() -> tauri::Result<()> {
    #[cfg(debug_assertions)]
    fixture::configure().map_err(std::io::Error::other)?;
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = "local.atsumi.browser".into();
    #[cfg(debug_assertions)]
    if fixture::current().is_some() {
        context.config_mut().identifier = "local.atsumi.browser.qa".into();
    }
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(w) = app.get_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            #[cfg(debug_assertions)]
            let root = fixture::current().map(|qa| qa.root.clone()).unwrap_or(root);
            std::fs::create_dir_all(&root)?;
            let preferences = library::Preferences::load(&root);
            let saved_session = restore_session(&root);
            let library = library::Store::open(&root);
            let library_notice = library.as_ref().err().cloned();
            let profiles = vpn::load_profiles(&root);
            let mut network = restore_network(&root);
            network.ip = None;
            network.country = None;
            network.phase = if network.enabled { "error" } else { "direct" }.into();
            network.error = if network.enabled {
                Some("VPN을 다시 연결해 주세요. 그동안 브라우저 연결은 차단됩니다.".into())
            } else {
                None
            };
            let summaries = match profiles {
                Ok(profiles) => {
                    if !profiles.profiles.is_empty() {
                        match profiles.selected(network.profile_id.as_deref()) {
                            Ok(p) => {
                                network.profile = Some(p.profile.name.clone());
                                network.profile_id = Some(p.id.clone());
                            }
                            Err(e) => {
                                network.error = Some(e);
                                network.profile = None;
                            }
                        }
                    } else {
                        network.profile = None;
                        network.profile_id = None;
                    }
                    profiles.summaries()
                }
                Err(e) => {
                    network.error = Some(e);
                    network.profile = None;
                    Vec::new()
                }
            };
            let state = Arc::new(Browser {
                gateway: proxy::Gateway::start()?,
                root: root.clone(),
                library: library.ok(),
                preferences: Mutex::new(preferences.clone()),
                password_saving_available: AtomicBool::new(false),
                maximized: AtomicBool::new(false),
                closed: Mutex::new(
                    saved_session
                        .closed
                        .into_iter()
                        .rev()
                        .take(20)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect(),
                ),
                engine: Mutex::new(None),
                network: Mutex::new(network),
                profiles: Mutex::new(summaries),
                tabs: Mutex::default(),
                active: Mutex::default(),
                busy: AtomicBool::new(false),
                downloads: Mutex::default(),
                notice: Mutex::new(library_notice),
                bounds: Mutex::new((0.0, 0.0, 0.0, 0.0)),
            });
            {
                let n = state.network.lock().unwrap().clone();
                state.gateway.change(
                    if n.enabled {
                        proxy::Route::Blocked
                    } else {
                        proxy::Route::Direct
                    },
                    n.fragment,
                );
            }
            app.manage(state.clone());
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("browser.html".into()))
                .title("Atsumi Browser")
                .decorations(false)
                .shadow(true)
                .inner_size(1200.0, 850.0)
                .min_inner_size(720.0, 500.0)
                .data_directory(root.join("chrome-profile"))
                .build()?;
            if let Some(chrome) = app.get_webview("main") {
                install_shortcuts(&chrome);
            }
            // Keep profile cookies and local storage; restoring tabs never clones another browser profile.
            let mut restored_active = None;
            if preferences.restore_tabs {
                for (index, tab) in saved_session.tabs.into_iter().take(30).enumerate() {
                    if address(&tab.url).is_ok() {
                        if let Ok(id) = new_tab(app.handle(), &tab.url, true) {
                            if saved_session.active == Some(index) {
                                restored_active = Some(id);
                            }
                        }
                    }
                }
            }
            if let Some(id) = restored_active {
                *state.active.lock().unwrap() = Some(id);
                state.layout(app.handle());
                state.save_tabs();
            }
            if state.tabs.lock().unwrap().is_empty() {
                new_tab(app.handle(), "about:blank", true).map_err(std::io::Error::other)?;
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut health = std::time::Instant::now();
                loop {
                    std::thread::sleep(Duration::from_secs(3));
                    if state.gateway_stopped() {
                        break;
                    }
                    if state.busy.load(Ordering::Acquire) {
                        continue;
                    }
                    let died = state
                        .engine
                        .lock()
                        .unwrap()
                        .as_mut()
                        .is_some_and(|e| !e.alive());
                    if died {
                        let fragment = state.network.lock().unwrap().fragment;
                        state.gateway.change(proxy::Route::Blocked, fragment);
                        let mut n = state.network.lock().unwrap();
                        n.phase = "error".into();
                        n.error =
                            Some("VPN 연결 엔진이 종료되어 브라우저 연결을 차단했습니다.".into());
                        drop(n);
                        state.engine.lock().unwrap().take();
                        state.emit(&handle);
                    }
                    if health.elapsed() > Duration::from_secs(30) {
                        health = std::time::Instant::now();
                        let connected = state.network.lock().unwrap().phase == "connected";
                        if connected && !state.busy.swap(true, Ordering::AcqRel) {
                            let result = exit_location(state.gateway.port);
                            let mut n = state.network.lock().unwrap();
                            match result {
                                Ok((ip, country)) => {
                                    n.ip = Some(ip);
                                    n.country = Some(country);
                                }
                                Err(error) => {
                                    state.gateway.change(proxy::Route::Blocked, n.fragment);
                                    n.phase = "error".into();
                                    n.error = Some(error);
                                }
                            }
                            drop(n);
                            state.busy.store(false, Ordering::Release);
                            state.emit(&handle);
                        }
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let Some(state) = window.app_handle().try_state::<Arc<Browser>>() {
                match event {
                    tauri::WindowEvent::Resized(_) => {
                        state
                            .maximized
                            .store(window.is_maximized().unwrap_or(false), Ordering::Release);
                        state.emit(window.app_handle());
                    }
                    tauri::WindowEvent::CloseRequested { api, .. }
                        if !state.downloads.lock().unwrap().is_empty() =>
                    {
                        api.prevent_close();
                        *state.notice.lock().unwrap() =
                            Some("다운로드가 끝나면 브라우저를 닫을 수 있습니다.".into());
                        state.emit(window.app_handle());
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            browser_snapshot,
            browser_tab,
            browser_bounds,
            browser_import,
            browser_network,
            browser_window,
            browser_library_query,
            browser_library_update,
            browser_preferences
        ])
        .build(context)?;
    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            if let Some(s) = handle.try_state::<Arc<Browser>>() {
                s.save_tabs();
                if let Some(store) = &s.library {
                    store.flush();
                }
                s.gateway.stop();
                s.engine.lock().unwrap().take();
            }
        }
    });
    Ok(())
}
impl Browser {
    fn gateway_stopped(&self) -> bool {
        self.gateway.is_stopped()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn damaged_preferences_never_restore_direct_connections() {
        let root =
            std::env::temp_dir().join(format!("atsumi-browser-pref-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        assert!(!restore_network(&root).enabled);
        std::fs::write(root.join("network.json"), b"broken").unwrap();
        assert!(restore_network(&root).enabled);
        let enabled = Network {
            enabled: true,
            fragment: true,
            ..Network::default()
        };
        save_network(&root, &enabled).unwrap();
        assert!(restore_network(&root).enabled);
        assert!(restore_network(&root).fragment);
        save_network(&root, &Network::default()).unwrap();
        assert!(!restore_network(&root).enabled);
        std::fs::remove_file(root.join("network.json")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn old_tabs_migrate_and_selected_tab_and_closed_tabs_roundtrip() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("tabs.json"),
            br#"[{"id":"a","url":"https://example.com/","title":"A","loading":false}]"#,
        )
        .unwrap();
        let mut session = restore_session(root.path());
        assert_eq!(session.active, Some(0));
        assert_eq!(session.tabs.len(), 1);
        assert!(!session.tabs[0].can_go_back);
        session.closed = session.tabs.clone();
        std::fs::write(
            root.path().join("tabs.json"),
            serde_json::to_vec(&session).unwrap(),
        )
        .unwrap();
        let reopened = restore_session(root.path());
        assert_eq!(reopened.closed.len(), 1);
        assert_eq!(reopened.active, Some(0));
        std::fs::write(root.path().join("tabs.json"), b"bad").unwrap();
        assert!(restore_session(root.path()).tabs.is_empty());
    }
    #[test]
    fn browser_navigation_rejects_privileged_and_external_protocols() {
        for input in [
            "file:///C:/secret",
            "javascript://alert/1",
            "http://127.0.0.1:14200/browser.html",
            "https://tauri.localhost/",
            "http://[::1]/",
        ] {
            assert!(address(input).is_err(), "{input}");
        }
        assert_eq!(
            address("example.com").unwrap().as_str(),
            "https://example.com/"
        );
        assert_eq!(
            address("hello world").unwrap().host_str(),
            Some("www.google.com")
        );
    }
}

#[cfg(windows)]
fn install_shortcuts(view: &tauri::Webview) {
    use webview2_com::{
        AcceleratorKeyPressedEventHandler,
        Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
        },
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
    let app = view.app_handle().clone();
    let _ = view.with_webview(move |platform| unsafe {
        let controller = platform.controller();
        let mut token = 0;
        let _ = controller.add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
                args.KeyEventKind(&mut kind)?;
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                    && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
                {
                    return Ok(());
                }
                let mut key = 0;
                args.VirtualKey(&mut key)?;
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                let alt = GetKeyState(VK_MENU.0 as i32) < 0;
                let action = if ctrl && !alt {
                    match key {
                        76 => Some("address"),
                        84 => Some(if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                            "restore"
                        } else {
                            "new"
                        }),
                        72 => Some("history"),
                        68 => Some("bookmark"),
                        87 => Some("close"),
                        9 => Some(if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                            "previous"
                        } else {
                            "next"
                        }),
                        _ => None,
                    }
                } else if key == 116 {
                    Some("reload")
                } else if alt {
                    match key {
                        37 => Some("back"),
                        39 => Some("forward"),
                        _ => None,
                    }
                } else {
                    None
                };
                if let Some(action) = action {
                    args.SetHandled(true)?;
                    let app2 = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        if let Some(chrome) = app2.get_webview("main") {
                            let _ = chrome.set_focus();
                        }
                        let _ = app2.emit_to("main", "browser:shortcut", action);
                    });
                }
                Ok(())
            })),
            &mut token,
        );
    });
}
#[cfg(not(windows))]
fn install_shortcuts(_: &tauri::Webview) {}

#[tauri::command]
fn browser_window(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    action: String,
) -> Result<(), String> {
    trusted(&webview)?;
    let window = app
        .get_window("main")
        .ok_or("브라우저 창을 찾지 못했습니다.")?;
    let result = match action.as_str() {
        "minimize" => window.minimize(),
        "maximize" => {
            if window.is_maximized().unwrap_or(false) {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }
        "drag" => window.start_dragging(),
        "close" => window.close(),
        _ => return Err("지원하지 않는 창 동작입니다.".into()),
    };
    result.map_err(|_| "창 상태를 변경하지 못했습니다.".into())
}
#[tauri::command]
async fn browser_library_query(
    webview: tauri::Webview,
    state: tauri::State<'_, Arc<Browser>>,
    kind: String,
    query: String,
    offset: usize,
) -> Result<library::Page, String> {
    trusted(&webview)?;
    let store = state
        .library
        .clone()
        .ok_or("방문 기록을 사용할 수 없습니다.")?;
    store.search(kind, query, offset).await
}
#[tauri::command]
async fn browser_library_update(
    webview: tauri::Webview,
    state: tauri::State<'_, Arc<Browser>>,
    kind: String,
    action: String,
    id: Option<String>,
    url: Option<String>,
    title: Option<String>,
) -> Result<bool, String> {
    trusted(&webview)?;
    let store = state
        .library
        .clone()
        .ok_or("방문 기록을 사용할 수 없습니다.")?;
    store
        .change(
            kind,
            action,
            id.unwrap_or_default(),
            url.unwrap_or_default(),
            title.unwrap_or_default(),
        )
        .await
}
#[tauri::command]
async fn browser_preferences(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: tauri::State<'_, Arc<Browser>>,
    preferences: library::Preferences,
) -> Result<(), String> {
    trusted(&webview)?;
    let worker = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        preferences.save(&worker.root)?;
        *worker.preferences.lock().unwrap() = preferences;
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "설정을 저장하지 못했습니다.")??;
    let ids: Vec<_> = state
        .tabs
        .lock()
        .unwrap()
        .iter()
        .map(|t| t.id.clone())
        .collect();
    for id in ids {
        if let Some(view) = app.get_webview(&id) {
            apply_browser_preferences(&view);
        }
    }
    state.emit(&app);
    Ok(())
}
#[cfg(windows)]
fn apply_browser_preferences(view: &tauri::Webview) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings4;
    use windows::core::Interface;
    let app = view.app_handle().clone();
    let _ = view.with_webview(move |platform| unsafe {
        let state = app.state::<Arc<Browser>>();
        let p = state.preferences.lock().unwrap().clone();
        let result = (|| -> windows::core::Result<()> {
            let settings = platform
                .controller()
                .CoreWebView2()?
                .Settings()?
                .cast::<ICoreWebView2Settings4>()?;
            settings.SetIsPasswordAutosaveEnabled(p.save_passwords)?;
            settings.SetIsGeneralAutofillEnabled(p.autofill)?;
            let mut passwords = windows::core::BOOL::default();
            let mut autofill = windows::core::BOOL::default();
            settings.IsPasswordAutosaveEnabled(&mut passwords)?;
            settings.IsGeneralAutofillEnabled(&mut autofill)?;
            if passwords.as_bool() != p.save_passwords || autofill.as_bool() != p.autofill {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80004005u32 as i32,
                )));
            }
            Ok(())
        })();
        state
            .password_saving_available
            .store(result.is_ok(), Ordering::Release);
        if result.is_err() {
            *state.notice.lock().unwrap() = Some(
                "비밀번호 저장 기능을 사용할 수 없습니다. WebView2 업데이트를 확인해 주세요."
                    .into(),
            );
        }
        state.emit(&app);
    });
}
#[cfg(not(windows))]
fn apply_browser_preferences(_: &tauri::Webview) {}
#[cfg(windows)]
fn install_browser_features(view: &tauri::Webview) {
    use webview2_com::{HistoryChangedEventHandler, NavigationCompletedEventHandler};
    let app = view.app_handle().clone();
    let id = view.label().to_owned();
    apply_browser_preferences(view);
    let _ = view.with_webview(move |platform| unsafe {
        let Ok(core) = platform.controller().CoreWebView2() else {
            return;
        };
        let complete_app = app.clone();
        let complete_id = id.clone();
        let mut complete_token = 0;
        let _ = core.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut success = windows::core::BOOL::default();
                args.IsSuccess(&mut success)?;
                if success.as_bool() {
                    let state = complete_app.state::<Arc<Browser>>();
                    let tab = state
                        .tabs
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|t| t.id == complete_id)
                        .cloned();
                    if let (Some(store), Some(tab)) = (&state.library, tab) {
                        store.visit(&tab.url, &tab.title);
                    }
                }
                Ok(())
            })),
            &mut complete_token,
        );
        let mut token = 0;
        let _ = core.add_HistoryChanged(
            &HistoryChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(core) = sender else { return Ok(()) };
                let mut back = windows::core::BOOL::default();
                let mut forward = windows::core::BOOL::default();
                core.CanGoBack(&mut back)?;
                core.CanGoForward(&mut forward)?;
                let state = app.state::<Arc<Browser>>();
                if let Some(tab) = state.tabs.lock().unwrap().iter_mut().find(|t| t.id == id) {
                    tab.can_go_back = back.as_bool();
                    tab.can_go_forward = forward.as_bool();
                }
                state.emit(&app);
                Ok(())
            })),
            &mut token,
        );
    });
}
#[cfg(not(windows))]
fn install_browser_features(_: &tauri::Webview) {}
