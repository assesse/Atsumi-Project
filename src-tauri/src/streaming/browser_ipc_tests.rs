//! Exercise the vendored IPC policy with Tauri's in-memory public mock API.
//! No application setup, database, browser, or upstream fixture files are used.

use serde_json::{json, Value};
use tauri::{
    ipc::{CallbackFn, InvokeBody},
    test::{get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime},
    webview::InvokeRequest,
    App, WebviewWindow, WebviewWindowBuilder,
};

const FETCH_CHANNEL: &str = "plugin:__TAURI_CHANNEL__|fetch";
const REMOTE_DENIED: &str = "Tauri IPC is disabled for remote content";
const PING: &str = "browser_ipc_test_ping";
const CONFIRM_CONTROL: &str = "chzzk_browser_confirm_control";
const MULTIVIEW_COMMANDS: [&str; 19] = [
    "replay_open",
    "replay_close",
    "replay_chat_at",
    "replay_chat_page",
    "replay_timeline",
    "replay_set_offset",
    "chzzk_browser_open_merged",
    "chzzk_browser_retry_merge",
    "chzzk_multiview_configure",
    "chzzk_multiview_snapshot",
    "chzzk_multiview_close",
    "chzzk_multiview_set_audio",
    "chzzk_multiview_set_viewport",
    "chzzk_multiview_request_control",
    "chzzk_multiview_confirm_control",
    "chzzk_multiview_ack_ui_action",
    "chzzk_multiview_set_pane_audio",
    "chzzk_browser_request_control",
    "chzzk_browser_ack_ui_action",
];

fn request(app: &App<MockRuntime>, command: &str, origin: &str) -> InvokeRequest {
    InvokeRequest {
        cmd: command.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: origin.parse().unwrap(),
        body: InvokeBody::default(),
        headers: Default::default(),
        invoke_key: app.handle().invoke_key().to_string(),
    }
}

fn assert_remote_denied(
    app: &App<MockRuntime>,
    window: &WebviewWindow<MockRuntime>,
    command: &str,
    origin: &str,
) {
    assert_eq!(
        get_ipc_response(window, request(app, command, origin)).err(),
        Some(json!(REMOTE_DENIED)),
        "remote IPC must be denied: {}, {command}, {origin}",
        window.label()
    );
}

#[test]
fn remote_custom_commands_and_internal_channel_are_denied_for_every_label() {
    let app = mock_builder()
        .invoke_handler(|_| panic!("remote request reached the application dispatcher"))
        .build(mock_context(noop_assets()))
        .unwrap();
    for label in [
        "main",
        "chzzk-official-browser",
        "chzzk-login-test",
        "chzzk-mado-7-video",
        "chzzk-mado-7-chat",
    ] {
        let window = WebviewWindowBuilder::new(&app, label, Default::default())
            .build()
            .unwrap();
        for origin in [
            "https://chzzk.naver.com/live/test",
            "https://nid.naver.com/",
            "https://tauri.localhost.evil.example/",
            "http://127.0.0.1:1420/",
            "data:text/html,test",
        ] {
            for command in [PING, FETCH_CHANNEL, CONFIRM_CONTROL]
                .into_iter()
                .chain(MULTIVIEW_COMMANDS)
            {
                assert_remote_denied(&app, &window, command, origin);
            }
        }
    }
}

#[test]
fn explicit_remote_permissions_cannot_override_native_ipc_denial() {
    let mut context = mock_context(noop_assets());
    for command in [PING, FETCH_CHANNEL, CONFIRM_CONTROL]
        .into_iter()
        .chain(MULTIVIEW_COMMANDS)
    {
        context.runtime_authority_mut().__allow_command(
            command.into(),
            tauri::utils::acl::ExecutionContext::Remote {
                url: "https://chzzk.naver.com/*".parse().unwrap(),
            },
        );
    }
    let app = mock_builder()
        .invoke_handler(|_| panic!("remote capability bypassed native IPC policy"))
        .build(context)
        .unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    for command in [PING, FETCH_CHANNEL, CONFIRM_CONTROL]
        .into_iter()
        .chain(MULTIVIEW_COMMANDS)
    {
        assert_remote_denied(&app, &window, command, "https://chzzk.naver.com/live/test");
    }
}

