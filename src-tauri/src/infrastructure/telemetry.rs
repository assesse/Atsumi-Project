use std::sync::OnceLock;

use tracing_subscriber::EnvFilter;

static TRACING_INITIALIZED: OnceLock<()> = OnceLock::new();

pub fn init() {
    TRACING_INITIALIZED.get_or_init(|| {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("atsumi_next=info,tauri=info"));

        if let Err(error) = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .with_ansi(false)
            .try_init()
        {
            eprintln!("structured tracing subscriber was not installed: {error}");
        }
    });
}
