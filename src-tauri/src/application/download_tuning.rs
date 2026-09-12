use super::RepositoryError;

/// Durable, host-level observations from real download requests.
///
/// This profile contains no request URLs, paths, or gallery identifiers.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadTuningProfile {
    pub host: String,
    pub algorithm_version: u32,
    pub stable_limit: usize,
    pub baseline_bytes_per_second: f64,
    pub updated_at_unix_ms: u64,
    pub cooldown_until_unix_ms: u64,
    pub blocked_until_unix_ms: u64,
}

pub trait DownloadTuningStore: Send + Sync {
    fn load_all(&self) -> Result<Vec<DownloadTuningProfile>, RepositoryError>;
    fn save(&self, profile: &DownloadTuningProfile) -> Result<(), RepositoryError>;
}
