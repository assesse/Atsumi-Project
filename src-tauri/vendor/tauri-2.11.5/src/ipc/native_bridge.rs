// Atsumi-specific routing only; it grants no native or Tauri permissions.
pub(crate) fn is_atsumi_native_bridge_message(body: &str) -> bool {
    // Fixed-length comparisons only: never parse, allocate, or scan the payload.
    body.starts_with("ATSUMI_BROWSER_CAPTURE:")
        || body.starts_with("ATSUMI_SOOP_ACCOUNT_V1:")
        || body.starts_with("ATSUMI_SOOP_V1:")
        || body.starts_with("ATSUMI_YOUTUBE_V1:")
        || body.starts_with("ATSUMI_PANE_POINTER:")
}

#[cfg(test)]
mod tests {
    use super::is_atsumi_native_bridge_message;

    const PREFIXES: [&str; 5] = [
        "ATSUMI_BROWSER_CAPTURE:",
        "ATSUMI_SOOP_ACCOUNT_V1:",
        "ATSUMI_SOOP_V1:",
        "ATSUMI_YOUTUBE_V1:",
        "ATSUMI_PANE_POINTER:",
    ];

    #[test]
    fn native_bridge_prefix_is_payload_agnostic() {
        for prefix in PREFIXES {
            for suffix in [
                "",
                "{}",
                "{\"kind\":\"chunk\"}",
                "not JSON",
                "{\"cmd\":\"settings_factory_reset\"}",
            ] {
                assert!(is_atsumi_native_bridge_message(&format!(
                    "{prefix}{suffix}"
                )));
            }
            // Independent native listeners reject invalid/oversized messages.
            assert!(is_atsumi_native_bridge_message(&format!(
                "{prefix}{}",
                "x".repeat(128 * 1024)
            )));
        }
    }

    #[test]
    fn native_bridge_near_misses_still_use_normal_ipc_validation() {
        for prefix in PREFIXES {
            let stem = prefix.trim_end_matches(':');
            for body in [
                stem.to_string(),
                format!("{stem}X:{{}}"),
                format!("{stem} :{{}}"),
                prefix.to_ascii_lowercase(),
                format!(" {prefix}{{}}"),
                format!("\n{prefix}{{}}"),
                format!("\"{prefix}{{}}\""),
                format!("{{\"payload\":\"{prefix}{{}}\"}}"),
            ] {
                assert!(!is_atsumi_native_bridge_message(&body), "{body}");
            }
        }
    }

    #[test]
    fn ordinary_ipc_and_unknown_protocols_are_not_skipped() {
        for body in [
            "",
            "{\"cmd\":\"atsumi_test_ping\"}",
            "ATSUMI_UNKNOWN:{}",
            "ATSUMI_SOOP_ACCOUNT_V2:{}",
            "ATSUMI_YOUTUBE_V2:{}",
        ] {
            assert!(!is_atsumi_native_bridge_message(body), "{body}");
        }
    }
}
