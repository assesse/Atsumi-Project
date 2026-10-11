pub(crate) const PRODUCTION_ORIGINS: [&str; 3] = [
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
];

/// This value must come from native Tauri configuration, not a request/header.
pub(crate) fn development_origin(
    development: bool,
    dev_url: Option<&tauri::Url>,
) -> Option<String> {
    dev_url
        .filter(|url| {
            development
                && url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.username().is_empty()
                && url.password().is_none()
        })
        .map(|url| url.origin().ascii_serialization())
}

/// An Origin header is an exact origin, never a document URL or arbitrary local port.
pub(crate) fn trusted_origin(
    origin: &str,
    development: bool,
    dev_url: Option<&tauri::Url>,
) -> bool {
    PRODUCTION_ORIGINS.contains(&origin)
        || development_origin(development, dev_url).as_deref() == Some(origin)
}

/// Privileged IPC is limited to our main WebView. In development trust only the
/// exact loopback origin embedded by Tauri's config, never arbitrary local ports.
pub(crate) fn trusted_main(
    label: &str,
    url: &tauri::Url,
    development: bool,
    dev_url: Option<&tauri::Url>,
) -> bool {
    label == "main"
        && url.username().is_empty()
        && url.password().is_none()
        && (matches!(
            (url.scheme(), url.host_str(), url.port()),
            ("tauri", Some("localhost"), None) | ("http" | "https", Some("tauri.localhost"), None)
        ) || development
            && url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && dev_url.is_some_and(|configured| {
                configured.username().is_empty()
                    && configured.password().is_none()
                    && configured.origin() == url.origin()
            }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_headers_are_exact_and_development_is_opt_in() {
        let config = "http://127.0.0.1:14200/nested".parse().unwrap();
        assert!(trusted_origin(
            "http://127.0.0.1:14200",
            true,
            Some(&config)
        ));
        assert!(!trusted_origin(
            "http://127.0.0.1:14200",
            false,
            Some(&config)
        ));
        assert!(!trusted_origin("http://127.0.0.1:14200", true, None));
        for origin in [
            "null",
            "http://127.0.0.1:1420",
            "http://127.0.0.1:14201",
            "http://127.0.0.1:14200/",
            "http://127.0.0.1:14200/path",
            "http://user@127.0.0.1:14200",
            "http://tauri.localhost.evil.example",
        ] {
            assert!(!trusted_origin(origin, true, Some(&config)), "{origin}");
        }
        for origin in PRODUCTION_ORIGINS {
            assert!(trusted_origin(origin, false, None));
        }
    }

    #[test]
    fn configured_port_only_not_all_loopback_servers() {
        let config = "http://127.0.0.1:14200".parse().unwrap();
        assert!(trusted_main("main", &config, true, Some(&config)));
        for value in [
            "http://127.0.0.1:1420",
            "http://127.0.0.1:14201",
            "http://localhost:14200",
            "http://user@127.0.0.1:14200",
            "https://example.com",
        ] {
            assert!(
                !trusted_main("main", &value.parse().unwrap(), true, Some(&config)),
                "{value}"
            );
        }
        assert!(!trusted_main("main", &config, false, Some(&config)));
        assert!(!trusted_main("main", &config, true, None));
        assert!(!trusted_main(
            "official-browser",
            &config,
            true,
            Some(&config)
        ));
    }
}
