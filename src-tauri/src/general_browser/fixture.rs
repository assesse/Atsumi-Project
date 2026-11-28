//! Debug-only, isolated native UI fixture. Production navigation policy is unchanged.
use std::{path::PathBuf, sync::OnceLock};
pub struct Config {
    pub root: PathBuf,
    pub origin: String,
}
static CONFIG: OnceLock<Config> = OnceLock::new();
pub fn current() -> Option<&'static Config> {
    CONFIG.get()
}
pub fn configure() -> Result<(), String> {
    let Some(path) = std::env::var_os("ATSUMI_BROWSER_QA_ROOT") else {
        return Ok(());
    };
    let root = PathBuf::from(path)
        .canonicalize()
        .map_err(|_| "QA profile directory missing")?;
    let temp = std::env::temp_dir()
        .canonicalize()
        .map_err(|_| "QA temporary directory missing")?;
    if !root.starts_with(&temp)
        || !root
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("AtsumiBrowserQA-"))
        || std::fs::read_to_string(root.join("fixture-only.marker"))
            .ok()
            .as_deref()
            != Some("synthetic-local-browser-test")
    {
        return Err("QA profile must be a marked isolated temporary directory".into());
    }
    let url = tauri::Url::parse(
        &std::env::var("ATSUMI_BROWSER_QA_ORIGIN").map_err(|_| "QA origin missing")?,
    )
    .map_err(|_| "Invalid QA origin")?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().unwrap_or(0) < 1024
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("QA origin must be one explicit loopback HTTP port".into());
    }
    // Keep canonical containment validation, but pass an ordinary Windows path
    // to the browser engine, matching the production profile path representation.
    #[cfg(windows)]
    let root = {
        let text = root.to_string_lossy();
        match text.strip_prefix(r"\\?\") {
            Some(path) if path.as_bytes().get(1) == Some(&b':') => PathBuf::from(path),
            _ => root,
        }
    };
    CONFIG
        .set(Config {
            root,
            origin: url.origin().ascii_serialization(),
        })
        .map_err(|_| "QA profile already initialized".into())
}
