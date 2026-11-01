//! Explicit development acceptance aid. All destinations and actions are fixed;
//! no caller-provided URL, session export, upload, save or recording command.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

const OPT_IN: &str = "20261025-supported-link-v1";
const CONTROL: &str = "supported-link-verification";
const CHILD: &str = "supported-link-verification-child";
const LINKS: &[(&str, &str)] = &[
    (
        "Danbooru · 구주소 · #12439739",
        "https://danbooru.donmai.us/post/show/12439739",
    ),
    (
        "Danbooru · 현재 주소 · #12439739",
        "https://danbooru.donmai.us/posts/12439739",
    ),
    (
        "Pixiv · 프로필 · #32715112",
        "https://www.pixiv.net/users/32715112",
    ),
    ("Pixiv · 프로필 · #4912", "https://www.pixiv.net/users/4912"),
    (
        "Pinterest · 핀",
        "https://www.pinterest.com/pin/621004236103700020/",
    ),
    (
        "YouTube · Big Buck Bunny",
        "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
    ),
    ("YouTube · NASA 채널", "https://www.youtube.com/@NASA"),
    ("SOOP · LCK_KR 채널", "https://play.sooplive.com/aflol"),
    (
        "CHZZK · 홀랜 채널",
        "https://chzzk.naver.com/live/bfe2a317d2b5ffb300b5af556a727afa",
    ),
    ("X · NASA 프로필", "https://x.com/NASA"),
    (
        "X · 일반 영상 4개 게시물",
        "https://x.com/notjignes/status/2113958988949942538",
    ),
    (
        "Hitomi · gallery ID",
        "https://hitomi.la/galleries/4234573.html",
    ),
    (
        "Pixiv · 작품 · #144440262",
        "https://www.pixiv.net/artworks/144440262",
    ),
    ("X · 일반 그래픽 게시자", "https://x.com/vib3coded"),
    ("X · No Starch Press", "https://x.com/nostarch"),
    ("X · Pixiv 공식 프로필", "https://x.com/pixiv"),
];

static INSTALLED: AtomicBool = AtomicBool::new(false);

pub(crate) fn install(view: &Webview) {
    if std::env::var("ATSUMI_SUPPORTED_LINK_VERIFICATION").as_deref() != Ok(OPT_IN)
        || INSTALLED.swap(true, Ordering::AcqRel)
    {
        return;
    }
    // Windows WebView creation and even querying its URL can synchronously wait
    // for the native UI loop. Never perform either inside a page-load callback.
    let view = view.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("atsumi-link-verification".into())
        .spawn(move || {
            if trusted(&view).is_err() {
                return;
            }
            tracing::info!("supported-link verification control creating");
            if let Err(error) = create(view.app_handle()) {
                tracing::warn!(%error, "supported-link verification control unavailable");
            } else {
                tracing::info!("supported-link verification control ready");
            }
        })
    {
        INSTALLED.store(false, Ordering::Release);
        tracing::warn!(%error, "supported-link verification worker unavailable");
    }
}

fn create(app: &AppHandle) -> Result<(), String> {
    let main = app.get_window("main").ok_or(ERROR)?;
    let child = main
        .add_child(
            tauri::WebviewBuilder::new(
                CHILD,
                tauri::WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .focused(false)
            .incognito(true)
            .on_navigation(|url| url.as_str() == "about:blank")
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .on_download(|_, _| false),
            tauri::LogicalPosition::new(0.0, 0.0),
            tauri::LogicalSize::new(2.0, 2.0),
        )
        .map_err(|_| "verification child creation failed")?;
    child
        .hide()
        .map_err(|_| "verification child concealment failed")?;
    let nonce = uuid::Uuid::new_v4().to_string();
    let labels = LINKS.iter().map(|(label, _)| *label).collect::<Vec<_>>();
    let config = serde_json::json!({"labels":labels,"nonce":nonce});
    let script = r#"(() => {
      if (location.href !== 'about:blank') return;
      const config = __CONFIG__;
      const render = () => {
        if (document.getElementById('link-verification')) return;
        document.title = 'Atsumi · 링크 연결 검증';
        const style = document.createElement('style');
        style.textContent = 'body{margin:0;padding:18px;background:#18171c;color:#eee;font:13px system-ui}h2{font-size:17px;margin:0 0 6px}p{color:#b6acba;margin:0 0 14px}button{display:block;width:100%;padding:10px;margin:6px 0;text-align:left;color:#eee;background:#302a35;border:1px solid #5b4b60;border-radius:6px;cursor:pointer}button:hover{border-color:#d79bad;background:#463442}';
        document.head.append(style);
        const root = document.createElement('main'); root.id = 'link-verification';
        const title = document.createElement('h2'); title.textContent = '지원 링크 실제 앱 검증'; root.append(title);
        const note = document.createElement('p'); note.textContent = '고정된 링크 · 실제 플랫폼 응답 · 저장/녹화 없음'; root.append(note);
        config.labels.forEach((label, index) => {
          const button = document.createElement('button'); button.textContent = label;
          button.onclick = event => { if (event.isTrusted) location.href = 'https://atsumi-link-verification.invalid/' + config.nonce + '/' + index; };
          root.append(button);
        });
        document.body.append(root);
      };
      if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', render, {once:true}); else render();
    })();"#.replace("__CONFIG__", &config.to_string());
    let opener = app.clone();
    let expected = nonce.clone();
    let control = tauri::WebviewWindowBuilder::new(
        app,
        CONTROL,
        tauri::WebviewUrl::External("about:blank".parse().unwrap()),
    )
    .title("Atsumi · 링크 연결 검증")
    .inner_size(350.0, 660.0)
    .focused(false)
    .incognito(true)
    .initialization_script(script.clone())
    .on_navigation(move |url| {
        if url.as_str() == "about:blank" {
            return true;
        }
        if url.scheme() != "https" || url.host_str() != Some("atsumi-link-verification.invalid") {
            return false;
        }
        let parts = url
            .path()
            .trim_start_matches('/')
            .split('/')
            .collect::<Vec<_>>();
        if parts.len() == 2
            && parts[0] == expected
            && url.query().is_none()
            && url.fragment().is_none()
        {
            if let Some((_, destination)) = parts[1]
                .parse::<usize>()
                .ok()
                .and_then(|index| LINKS.get(index))
            {
                let result = open_supported_or_external(&opener, destination);
                tracing::info!(
                    probe_index = parts[1],
                    success = result.is_ok(),
                    "supported-link verification dispatch"
                );
            }
        }
        false
    })
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .on_download(|_, _| false)
    .build()
    .map_err(|_| "verification control creation failed")?;
    // WebView2 may retain its initial about:blank without a new navigation;
    // document-created scripts alone do not initialize that first document.
    control
        .eval(&script)
        .map_err(|_| "verification control rendering failed")?;
    let owner = app.clone();
    control.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            if let Some(child) = owner.get_webview(CHILD) {
                let _ = child.close();
            }
        }
    });
    Ok(())
}
