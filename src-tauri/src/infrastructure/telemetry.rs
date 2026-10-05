use std::sync::OnceLock;

use tracing_subscriber::{prelude::*, EnvFilter};

static TRACING_INITIALIZED: OnceLock<()> = OnceLock::new();

pub fn init() {
    TRACING_INITIALIZED.get_or_init(|| {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("atsumi=info,tauri=info"));

        let console = tracing_subscriber::fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .with_ansi(false)
            .with_filter(filter);
        // Persistent diagnostics remain enabled in release even without a
        // console or RUST_LOG. The layer only copies allowlisted fields.
        let local = crate::diagnostics::DiagnosticLayer::default().with_filter(
            tracing_subscriber::filter::filter_fn(|meta| {
                meta.target().starts_with("atsumi") && *meta.level() <= tracing::Level::INFO
            }),
        );
        if let Err(error) = tracing_subscriber::registry()
            .with(console)
            .with(local)
            .try_init()
        {
            eprintln!("structured tracing subscriber was not installed: {error}");
        }
    });
}
