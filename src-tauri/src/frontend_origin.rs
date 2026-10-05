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