#[test]
fn local_custom_large_json_and_internal_channel_dispatch_remain_available() {
    let expected = json!({"payload": "x".repeat(16 * 1024)});
    let reply = expected.clone();
    let app = mock_builder()
        .invoke_handler(move |invoke| {
            assert_eq!(invoke.message.command(), PING);
            invoke.resolver.resolve(reply.clone());
            true
        })
        .build(mock_context(noop_assets()))
        .unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let origin = if cfg!(any(windows, target_os = "android")) {
        "http://tauri.localhost/"
    } else {
        "tauri://localhost/"
    };
    let reply = get_ipc_response(&window, request(&app, PING, origin))
        .unwrap()
        .deserialize::<Value>()
        .unwrap();
    assert_eq!(reply, expected);

    // Verify actual built-in fetch routing without touching any queued data ID.
    // This is dispatcher coverage, not a real WebView transport/timing test.
    assert_eq!(
        get_ipc_response(&window, request(&app, FETCH_CHANNEL, origin)).err(),
        Some(json!("missing channel id header"))
    );
}

#[test]
fn configured_dev_origin_is_distinct_from_remote_and_wrong_port_origins() {
    let mut context = mock_context(noop_assets());
    context.config_mut().build.dev_url = Some("http://127.0.0.1:1420/".parse().unwrap());
    let app = mock_builder()
        .invoke_handler(|invoke| {
            assert_eq!(invoke.message.command(), PING);
            invoke.resolver.resolve("pong");
            true
        })
        .build(context)
        .unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let dev_origin = "http://127.0.0.1:1420/nested/path";
    if tauri::is_dev() {
        assert_eq!(
            get_ipc_response(&window, request(&app, PING, dev_origin))
                .unwrap()
                .deserialize::<String>()
                .unwrap(),
            "pong"
        );
    } else {
        // Tauri ignores devUrl in production; it must not become a trusted URL.
        assert_remote_denied(&app, &window, PING, dev_origin);
    }
    for origin in [
        "https://chzzk.naver.com/",
        "http://127.0.0.1:1421/",
        "https://127.0.0.1:1420/",
        "http://127.0.0.1.evil.example:1420/",
    ] {
        for command in [PING, FETCH_CHANNEL] {
            assert_remote_denied(&app, &window, command, origin);
        }
    }
}

#[test]
fn remote_child_cannot_inherit_trusted_parent_window_ipc() {
    use tauri::{LogicalPosition, LogicalSize, Manager, WebviewBuilder, WebviewUrl};
    let app = mock_builder()
        .invoke_handler(|_| panic!("remote child reached native dispatcher"))
        .build(mock_context(noop_assets()))
        .unwrap();
    let main = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let parent = app.get_window("main").unwrap();
    let child = parent
        .add_child(
            WebviewBuilder::new(
                "chzzk-official",
                WebviewUrl::External("https://chzzk.naver.com/live/test".parse().unwrap()),
            ),
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(800.0, 500.0),
        )
        .unwrap();
    struct Child(tauri::Webview<MockRuntime>);
    impl AsRef<tauri::Webview<MockRuntime>> for Child {
        fn as_ref(&self) -> &tauri::Webview<MockRuntime> {
            &self.0
        }
    }
    assert_eq!(child.window().label(), "main");
    assert!(app.get_window("main").is_some());
    assert_eq!(main.label(), "main");
    let child = Child(child);
    for command in [PING, FETCH_CHANNEL, CONFIRM_CONTROL] {
        assert_eq!(
            get_ipc_response(
                &child,
                request(&app, command, "https://chzzk.naver.com/live/test")
            )
            .err(),
            Some(json!(REMOTE_DENIED))
        );
    }
}
