//! Immutable public player assets, hosted outside the privileged app origin.
//!
//! This protocol serves only compiled-in CHZZK player code and our small frame
//! adapter. It never resolves filesystem paths, replay tokens, or user data.
//! The embedding iframe and this response both keep the document sandboxed;
//! Vue's runtime compiler needs unsafe-eval here, not in the main app's CSP.
use tauri::http::{header, Method, Request, Response, StatusCode};

#[path = "accepted_player_assets.rs"]
mod accepted;

const PLAYER_ORIGINS: &str =
    "atsumi-player://localhost http://atsumi-player.localhost https://atsumi-player.localhost";
const MEDIA_ORIGINS: &str =
    "atsumi-replay://localhost http://atsumi-replay.localhost https://atsumi-replay.localhost";
const MAIN_ORIGINS: &str = "tauri://localhost http://tauri.localhost https://tauri.localhost";

fn content_security_policy() -> String {
    let ancestors = if cfg!(debug_assertions) {
        format!("{MAIN_ORIGINS} http://127.0.0.1:1420")
    } else {
        MAIN_ORIGINS.to_owned()
    };
    format!(
        "default-src 'none'; base-uri 'none'; object-src 'none'; \
         frame-src 'none'; frame-ancestors {ancestors}; \
         script-src {PLAYER_ORIGINS} 'unsafe-eval'; \
         style-src {PLAYER_ORIGINS} 'unsafe-inline'; \
         img-src {PLAYER_ORIGINS} data: blob:; font-src {PLAYER_ORIGINS} data:; \
         media-src {MEDIA_ORIGINS} blob:; \
         connect-src 'none'; worker-src 'none'; form-action 'none'; sandbox allow-scripts"
    )
}

fn asset(path: &str) -> Option<(&'static [u8], &'static str)> {
    Some(match path {
        "/frame.html" => (
            include_bytes!("../../../public/original-player/frame.html"),
            "text/html; charset=utf-8",
        ),
        "/runtime.js" => (
            include_bytes!("../../../public/original-player/runtime.js"),
            "text/javascript; charset=utf-8",
        ),
        "/player-vendor-BYg0wCyN.js" => (
            include_bytes!("../../../public/original-player/player-vendor-BYg0wCyN.js"),
            "text/javascript; charset=utf-8",
        ),
        "/common-vendor-pcHQV1G2.js" => (
            include_bytes!("../../../public/original-player/common-vendor-pcHQV1G2.js"),
            "text/javascript; charset=utf-8",
        ),
        "/rolldown-runtime-CJJwijRH.js" => (
            include_bytes!("../../../public/original-player/rolldown-runtime-CJJwijRH.js"),
            "text/javascript; charset=utf-8",
        ),
        "/chzzk-slot-template.js" => (
            include_bytes!("../../../public/original-player/chzzk-slot-template.js"),
            "text/javascript; charset=utf-8",
        ),
        "/player-vendor-Ct4cRQDi.css" => (
            include_bytes!("../../../public/original-player/player-vendor-Ct4cRQDi.css"),
            "text/css; charset=utf-8",
        ),
        "/chzzk-theme.css" => (
            include_bytes!("../../../public/original-player/chzzk-theme.css"),
            "text/css; charset=utf-8",
        ),
        "/frame.css" => (
            include_bytes!("../../../public/original-player/frame.css"),
            "text/css; charset=utf-8",
        ),
        _ => return accepted::asset(path),
    })
}

fn trusted_uri(uri: &tauri::http::Uri) -> bool {
    matches!(
        (
            uri.scheme_str(),
            uri.authority().map(|value| value.as_str())
        ),
        (Some("atsumi-player"), Some("localhost"))
            | (Some("http" | "https"), Some("atsumi-player.localhost"))
    ) && uri.query().is_none()
}

/// `webview_label` must come from UriSchemeContext, never a request header.
pub fn response(request: &Request<Vec<u8>>, webview_label: &str) -> Response<Vec<u8>> {
    if webview_label != "main" {
        return build_response(StatusCode::FORBIDDEN, &[], None, false);
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return build_response(StatusCode::METHOD_NOT_ALLOWED, &[], None, false);
    }
    if !trusted_uri(request.uri()) || !request.body().is_empty() {
        return build_response(StatusCode::BAD_REQUEST, &[], None, false);
    }
    match asset(request.uri().path()) {
        Some((bytes, mime)) => build_response(
            StatusCode::OK,
            bytes,
            Some(mime),
            request.method() == Method::HEAD,
        ),
        None => build_response(StatusCode::NOT_FOUND, &[], None, false),
    }
}

fn build_response(
    status: StatusCode,
    bytes: &[u8],
    mime: Option<&str>,
    head: bool,
) -> Response<Vec<u8>> {
    let mut builder = Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .header("Content-Security-Policy", content_security_policy())
        .header(
            "Permissions-Policy",
            "camera=(), microphone=(), geolocation=(), display-capture=(), \
             clipboard-read=(), clipboard-write=(), payment=(), usb=(), serial=(), hid=()",
        );
    if let Some(mime) = mime {
        // Only these immutable public assets are CORS-readable. The opaque
        // iframe's ESM imports send Origin:null. No credentials are accepted,
        // and this permission never applies to the replay media protocol.
        builder = builder
            .header(header::CONTENT_TYPE, mime)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header("Cross-Origin-Resource-Policy", "cross-origin");
    }
    if status == StatusCode::METHOD_NOT_ALLOWED {
        builder = builder.header(header::ALLOW, "GET, HEAD");
    }
    builder
        .body(if head { Vec::new() } else { bytes.to_vec() })
        .expect("constant original-player response headers")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(path: &str) -> Request<Vec<u8>> {
        Request::builder()
            .uri(format!("http://atsumi-player.localhost{path}"))
            .body(Vec::new())
            .unwrap()
    }

    #[test]
    fn serves_only_compiled_public_assets_with_correct_types() {
        for (path, expected_type) in [
            ("/frame.html", "text/html"),
            ("/runtime.js", "text/javascript"),
            ("/player-vendor-BYg0wCyN.js", "text/javascript"),
            ("/common-vendor-pcHQV1G2.js", "text/javascript"),
            ("/rolldown-runtime-CJJwijRH.js", "text/javascript"),
            ("/chzzk-slot-template.js", "text/javascript"),
            ("/player-vendor-Ct4cRQDi.css", "text/css"),
            ("/chzzk-theme.css", "text/css"),
            ("/frame.css", "text/css"),
            ("/accepted-vod-template.js", "text/javascript"),
            ("/recording-metadata.js", "text/javascript"),
            ("/original-surfaces.js", "text/javascript"),
            ("/replay-search.js", "text/javascript"),
            ("/assets/default_profile_dark.png", "image/png"),
            ("/assets/index-2Gtwn4qD.css", "text/css"),
        ] {
            let result = response(&request(path), "main");
            assert_eq!(result.status(), StatusCode::OK, "{path}");
            assert!(!result.body().is_empty(), "{path}");
            assert!(result.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with(expected_type));
            assert_eq!(result.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
            assert!(!result
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS));
        }
    }

    #[test]
    fn sandbox_and_runtime_eval_are_confined_to_original_player_response() {
        let result = response(&request("/frame.html"), "main");
        let policy = result.headers()["Content-Security-Policy"]
            .to_str()
            .unwrap();
        for required in [
            "default-src 'none'",
            "'unsafe-eval'",
            "connect-src 'none'",
            "frame-src 'none'",
            "sandbox allow-scripts",
            "form-action 'none'",
            "frame-ancestors tauri://localhost http://tauri.localhost https://tauri.localhost",
        ] {
            assert!(policy.contains(required), "{required}");
        }
        for forbidden in ["allow-same-origin", "allow-popups", "ipc:", "pstatic.net"] {
            assert!(!policy.contains(forbidden), "{forbidden}");
        }
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        let main_script = config["app"]["security"]["csp"]["script-src"]
            .as_str()
            .unwrap();
        assert_eq!(main_script, "'self'");
    }

    #[test]
    fn rejects_remote_webviews_and_non_exact_routes() {
        for label in ["chzzk-official", "chzzk-login", "replay", ""] {
            assert_eq!(
                response(&request("/frame.html"), label).status(),
                StatusCode::FORBIDDEN
            );
        }
        for path in [
            "/",
            "/../frame.html",
            "/%2e%2e/frame.html",
            "/%66rame.html",
            "/frame.html/extra",
            "/Frame.html",
            "/chat.jsonl",
            "/assets/../../chat.jsonl",
            "/provenance.json",
            "/media/00000000000000000000000000000000",
        ] {
            let result = response(&request(path), "main");
            assert_eq!(result.status(), StatusCode::NOT_FOUND, "{path}");
            assert!(!result
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
        }
        for url in [
            "http://atsumi-player.localhost/frame.html?file=C:/private",
            "http://atsumi-player.localhost:80/frame.html",
            "http://atsumi-player.localhost.evil.example/frame.html",
            "http://user@atsumi-player.localhost/frame.html",
            "https://chzzk.naver.com/frame.html",
            "file://localhost/frame.html",
        ] {
            let request = Request::builder().uri(url).body(Vec::new()).unwrap();
            assert_eq!(
                response(&request, "main").status(),
                StatusCode::BAD_REQUEST,
                "{url}"
            );
        }
    }

    #[test]
    fn head_retains_length_without_body_and_other_methods_are_rejected() {
        let get = response(&request("/frame.html"), "main");
        let mut head = request("/frame.html");
        *head.method_mut() = Method::HEAD;
        let head = response(&head, "main");
        assert_eq!(head.status(), StatusCode::OK);
        assert!(head.body().is_empty());
        assert_eq!(
            head.headers()[header::CONTENT_LENGTH],
            get.headers()[header::CONTENT_LENGTH]
        );
        for method in [Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS] {
            let mut request = request("/frame.html");
            *request.method_mut() = method;
            let result = response(&request, "main");
            assert_eq!(result.status(), StatusCode::METHOD_NOT_ALLOWED);
            assert_eq!(result.headers()[header::ALLOW], "GET, HEAD");
        }
        let mut body = request("/frame.html");
        body.body_mut().push(1);
        assert_eq!(response(&body, "main").status(), StatusCode::BAD_REQUEST);
    }
}
