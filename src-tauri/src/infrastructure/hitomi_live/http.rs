use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant, SystemTime},
};

use reqwest::{
    blocking::{Client, Response},
    header::{
        ACCEPT, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE, REFERER, RETRY_AFTER,
        USER_AGENT,
    },
    redirect::Policy,
    Url,
};

use crate::{
    application::{image_work_budget, DownloadTuningProfile, DownloadTuningStore},
    source::{
        map_http_status, map_transport_failure, SourceContractError, SourceErrorCode,
        TransportFailureKind,
    },
    thumbnail::{CancellationToken, ThumbnailPriority},
};

use super::tuning::{DownloadSample, DownloadTuner, PROFILE_HOST};

const USER_AGENT_VALUE: &str = concat!(
    "Atsumi/",
    env!("CARGO_PKG_VERSION"),
    " (+desktop source adapter)"
);
const HITOMI_REFERER: &str = "https://hitomi.la/";
const DEFAULT_RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(30);
const DEFAULT_UNAVAILABLE_COOLDOWN: Duration = Duration::from_secs(2);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(50);
const RECOVERY_SUCCESS_COUNT: u32 = 30;
const RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HttpPriority {
    Critical,
    Visible,
    #[allow(dead_code)] // Reserved for page downloads on this shared scheduler.
    Download,
    Prefetch,
}

impl HttpPriority {
    const fn rank(self) -> u8 {
        match self {
            Self::Critical => 0,
            Self::Visible => 1,
            Self::Download => 2,
            Self::Prefetch => 3,
        }
    }
}

impl From<ThumbnailPriority> for HttpPriority {
    fn from(value: ThumbnailPriority) -> Self {
        match value {
            ThumbnailPriority::Critical => Self::Critical,
            ThumbnailPriority::Visible => Self::Visible,
            ThumbnailPriority::Prefetch => Self::Prefetch,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WaitingRequest {
    ticket: u64,
    priority: HttpPriority,
    host: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExpectedContent {
    Script,
    Html,
    Nozomi,
    Index,
    Image,
}

impl ExpectedContent {
    fn accept(self) -> &'static str {
        match self {
            Self::Script => "text/javascript, application/javascript;q=0.9, text/plain;q=0.5",
            Self::Html => "text/html, application/xhtml+xml;q=0.9",
            Self::Nozomi => "application/x-nozomi, application/octet-stream;q=0.5",
            Self::Index => "application/octet-stream, text/plain;q=0.5",
            Self::Image => "image/webp, image/avif;q=0.9, image/jpeg;q=0.8, image/png;q=0.7",
        }
    }

    fn accepts(self, content_type: &str) -> bool {
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        match self {
            Self::Script => matches!(
                mime.as_str(),
                "text/javascript"
                    | "application/javascript"
                    | "application/x-javascript"
                    | "text/plain"
            ),
            Self::Html => matches!(mime.as_str(), "text/html" | "application/xhtml+xml"),
            Self::Nozomi => {
                matches!(
                    mime.as_str(),
                    "application/x-nozomi" | "application/octet-stream"
                )
            }
            Self::Index => matches!(mime.as_str(), "application/octet-stream" | "text/plain"),
            Self::Image => {
                matches!(mime.as_str(), "" | "application/octet-stream")
                    || mime.starts_with("image/")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct HttpRequest {
    pub url: String,
    pub expected: ExpectedContent,
    pub max_bytes: usize,
    pub range: Option<String>,
    pub priority: HttpPriority,
    pub cancellation: Option<CancellationToken>,
}

#[derive(Debug, Clone)]
pub(super) struct HttpPayload {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub status: u16,
}

pub(super) trait HttpTransport: Send + Sync {
    fn execute(&self, request: HttpRequest) -> Result<HttpPayload, SourceContractError>;
}

pub(super) struct ReqwestTransport {
    client: Client,
    gate: Arc<RequestGate>,
    retry: RetryPolicy,
    tuning_store: Option<Arc<dyn DownloadTuningStore>>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct HttpSchedulerConfig {
    pub max_concurrent_requests: usize,
    pub max_concurrent_per_host: usize,
    pub request_start_interval: Duration,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_retries: u8,
    pub retry_base_delay: Duration,
    pub retry_max_delay: Duration,
}

#[derive(Debug, Clone, Copy)]
struct RetryPolicy {
    max_retries: u8,
    base_delay: Duration,
    max_delay: Duration,
}

impl ReqwestTransport {
    pub(super) fn new(config: HttpSchedulerConfig) -> Result<Self, SourceContractError> {
        let client = Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    return attempt.error("Hitomi redirect limit was exceeded");
                }
                if validate_source_url(attempt.url()).is_err() {
                    return attempt.error("Hitomi redirect target is outside the source allowlist");
                }
                attempt.follow()
            }))
            .build()
            .map_err(|error| {
                map_transport_failure(
                    TransportFailureKind::Other,
                    format!("could not construct HTTP client: {error}"),
                )
            })?;

        Ok(Self {
            client,
            gate: Arc::new(RequestGate::new(
                config.max_concurrent_requests.max(1),
                config.max_concurrent_per_host.max(1),
                config.request_start_interval,
            )),
            retry: RetryPolicy {
                max_retries: config.max_retries,
                base_delay: config.retry_base_delay,
                max_delay: config.retry_max_delay,
            },
            tuning_store: None,
        })
    }

    pub(super) fn with_download_tuning(
        mut self,
        ceiling: Option<usize>,
        store: Arc<dyn DownloadTuningStore>,
    ) -> Self {
        let profiles = store.load_all().unwrap_or_else(|error| {
            tracing::warn!(
                code = error.stable_code(),
                "download tuning history unavailable; using conservative start"
            );
            Vec::new()
        });
        let profile = profiles.iter().find(|profile| profile.host == PROFILE_HOST);
        let now = Instant::now();
        let wall_ms = unix_ms();
        let mut tuner = DownloadTuner::new(
            self.gate.max_active.min(5),
            ceiling.unwrap_or(self.gate.max_active),
            profile,
            now,
            wall_ms,
        );
        if ceiling.is_none() {
            tuner.automatic = false;
            tuner.current = self.gate.max_active.clamp(1, 8);
        }
        let mut state = unpoison(self.gate.state.lock());
        state.download_tuner = Some(tuner);
        // Restore actual host waits for thumbnails/metadata too, even when the
        // user disables learning. A restart is not a Retry-After override.
        for saved in &profiles {
            if saved.host != PROFILE_HOST && saved.cooldown_until_unix_ms > wall_ms {
                state.host_cooldowns.insert(
                    saved.host.clone(),
                    HostCooldown {
                        started: now,
                        duration: Duration::from_millis(saved.cooldown_until_unix_ms - wall_ms),
                    },
                );
            }
        }
        drop(state);
        self.tuning_store = Some(store);
        self
    }

    fn save_tuning(&self, profile: Option<DownloadTuningProfile>) {
        if let (Some(store), Some(profile)) = (&self.tuning_store, profile) {
            // No scheduler lock is held across SQLite I/O. Old concurrent saves
            // are rejected by the repository's monotonic observation timestamp.
            if let Err(error) = store.save(&profile) {
                tracing::warn!(
                    code = error.stable_code(),
                    "could not persist download tuning history"
                );
            }
        }
    }

    fn execute_once(
        &self,
        request: &HttpRequest,
        url: &Url,
        host: &str,
    ) -> Result<HttpPayload, SourceContractError> {
        ensure_not_cancelled(request.cancellation.as_ref())?;
        let permit = self
            .gate
            .acquire(host, request.priority, request.cancellation.as_ref())?;
        let service_started = Instant::now();
        let result = self.execute_acquired(request, url, host, &permit, service_started);
        if let Err(error) = &result {
            if request.priority == HttpPriority::Download
                && request.expected == ExpectedContent::Image
                && !matches!(
                    error.code,
                    SourceErrorCode::RateLimited
                        | SourceErrorCode::TemporarilyUnavailable
                        | SourceErrorCode::Cancelled
                )
            {
                self.save_tuning(self.gate.download_failure(
                    matches!(
                        error.code,
                        SourceErrorCode::Timeout | SourceErrorCode::Transport
                    ),
                    permit.tuning_generation,
                ));
            }
        }
        result
    }

    fn execute_acquired(
        &self,
        request: &HttpRequest,
        url: &Url,
        host: &str,
        permit: &RequestPermit,
        service_started: Instant,
    ) -> Result<HttpPayload, SourceContractError> {
        let mut builder = self
            .client
            .get(url.clone())
            .header(USER_AGENT, USER_AGENT_VALUE)
            .header(ACCEPT, request.expected.accept())
            .header(REFERER, HITOMI_REFERER);
        if let Some(range) = &request.range {
            builder = builder.header(RANGE, range);
        }

        let response = match builder.send() {
            Ok(response) => {
                self.gate.record_online();
                response
            }
            Err(error) => {
                let error = map_reqwest_error(error);
                self.gate.record_failure(error.code);
                return Err(error);
            }
        };
        ensure_not_cancelled(request.cancellation.as_ref())?;
        let status = response.status().as_u16();
        let retry_after = retry_after_seconds(&response);
        if status == 429 || status == 503 {
            let fallback = if status == 429 {
                DEFAULT_RATE_LIMIT_COOLDOWN
            } else {
                DEFAULT_UNAVAILABLE_COOLDOWN
            };
            let duration = retry_after.map(Duration::from_secs).unwrap_or(fallback);
            let profile = self.gate.cool_down(host, duration);
            if let Some(mut host_profile) = profile.clone() {
                host_profile.host = host.to_owned();
                host_profile.cooldown_until_unix_ms = self.gate.host_cooldown_until(host);
                host_profile.blocked_until_unix_ms = 0;
                host_profile.baseline_bytes_per_second = 0.0;
                self.save_tuning(Some(host_profile));
            }
            self.save_tuning(profile);
        }
        map_http_status(status, retry_after)?;
        if request.range.is_some() {
            if status != 206 {
                return Err(SourceContractError::protocol(
                    "ranged source request did not return HTTP 206",
                ));
            }
            let has_content_range = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.trim().starts_with("bytes "));
            if !has_content_range {
                return Err(SourceContractError::protocol(
                    "ranged source response omitted a valid Content-Range header",
                ));
            }
        }

        let result = read_payload(
            response,
            status,
            request.expected,
            request.max_bytes,
            request.cancellation.as_ref(),
        )
        .map_err(|mut error| {
            if error.http_status.is_none() {
                error.http_status = Some(status);
            }
            error
        });
        if result.is_ok() {
            self.gate.record_success(host, Instant::now());
        }
        if let Ok(payload) = &result {
            if request.priority == HttpPriority::Download
                && request.expected == ExpectedContent::Image
            {
                self.save_tuning(self.gate.observe_download(
                    DownloadSample {
                        bytes: payload.bytes.len(),
                        service_time: service_started.elapsed(),
                        had_demand: permit.had_download_demand,
                        processing_backlogged: image_work_budget::is_backlogged(),
                        stored_bytes: image_work_budget::stored_progress().0,
                    },
                    permit.tuning_generation,
                ));
            }
        }
        result
    }
}

impl HttpTransport for ReqwestTransport {
    fn execute(&self, request: HttpRequest) -> Result<HttpPayload, SourceContractError> {
        let url = Url::parse(&request.url).map_err(|error| {
            SourceContractError::validation("sourceUrl", format!("is malformed: {error}"))
        })?;
        validate_source_url(&url)?;
        if request.max_bytes == 0 {
            return Err(SourceContractError::validation(
                "maxBytes",
                "must be greater than zero",
            ));
        }
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let started = Instant::now();
        for attempt in 0..=self.retry.max_retries {
            let result = self.execute_once(&request, &url, &host);
            match result {
                Ok(payload) => {
                    tracing::debug!(
                        host,
                        priority = ?request.priority,
                        attempt = attempt + 1,
                        elapsed_ms = started.elapsed().as_millis(),
                        "source HTTP request completed"
                    );
                    return Ok(payload);
                }
                Err(error) => {
                    self.gate.record_unsuccessful(&host);
                    let retry = attempt < self.retry.max_retries && should_retry(&error);
                    tracing::warn!(
                        host,
                        priority = ?request.priority,
                        attempt = attempt + 1,
                        error_code = error.code.as_str(),
                        retry,
                        "source HTTP request failed"
                    );
                    if !retry {
                        return Err(error);
                    }
                    let delay = retry_delay(self.retry, &error, attempt + 1, &host);
                    self.gate.record_retry();
                    tracing::debug!(
                        host,
                        attempt = attempt + 1,
                        retry_delay_ms = delay.as_millis(),
                        "source HTTP retry scheduled"
                    );
                    wait_cooperatively(delay, request.cancellation.as_ref())?;
                }
            }
        }
        unreachable!("bounded retry loop always returns")
    }
}

fn read_payload(
    mut response: Response,
    status: u16,
    expected: ExpectedContent,
    max_bytes: usize,
    cancellation: Option<&CancellationToken>,
) -> Result<HttpPayload, SourceContractError> {
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim()
        .to_owned();
    if !expected.accepts(&content_type) {
        let detail = if content_type.is_empty() {
            "HTTP Content-Type header is missing".to_owned()
        } else {
            format!("HTTP Content-Type has unexpected value {content_type:?}")
        };
        let mut error = invalid_response(expected, detail);
        error.diagnostic_content_type = diagnostic_content_type(&content_type);
        return Err(error);
    }

    if let Some(content_length) = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        if content_length > max_bytes as u64 {
            let mut error = invalid_response(
                expected,
                format!("declared payload exceeds the {max_bytes}-byte limit"),
            );
            error.diagnostic_content_type = diagnostic_content_type(&content_type);
            return Err(error);
        }
    }

    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024));
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        ensure_not_cancelled(cancellation)?;
        let read = response.read(&mut chunk).map_err(|_| {
            map_transport_failure(
                TransportFailureKind::Connection,
                "response body could not be read",
            )
        })?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > max_bytes {
            let mut error = invalid_response(
                expected,
                format!("payload exceeds the {max_bytes}-byte limit"),
            );
            error.diagnostic_content_type = diagnostic_content_type(&content_type);
            error.diagnostic_bytes_received = u64::try_from(bytes.len()).ok();
            return Err(error);
        }
    }

    Ok(HttpPayload {
        bytes,
        content_type,
        status,
    })
}

fn diagnostic_content_type(content_type: &str) -> Option<String> {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if mime.is_empty() {
        None
    } else if mime.len() <= 127
        && mime
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'+' | b'-'))
    {
        Some(mime)
    } else {
        Some("invalid".into())
    }
}

fn map_reqwest_error(error: reqwest::Error) -> SourceContractError {
    if error.is_redirect() {
        return SourceContractError::protocol("source redirect policy rejected the response");
    }
    let (kind, detail) = if error.is_timeout() {
        (TransportFailureKind::Timeout, "request timed out")
    } else if error.is_connect() {
        (
            TransportFailureKind::Connection,
            "connection could not be established",
        )
    } else {
        (TransportFailureKind::Other, "transport request failed")
    };
    map_transport_failure(kind, detail)
}

fn invalid_response(expected: ExpectedContent, detail: impl Into<String>) -> SourceContractError {
    let detail = detail.into();
    if expected == ExpectedContent::Image {
        SourceContractError::image_response_invalid(detail)
    } else {
        SourceContractError::invalid_data("HTTP response", detail)
    }
}

fn should_retry(error: &SourceContractError) -> bool {
    matches!(
        error.code,
        SourceErrorCode::RateLimited
            | SourceErrorCode::TemporarilyUnavailable
            | SourceErrorCode::Timeout
            | SourceErrorCode::Transport
    )
}

fn retry_delay(
    policy: RetryPolicy,
    error: &SourceContractError,
    retry_number: u8,
    host: &str,
) -> Duration {
    let exponent = u32::from(retry_number.saturating_sub(1)).min(16);
    let factor = 1_u32.checked_shl(exponent).unwrap_or(u32::MAX);
    let backoff = policy
        .base_delay
        .saturating_mul(factor)
        .min(policy.max_delay);
    let jitter_ceiling_ms = u64::try_from((backoff / 4).as_millis()).unwrap_or(u64::MAX);
    let jitter_ms = if jitter_ceiling_ms == 0 {
        0
    } else {
        stable_jitter(host, retry_number) % (jitter_ceiling_ms + 1)
    };
    let calculated = backoff
        .saturating_add(Duration::from_millis(jitter_ms))
        .min(policy.max_delay);
    error
        .retry_after_seconds
        .map(Duration::from_secs)
        .map_or(calculated, |delay| delay.max(calculated))
}

fn stable_jitter(host: &str, retry_number: u8) -> u64 {
    host.as_bytes()
        .iter()
        .chain(std::iter::once(&retry_number))
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

fn ensure_not_cancelled(
    cancellation: Option<&CancellationToken>,
) -> Result<(), SourceContractError> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        Err(SourceContractError::cancelled())
    } else {
        Ok(())
    }
}

fn wait_cooperatively(
    duration: Duration,
    cancellation: Option<&CancellationToken>,
) -> Result<(), SourceContractError> {
    // Compare elapsed time instead of adding an untrusted server delay to an
    // Instant. Even an unrepresentable deadline must not panic or retry early.
    let started = Instant::now();
    loop {
        ensure_not_cancelled(cancellation)?;
        let remaining = duration.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Ok(());
        }
        thread::sleep(remaining.min(CANCELLATION_POLL_INTERVAL));
    }
}

fn retry_after_seconds(response: &Response) -> Option<u64> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_retry_after(value, SystemTime::now()))
}

fn parse_retry_after(value: &str, now: SystemTime) -> Option<u64> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        // Syntactically valid but oversized delays must not become a short
        // fallback. Saturation leaves the request cancellable while preventing
        // a retry within the lifetime of this process.
        return Some(value.parse().unwrap_or(u64::MAX));
    }

    let deadline = httpdate::parse_http_date(value).ok()?;
    let remaining = deadline.duration_since(now).unwrap_or(Duration::ZERO);
    // The public error contract uses whole seconds. Round up so converting an
    // HTTP-date never permits a retry before the server's deadline.
    Some(
        remaining
            .as_secs()
            .saturating_add(u64::from(remaining.subsec_nanos() != 0)),
    )
}

pub(super) fn validate_source_url(url: &Url) -> Result<(), SourceContractError> {
    if url.scheme() != "https" {
        return Err(SourceContractError::validation(
            "sourceUrl",
            "must use HTTPS",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SourceContractError::validation(
            "sourceUrl",
            "must not contain credentials",
        ));
    }
    if url.port().is_some_and(|port| port != 443) {
        return Err(SourceContractError::validation(
            "sourceUrl",
            "must not use a non-standard port",
        ));
    }

    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let allowed = host == "hitomi.la"
        || host == "ltn.gold-usergeneratedcontent.net"
        || host
            .strip_suffix(".gold-usergeneratedcontent.net")
            .is_some_and(|prefix| !prefix.is_empty() && !prefix.contains('.'));
    if !allowed {
        return Err(SourceContractError::validation(
            "sourceUrl",
            "host is outside the Hitomi source allowlist",
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct HostCooldown {
    started: Instant,
    duration: Duration,
}

impl HostCooldown {
    fn remaining_at(&self, now: Instant) -> Option<Duration> {
        let remaining = self
            .duration
            .saturating_sub(now.saturating_duration_since(self.started));
        (!remaining.is_zero()).then_some(remaining)
    }
}

#[derive(Debug)]
struct GateState {
    active: usize,
    active_downloads: usize,
    download_tuner: Option<DownloadTuner>,
    active_by_host: HashMap<String, usize>,
    last_started: Option<Instant>,
    host_cooldowns: HashMap<String, HostCooldown>,
    host_budgets: HashMap<String, AdaptiveHostBudget>,
    waiting: Vec<WaitingRequest>,
    next_ticket: u64,
    offline: bool,
    consecutive_transport_failures: u64,
    requests_started: u64,
    retries_scheduled: u64,
}

/// A response-driven reduction below the configured ceiling, never a limit probe.
#[derive(Debug)]
struct AdaptiveHostBudget {
    max_active: usize,
    start_interval: Duration,
    last_started: Option<Instant>,
    last_adjusted: Instant,
    successes: u32,
}

impl AdaptiveHostBudget {
    fn reduce(&mut self, now: Instant, base_interval: Duration) {
        self.max_active = (self.max_active / 2).max(1);
        self.start_interval = self
            .start_interval
            .saturating_mul(2)
            .max(Duration::from_millis(100))
            .min(Duration::from_secs(2))
            .max(base_interval);
        self.last_adjusted = now;
        self.successes = 0;
    }

    fn spacing_delay(&self, now: Instant) -> Option<Duration> {
        self.last_started
            .map(|last| {
                self.start_interval
                    .saturating_sub(now.saturating_duration_since(last))
            })
            .filter(|delay| !delay.is_zero())
    }

    fn record_success(&mut self, now: Instant, ceiling: usize, base_interval: Duration) -> bool {
        self.successes = self.successes.saturating_add(1);
        if self.successes < RECOVERY_SUCCESS_COUNT
            || now.saturating_duration_since(self.last_adjusted) < RECOVERY_INTERVAL
        {
            return false;
        }
        let previous = (self.max_active, self.start_interval);
        self.max_active = self.max_active.saturating_add(1).min(ceiling);
        self.start_interval = (self.start_interval / 2).max(base_interval);
        self.successes = 0;
        self.last_adjusted = now;
        previous != (self.max_active, self.start_interval)
    }
}

#[derive(Debug)]
struct RequestGate {
    max_active: usize,
    max_active_per_host: usize,
    start_interval: Duration,
    state: Mutex<GateState>,
    wake: Condvar,
}

impl RequestGate {
    fn new(max_active: usize, max_active_per_host: usize, start_interval: Duration) -> Self {
        Self {
            max_active,
            max_active_per_host,
            start_interval,
            state: Mutex::new(GateState {
                active: 0,
                active_downloads: 0,
                download_tuner: None,
                active_by_host: HashMap::new(),
                last_started: None,
                host_cooldowns: HashMap::new(),
                host_budgets: HashMap::new(),
                waiting: Vec::new(),
                next_ticket: 0,
                offline: false,
                consecutive_transport_failures: 0,
                requests_started: 0,
                retries_scheduled: 0,
            }),
            wake: Condvar::new(),
        }
    }

    fn acquire(
        self: &Arc<Self>,
        host: &str,
        priority: HttpPriority,
        cancellation: Option<&CancellationToken>,
    ) -> Result<RequestPermit, SourceContractError> {
        let mut state = unpoison(self.state.lock());
        let ticket = state.next_ticket;
        state.next_ticket = state.next_ticket.wrapping_add(1);
        state.waiting.push(WaitingRequest {
            ticket,
            priority,
            host: host.to_owned(),
        });
        loop {
            if cancellation.is_some_and(CancellationToken::is_cancelled) {
                state.waiting.retain(|candidate| candidate.ticket != ticket);
                self.wake.notify_all();
                return Err(SourceContractError::cancelled());
            }
            let now = Instant::now();
            let download_limit = state
                .download_tuner
                .as_ref()
                .map_or(self.max_active, |tuner| tuner.current);
            let total_limit = self.max_active.max(download_limit);
            let download_host_limit = state
                .download_tuner
                .as_ref()
                .filter(|tuner| tuner.automatic)
                .map_or(self.max_active_per_host, |_| {
                    self.max_active_per_host.max(download_limit)
                });
            let download_delay = (priority == HttpPriority::Download)
                .then(|| {
                    state
                        .download_tuner
                        .as_ref()
                        .and_then(|tuner| tuner.cooldown_remaining(now))
                })
                .flatten();
            let spacing_delay = state
                .last_started
                .map(|last| {
                    self.start_interval
                        .saturating_sub(now.saturating_duration_since(last))
                })
                .filter(|delay| !delay.is_zero());
            let host_delay = state
                .host_cooldowns
                .get(host)
                .and_then(|cooldown| cooldown.remaining_at(now));
            let adaptive_delay = state
                .host_budgets
                .get(host)
                .and_then(|budget| budget.spacing_delay(now));
            let host_limit = state.host_budgets.get(host).map_or_else(
                || {
                    if priority == HttpPriority::Download {
                        download_host_limit
                    } else {
                        self.max_active_per_host
                    }
                },
                |budget| {
                    budget
                        .max_active
                        .min(if priority == HttpPriority::Download {
                            download_host_limit
                        } else {
                            self.max_active_per_host
                        })
                },
            );
            let delay = spacing_delay
                .into_iter()
                .chain(host_delay)
                .chain(adaptive_delay)
                .chain(download_delay)
                .max();
            let host_active = state.active_by_host.get(host).copied().unwrap_or_default();
            let next_ticket = state
                .waiting
                .iter()
                .filter(|candidate| {
                    let candidate_host_active = state
                        .active_by_host
                        .get(&candidate.host)
                        .copied()
                        .unwrap_or_default();
                    let candidate_cooled_down = state
                        .host_cooldowns
                        .get(&candidate.host)
                        .is_some_and(|cooldown| cooldown.remaining_at(now).is_some());
                    let budget = state.host_budgets.get(&candidate.host);
                    let is_download = candidate.priority == HttpPriority::Download;
                    let candidate_limit = budget.map_or_else(
                        || {
                            if is_download {
                                download_host_limit
                            } else {
                                self.max_active_per_host
                            }
                        },
                        |budget| {
                            budget.max_active.min(if is_download {
                                download_host_limit
                            } else {
                                self.max_active_per_host
                            })
                        },
                    );
                    let class_available = if is_download {
                        state.active_downloads < download_limit
                            && state
                                .download_tuner
                                .as_ref()
                                .and_then(|tuner| tuner.cooldown_remaining(now))
                                .is_none()
                    } else {
                        state.active.saturating_sub(state.active_downloads) < self.max_active
                    };
                    candidate_host_active < candidate_limit
                        && class_available
                        && !candidate_cooled_down
                        && budget
                            .and_then(|budget| budget.spacing_delay(now))
                            .is_none()
                })
                .min_by_key(|candidate| (candidate.priority.rank(), candidate.ticket))
                .map(|candidate| candidate.ticket);
            if next_ticket == Some(ticket)
                && state.active < total_limit
                && host_active < host_limit
                && delay.is_none()
            {
                let had_download_demand = state.active_downloads.saturating_add(1)
                    >= download_limit
                    || state.waiting.iter().any(|candidate| {
                        candidate.ticket != ticket && candidate.priority == HttpPriority::Download
                    });
                let tuning_generation = state
                    .download_tuner
                    .as_ref()
                    .map_or(0, DownloadTuner::generation);
                state.waiting.retain(|candidate| candidate.ticket != ticket);
                state.active += 1;
                if priority == HttpPriority::Download {
                    state.active_downloads += 1;
                }
                *state.active_by_host.entry(host.to_owned()).or_default() += 1;
                state.last_started = Some(now);
                if let Some(budget) = state.host_budgets.get_mut(host) {
                    budget.last_started = Some(now);
                }
                state.requests_started = state.requests_started.saturating_add(1);
                return Ok(RequestPermit {
                    gate: Arc::clone(self),
                    host: host.to_owned(),
                    priority,
                    had_download_demand,
                    tuning_generation,
                });
            }

            state = if delay.is_some() || cancellation.is_some() {
                let wait = delay
                    .unwrap_or(CANCELLATION_POLL_INTERVAL)
                    .min(CANCELLATION_POLL_INTERVAL);
                let (guard, _) = self
                    .wake
                    .wait_timeout(state, wait)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard
            } else {
                unpoison(self.wake.wait(state))
            };
        }
    }

    fn cool_down(&self, host: &str, duration: Duration) -> Option<DownloadTuningProfile> {
        let mut state = unpoison(self.state.lock());
        let now = Instant::now();
        let current = state
            .host_cooldowns
            .get(host)
            .and_then(|cooldown| cooldown.remaining_at(now));
        if current.is_none_or(|remaining| duration > remaining) {
            state.host_cooldowns.insert(
                host.to_owned(),
                HostCooldown {
                    started: now,
                    duration,
                },
            );
        }
        let host_ceiling = state
            .download_tuner
            .as_ref()
            .filter(|tuner| tuner.automatic)
            .map_or(self.max_active_per_host, |tuner| {
                self.max_active_per_host.max(tuner.current)
            });
        let budget = state
            .host_budgets
            .entry(host.to_owned())
            .or_insert(AdaptiveHostBudget {
                max_active: host_ceiling,
                start_interval: self.start_interval,
                last_started: None,
                last_adjusted: now,
                successes: 0,
            });
        budget.reduce(now, self.start_interval);
        tracing::info!(
            host,
            effective_concurrency = budget.max_active,
            request_interval_ms = budget.start_interval.as_millis(),
            cooldown_seconds = duration.as_secs(),
            "source request budget reduced after server backpressure"
        );
        self.wake.notify_all();
        // Pause all download routes as well as this host. Other CDN shards must
        // not keep driving new download traffic through a server cooldown.
        state
            .download_tuner
            .as_mut()
            .map(|tuner| tuner.server_backpressure(duration, now, unix_ms()))
    }

    fn observe_download(
        &self,
        sample: DownloadSample,
        generation: u64,
    ) -> Option<DownloadTuningProfile> {
        let mut state = unpoison(self.state.lock());
        let tuner = state.download_tuner.as_mut()?;
        let previous = tuner.current;
        let profile = tuner.observe(sample, generation, Instant::now(), unix_ms());
        if previous != tuner.current {
            tracing::info!(
                previous,
                current = tuner.current,
                ceiling = tuner.ceiling,
                processing_pressure_epoch = image_work_budget::pressure_epoch(),
                "download concurrency adjusted from measured transfer performance"
            );
            self.wake.notify_all();
        }
        profile
    }

    fn host_cooldown_until(&self, host: &str) -> u64 {
        unpoison(self.state.lock())
            .host_cooldowns
            .get(host)
            .and_then(|cooldown| cooldown.remaining_at(Instant::now()))
            .map(|remaining| {
                unix_ms()
                    .saturating_add(u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX))
                    .saturating_add(1)
                    .min(i64::MAX as u64)
            })
            .unwrap_or_default()
    }

    fn download_failure(&self, congestion: bool, generation: u64) -> Option<DownloadTuningProfile> {
        let mut state = unpoison(self.state.lock());
        let tuner = state.download_tuner.as_mut()?;
        if tuner.generation() != generation {
            return None;
        }
        let profile = tuner.failure(congestion, Instant::now(), unix_ms());
        self.wake.notify_all();
        profile
    }

    fn record_success(&self, host: &str, now: Instant) {
        let mut state = unpoison(self.state.lock());
        if state
            .host_cooldowns
            .get(host)
            .is_some_and(|cooldown| cooldown.remaining_at(now).is_some())
        {
            return;
        }
        let host_ceiling = state
            .download_tuner
            .as_ref()
            .filter(|tuner| tuner.automatic)
            .map_or(self.max_active_per_host, |tuner| {
                self.max_active_per_host.max(tuner.current)
            });
        if let Some(budget) = state.host_budgets.get_mut(host) {
            if budget.record_success(now, host_ceiling, self.start_interval) {
                tracing::info!(
                    host,
                    effective_concurrency = budget.max_active,
                    request_interval_ms = budget.start_interval.as_millis(),
                    "source request budget cautiously recovered within configured ceiling"
                );
                self.wake.notify_all();
            }
        }
    }

    fn record_unsuccessful(&self, host: &str) {
        if let Some(budget) = unpoison(self.state.lock()).host_budgets.get_mut(host) {
            budget.successes = 0;
        }
    }

    fn record_online(&self) {
        let mut state = unpoison(self.state.lock());
        state.offline = false;
        state.consecutive_transport_failures = 0;
    }

    fn record_failure(&self, code: SourceErrorCode) {
        if code != SourceErrorCode::Transport {
            return;
        }
        let mut state = unpoison(self.state.lock());
        state.offline = true;
        state.consecutive_transport_failures =
            state.consecutive_transport_failures.saturating_add(1);
    }

    fn record_retry(&self) {
        let mut state = unpoison(self.state.lock());
        state.retries_scheduled = state.retries_scheduled.saturating_add(1);
    }

    fn release(&self, host: &str, priority: HttpPriority) {
        let mut state = unpoison(self.state.lock());
        state.active = state.active.saturating_sub(1);
        if priority == HttpPriority::Download {
            state.active_downloads = state.active_downloads.saturating_sub(1);
        }
        if let Some(active) = state.active_by_host.get_mut(host) {
            *active = active.saturating_sub(1);
            if *active == 0 {
                state.active_by_host.remove(host);
            }
        }
        self.wake.notify_all();
    }

    #[cfg(test)]
    fn snapshot(&self) -> GateSnapshot {
        let state = unpoison(self.state.lock());
        GateSnapshot {
            active: state.active,
            waiting: state.waiting.len(),
            offline: state.offline,
            consecutive_transport_failures: state.consecutive_transport_failures,
            requests_started: state.requests_started,
            retries_scheduled: state.retries_scheduled,
        }
    }
}

struct RequestPermit {
    gate: Arc<RequestGate>,
    host: String,
    priority: HttpPriority,
    had_download_demand: bool,
    tuning_generation: u64,
}

impl Drop for RequestPermit {
    fn drop(&mut self) {
        self.gate.release(&self.host, self.priority);
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| {
            u64::try_from(duration.as_millis())
                .unwrap_or(i64::MAX as u64)
                .min(i64::MAX as u64)
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GateSnapshot {
    active: usize,
    waiting: usize,
    offline: bool,
    consecutive_transport_failures: u64,
    requests_started: u64,
    retries_scheduled: u64,
}

fn unpoison<T>(result: std::sync::LockResult<MutexGuard<'_, T>>) -> MutexGuard<'_, T> {
    result.unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) fn stable_thumbnail_error(error: &SourceContractError) -> (&'static str, bool) {
    match error.code {
        SourceErrorCode::Cancelled => ("thumbnail request was cancelled", false),
        SourceErrorCode::NotFound => ("thumbnail was not found at the source", false),
        SourceErrorCode::Unauthorized => ("thumbnail access was rejected by the source", false),
        SourceErrorCode::RateLimited => ("thumbnail source is rate limiting requests", true),
        SourceErrorCode::TemporarilyUnavailable
        | SourceErrorCode::Timeout
        | SourceErrorCode::Transport => ("thumbnail source is temporarily unavailable", true),
        SourceErrorCode::ImageCandidatesExhausted => {
            ("all thumbnail source candidates were exhausted", false)
        }
        SourceErrorCode::ImageResponseInvalid => {
            ("thumbnail source returned a non-image response", false)
        }
        SourceErrorCode::ImageDecodeFailed => {
            ("thumbnail image could not be decoded safely", false)
        }
        SourceErrorCode::ImageFormatUnsupported => ("thumbnail image format is unsupported", false),
        SourceErrorCode::Validation | SourceErrorCode::Protocol | SourceErrorCode::InvalidData => {
            ("thumbnail source returned invalid data", false)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    use super::*;

    struct MemoryTuningStore(Vec<DownloadTuningProfile>);

    impl DownloadTuningStore for MemoryTuningStore {
        fn load_all(
            &self,
        ) -> Result<Vec<DownloadTuningProfile>, crate::application::RepositoryError> {
            Ok(self.0.clone())
        }
        fn save(
            &self,
            _: &DownloadTuningProfile,
        ) -> Result<(), crate::application::RepositoryError> {
            Ok(())
        }
    }

    fn transport_with_profile(ceiling: Option<usize>, cooldown: bool) -> ReqwestTransport {
        let now = unix_ms();
        let mut profiles = vec![DownloadTuningProfile {
            host: PROFILE_HOST.into(),
            algorithm_version: 1,
            stable_limit: 7,
            baseline_bytes_per_second: 10.0,
            updated_at_unix_ms: now.saturating_sub(1000),
            cooldown_until_unix_ms: if cooldown { now + 60_000 } else { 0 },
            blocked_until_unix_ms: 0,
        }];
        if cooldown {
            let mut host = profiles[0].clone();
            host.host = "a.gold-usergeneratedcontent.net".into();
            profiles.push(host);
        }
        ReqwestTransport::new(HttpSchedulerConfig {
            max_concurrent_requests: 5,
            max_concurrent_per_host: 5,
            request_start_interval: Duration::ZERO,
            connect_timeout: Duration::from_secs(1),
            request_timeout: Duration::from_secs(1),
            max_retries: 0,
            retry_base_delay: Duration::ZERO,
            retry_max_delay: Duration::ZERO,
        })
        .unwrap()
        .with_download_tuning(ceiling, Arc::new(MemoryTuningStore(profiles)))
    }

    #[test]
    fn restored_download_budget_really_allows_seven_and_blocks_the_eighth() {
        let transport = transport_with_profile(Some(8), false);
        let gate = &transport.gate;
        let mut permits = Vec::new();
        for _ in 0..7 {
            permits.push(
                gate.acquire("a.example", HttpPriority::Download, None)
                    .unwrap(),
            );
        }
        assert!(permits.last().unwrap().had_download_demand);
        assert_eq!(gate.snapshot().active, 7);
        let worker_gate = Arc::clone(gate);
        let token = CancellationToken::new();
        let worker_token = token.clone();
        let worker = thread::spawn(move || {
            worker_gate
                .acquire("a.example", HttpPriority::Download, Some(&worker_token))
                .map(drop)
        });
        wait_for_waiters(gate, 1);
        token.cancel();
        assert_eq!(
            worker.join().unwrap().unwrap_err().code,
            SourceErrorCode::Cancelled
        );
        drop(permits);
        assert_eq!(unpoison(gate.state.lock()).active_downloads, 0);
    }

    #[test]
    fn disabled_learning_preserves_manual_limit_and_restart_waits_on_all_download_routes() {
        let transport = transport_with_profile(None, true);
        let gate = &transport.gate;
        assert_eq!(
            unpoison(gate.state.lock())
                .download_tuner
                .as_ref()
                .unwrap()
                .current,
            5
        );
        let token = CancellationToken::new();
        let workers: Vec<_> = [
            ("other.example", HttpPriority::Download),
            ("a.gold-usergeneratedcontent.net", HttpPriority::Visible),
        ]
        .into_iter()
        .map(|(host, priority)| {
            let gate = Arc::clone(gate);
            let token = token.clone();
            thread::spawn(move || gate.acquire(host, priority, Some(&token)).map(drop))
        })
        .collect();
        wait_for_waiters(gate, 2);
        assert_eq!(gate.snapshot().active, 0);
        token.cancel();
        for worker in workers {
            assert_eq!(
                worker.join().unwrap().unwrap_err().code,
                SourceErrorCode::Cancelled
            );
        }
    }

    #[test]
    fn static_downloads_do_not_bypass_the_original_per_host_limit() {
        let gate = Arc::new(RequestGate::new(5, 1, Duration::ZERO));
        let blocker = gate
            .acquire("a.example", HttpPriority::Download, None)
            .unwrap();
        let token = CancellationToken::new();
        let worker_token = token.clone();
        let worker_gate = Arc::clone(&gate);
        let worker = thread::spawn(move || {
            worker_gate
                .acquire("a.example", HttpPriority::Download, Some(&worker_token))
                .map(drop)
        });
        wait_for_waiters(&gate, 1);
        assert_eq!(gate.snapshot().active, 1);
        token.cancel();
        assert_eq!(
            worker.join().unwrap().unwrap_err().code,
            SourceErrorCode::Cancelled
        );
        drop(blocker);
    }

    #[test]
    fn old_failure_samples_do_not_reduce_a_new_generation() {
        let transport = transport_with_profile(Some(8), false);
        let gate = &transport.gate;
        let old_generation = unpoison(gate.state.lock())
            .download_tuner
            .as_ref()
            .unwrap()
            .generation();
        gate.cool_down("a.example", Duration::ZERO);
        let reduced = unpoison(gate.state.lock())
            .download_tuner
            .as_ref()
            .unwrap()
            .current;
        for _ in 0..3 {
            assert!(gate.download_failure(true, old_generation).is_none());
        }
        assert_eq!(
            unpoison(gate.state.lock())
                .download_tuner
                .as_ref()
                .unwrap()
                .current,
            reduced
        );
    }

    #[test]
    fn disabled_learning_never_expands_static_host_limit_during_recovery() {
        let gate = RequestGate::new(30, 5, Duration::ZERO);
        let mut tuner = DownloadTuner::new(8, 8, None, Instant::now(), unix_ms());
        tuner.automatic = false;
        unpoison(gate.state.lock()).download_tuner = Some(tuner);
        gate.cool_down("a.example", Duration::ZERO);
        assert_eq!(
            unpoison(gate.state.lock()).host_budgets["a.example"].max_active,
            2
        );
        let now = Instant::now();
        for step in 1..=8 {
            for _ in 0..30 {
                gate.record_success("a.example", now + Duration::from_secs(step * 31));
            }
        }
        assert_eq!(
            unpoison(gate.state.lock()).host_budgets["a.example"].max_active,
            5
        );
    }

    #[test]
    fn persisted_host_wait_does_not_inherit_another_hosts_longer_global_wait() {
        let transport = transport_with_profile(Some(8), false);
        let gate = &transport.gate;
        gate.cool_down("a.example", Duration::from_secs(3600));
        let global = gate.cool_down("b.example", Duration::from_secs(1)).unwrap();
        assert!(global.cooldown_until_unix_ms > unix_ms() + 3_500_000);
        let b_deadline = gate.host_cooldown_until("b.example");
        assert!(b_deadline > unix_ms());
        assert!(b_deadline <= unix_ms() + 1001);
    }

    #[test]
    fn server_backpressure_reduces_concurrency_and_respects_slower_user_pacing() {
        let gate = RequestGate::new(5, 5, Duration::from_millis(25));
        gate.cool_down("a.example", Duration::ZERO);
        {
            let state = unpoison(gate.state.lock());
            assert_eq!(state.host_budgets["a.example"].max_active, 2);
            assert_eq!(
                state.host_budgets["a.example"].start_interval,
                Duration::from_millis(100)
            );
            assert!(!state.host_budgets.contains_key("b.example"));
        }
        for _ in 0..20 {
            gate.cool_down("a.example", Duration::ZERO);
        }
        let state = unpoison(gate.state.lock());
        assert_eq!(state.host_budgets["a.example"].max_active, 1);
        assert_eq!(
            state.host_budgets["a.example"].start_interval,
            Duration::from_secs(2)
        );
        drop(state);
        let slow = RequestGate::new(1, 1, Duration::from_secs(3));
        slow.cool_down("a.example", Duration::ZERO);
        assert_eq!(
            unpoison(slow.state.lock()).host_budgets["a.example"].start_interval,
            Duration::from_secs(3)
        );
    }

    #[test]
    fn adaptive_recovery_requires_both_elapsed_time_and_successes_and_never_exceeds_ceiling() {
        let now = Instant::now();
        let base = Duration::from_millis(25);
        let mut budget = AdaptiveHostBudget {
            max_active: 5,
            start_interval: base,
            last_started: None,
            last_adjusted: now,
            successes: 0,
        };
        budget.reduce(now, base);
        for _ in 0..30 {
            assert!(!budget.record_success(now + Duration::from_secs(29), 5, base));
        }
        assert_eq!(budget.max_active, 2);
        assert!(budget.record_success(now + Duration::from_secs(30), 5, base));
        assert_eq!(budget.max_active, 3);
        for _ in 0..29 {
            assert!(!budget.record_success(now + Duration::from_secs(60), 5, base));
        }
        assert!(budget.record_success(now + Duration::from_secs(60), 5, base));
        assert_eq!(budget.max_active, 4);
        for step in 3..12 {
            for _ in 0..30 {
                budget.record_success(now + Duration::from_secs(step * 30), 5, base);
            }
        }
        assert_eq!(budget.max_active, 5);
        assert_eq!(budget.start_interval, base);
    }

    #[test]
    fn cooldown_successes_are_ignored_and_failed_requests_reset_recovery() {
        let gate = RequestGate::new(5, 5, Duration::from_millis(25));
        gate.cool_down("a.example", Duration::from_secs(60));
        let now = unpoison(gate.state.lock()).host_budgets["a.example"].last_adjusted;
        for _ in 0..40 {
            gate.record_success("a.example", now + Duration::from_secs(31));
        }
        assert_eq!(
            unpoison(gate.state.lock()).host_budgets["a.example"].successes,
            0
        );
        for _ in 0..29 {
            gate.record_success("a.example", now + Duration::from_secs(61));
        }
        gate.record_unsuccessful("a.example");
        gate.record_success("a.example", now + Duration::from_secs(61));
        let state = unpoison(gate.state.lock());
        assert_eq!(state.host_budgets["a.example"].max_active, 2);
        assert_eq!(state.host_budgets["a.example"].successes, 1);
    }

    #[test]
    fn reduced_host_budget_blocks_new_work_until_active_requests_drain() {
        let gate = Arc::new(RequestGate::new(5, 5, Duration::ZERO));
        let first = gate
            .acquire("a.example", HttpPriority::Download, None)
            .unwrap();
        let second = gate
            .acquire("a.example", HttpPriority::Download, None)
            .unwrap();
        let third = gate
            .acquire("a.example", HttpPriority::Download, None)
            .unwrap();
        gate.cool_down("a.example", Duration::ZERO);
        let worker_gate = Arc::clone(&gate);
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let permit = worker_gate
                .acquire("a.example", HttpPriority::Critical, None)
                .unwrap();
            sender.send(permit).unwrap();
        });
        wait_for_waiters(&gate, 1);
        let other = gate
            .acquire("b.example", HttpPriority::Visible, None)
            .unwrap();
        assert!(receiver.try_recv().is_err());
        drop(first);
        assert!(receiver.recv_timeout(Duration::from_millis(50)).is_err());
        drop(second);
        let permitted = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        drop((third, other, permitted));
        assert_eq!(gate.snapshot().active, 0);
    }

    fn wait_for_waiters(gate: &RequestGate, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if gate.snapshot().waiting == expected {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!(
            "expected {expected} waiters, observed {}",
            gate.snapshot().waiting
        );
    }

    #[test]
    fn critical_requests_overtake_prefetch_waiters() {
        let gate = Arc::new(RequestGate::new(1, 1, Duration::ZERO));
        let blocker = gate
            .acquire(
                "ltn.gold-usergeneratedcontent.net",
                HttpPriority::Visible,
                None,
            )
            .unwrap();
        let (sender, receiver) = mpsc::channel();

        let prefetch_gate = Arc::clone(&gate);
        let prefetch_sender = sender.clone();
        let prefetch = thread::spawn(move || {
            let _permit = prefetch_gate
                .acquire(
                    "ltn.gold-usergeneratedcontent.net",
                    HttpPriority::Prefetch,
                    None,
                )
                .unwrap();
            prefetch_sender.send("prefetch").unwrap();
        });
        wait_for_waiters(&gate, 1);

        let critical_gate = Arc::clone(&gate);
        let critical = thread::spawn(move || {
            let _permit = critical_gate
                .acquire(
                    "ltn.gold-usergeneratedcontent.net",
                    HttpPriority::Critical,
                    None,
                )
                .unwrap();
            sender.send("critical").unwrap();
        });
        wait_for_waiters(&gate, 2);
        drop(blocker);

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            "critical"
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            "prefetch"
        );
        critical.join().unwrap();
        prefetch.join().unwrap();
        assert_eq!(gate.snapshot().requests_started, 3);
    }

    #[test]
    fn downloads_overtake_speculative_prefetch_waiters() {
        let gate = Arc::new(RequestGate::new(1, 1, Duration::ZERO));
        let blocker = gate
            .acquire(
                "ltn.gold-usergeneratedcontent.net",
                HttpPriority::Visible,
                None,
            )
            .unwrap();
        let (sender, receiver) = mpsc::channel();

        let prefetch_gate = Arc::clone(&gate);
        let prefetch_sender = sender.clone();
        let prefetch = thread::spawn(move || {
            let _permit = prefetch_gate
                .acquire(
                    "ltn.gold-usergeneratedcontent.net",
                    HttpPriority::Prefetch,
                    None,
                )
                .unwrap();
            prefetch_sender.send("prefetch").unwrap();
        });
        wait_for_waiters(&gate, 1);

        let download_gate = Arc::clone(&gate);
        let download = thread::spawn(move || {
            let _permit = download_gate
                .acquire(
                    "ltn.gold-usergeneratedcontent.net",
                    HttpPriority::Download,
                    None,
                )
                .unwrap();
            sender.send("download").unwrap();
        });
        wait_for_waiters(&gate, 2);
        drop(blocker);

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            "download"
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            "prefetch"
        );
        download.join().unwrap();
        prefetch.join().unwrap();
    }

    #[test]
    fn a_busy_host_does_not_block_an_eligible_host() {
        let gate = Arc::new(RequestGate::new(2, 1, Duration::ZERO));
        let blocker = gate
            .acquire("a.example", HttpPriority::Visible, None)
            .unwrap();
        let (sender, receiver) = mpsc::channel();

        let same_host_gate = Arc::clone(&gate);
        let same_host_sender = sender.clone();
        let same_host = thread::spawn(move || {
            let _permit = same_host_gate
                .acquire("a.example", HttpPriority::Critical, None)
                .unwrap();
            same_host_sender.send("same-host").unwrap();
        });
        wait_for_waiters(&gate, 1);

        let other_host_gate = Arc::clone(&gate);
        let other_host = thread::spawn(move || {
            let _permit = other_host_gate
                .acquire("b.example", HttpPriority::Visible, None)
                .unwrap();
            sender.send("other-host").unwrap();
        });
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            "other-host"
        );
        drop(blocker);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            "same-host"
        );
        other_host.join().unwrap();
        same_host.join().unwrap();
    }

    #[test]
    fn cancellation_removes_a_waiting_request() {
        let gate = Arc::new(RequestGate::new(1, 1, Duration::ZERO));
        let blocker = gate
            .acquire("source.example", HttpPriority::Visible, None)
            .unwrap();
        let cancellation = CancellationToken::new();
        let worker_token = cancellation.clone();
        let worker_gate = Arc::clone(&gate);
        let worker = thread::spawn(move || {
            worker_gate.acquire(
                "source.example",
                HttpPriority::Critical,
                Some(&worker_token),
            )
        });
        wait_for_waiters(&gate, 1);
        cancellation.cancel();

        let error = match worker.join().unwrap() {
            Ok(_) => panic!("cancelled waiter unexpectedly acquired a permit"),
            Err(error) => error,
        };
        assert_eq!(error.code, SourceErrorCode::Cancelled);
        assert_eq!(gate.snapshot().waiting, 0);
        drop(blocker);
    }

    #[test]
    fn retry_delay_is_bounded_and_honors_retry_after() {
        let policy = RetryPolicy {
            max_retries: 2,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
        };
        let unavailable = map_http_status(503, None).unwrap_err();
        let first = retry_delay(policy, &unavailable, 1, "ltn.example");
        let second = retry_delay(policy, &unavailable, 2, "ltn.example");
        assert!((Duration::from_millis(100)..=Duration::from_millis(125)).contains(&first));
        assert!((Duration::from_millis(200)..=Duration::from_millis(250)).contains(&second));

        let limited = map_http_status(429, Some(17)).unwrap_err();
        assert_eq!(
            retry_delay(policy, &limited, 1, "ltn.example"),
            Duration::from_secs(17)
        );

        assert_eq!(
            retry_delay(policy, &unavailable, u8::MAX, "ltn.example"),
            policy.max_delay
        );
        for status in [429, 503] {
            for seconds in [3600, u64::MAX] {
                let error = map_http_status(status, Some(seconds)).unwrap_err();
                assert_eq!(
                    retry_delay(policy, &error, 1, "ltn.example"),
                    Duration::from_secs(seconds)
                );
            }
        }
    }

    #[test]
    fn retry_after_parses_delay_seconds_without_shortening_large_values() {
        let now = SystemTime::UNIX_EPOCH;
        for (value, expected) in [
            ("0", 0),
            (" 17 ", 17),
            ("00017", 17),
            ("3600", 3600),
            ("18446744073709551615", u64::MAX),
            ("18446744073709551616", u64::MAX),
        ] {
            assert_eq!(parse_retry_after(value, now), Some(expected), "{value}");
        }
    }

    #[test]
    fn retry_after_parses_all_http_date_formats_and_rounds_up() {
        // 1994-11-06 08:49:37 UTC, fixed independently of the HTTP-date parser.
        let deadline = SystemTime::UNIX_EPOCH + Duration::from_secs(784_111_777);
        for value in [
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
        ] {
            assert_eq!(
                parse_retry_after(value, deadline - Duration::from_secs(3600)),
                Some(3600),
                "{value}"
            );
            assert_eq!(
                parse_retry_after(value, deadline - Duration::from_millis(1250)),
                Some(2),
                "{value}"
            );
            assert_eq!(parse_retry_after(value, deadline), Some(0), "{value}");
            assert_eq!(
                parse_retry_after(value, deadline + Duration::from_secs(1)),
                Some(0),
                "{value}"
            );
        }
    }

    #[test]
    fn malformed_retry_after_values_use_the_fallback() {
        for value in [
            "",
            " ",
            "not a date",
            "+17",
            "-1",
            "1.5",
            "17 seconds",
            "Sun, 32 Nov 1994 08:49:37 GMT",
        ] {
            assert_eq!(
                parse_retry_after(value, SystemTime::UNIX_EPOCH),
                None,
                "{value}"
            );
        }
    }

    #[test]
    fn host_cooldowns_preserve_long_delays_without_instant_overflow() {
        let started = Instant::now();
        for seconds in [3600, u64::MAX] {
            let cooldown = HostCooldown {
                started,
                duration: Duration::from_secs(seconds),
            };
            assert_eq!(
                cooldown.remaining_at(started),
                Some(Duration::from_secs(seconds))
            );
            assert_eq!(
                cooldown.remaining_at(started + Duration::from_secs(1)),
                Some(Duration::from_secs(seconds - 1))
            );
        }
        let cooldown = HostCooldown {
            started,
            duration: Duration::from_secs(1),
        };
        assert_eq!(
            cooldown.remaining_at(started + Duration::from_secs(1)),
            None
        );

        let gate = RequestGate::new(1, 1, Duration::ZERO);
        gate.cool_down("source.example", Duration::from_secs(u64::MAX));
        gate.cool_down("source.example", Duration::from_secs(1));
        assert_eq!(
            unpoison(gate.state.lock()).host_cooldowns["source.example"].duration,
            Duration::from_secs(u64::MAX)
        );
    }

    #[test]
    fn an_overflow_sized_host_cooldown_blocks_its_host_and_allows_cancellation() {
        let gate = Arc::new(RequestGate::new(1, 1, Duration::ZERO));
        gate.cool_down("source.example", Duration::from_secs(u64::MAX));
        let cancellation = CancellationToken::new();
        let worker_token = cancellation.clone();
        let worker_gate = Arc::clone(&gate);
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = worker_gate
                .acquire(
                    "source.example",
                    HttpPriority::Critical,
                    Some(&worker_token),
                )
                .map(drop);
            sender.send(result).unwrap();
        });
        wait_for_waiters(&gate, 1);
        assert_eq!(gate.snapshot().requests_started, 0);
        let other_host = gate
            .acquire("other.example", HttpPriority::Visible, None)
            .unwrap();
        cancellation.cancel();
        let error = receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, SourceErrorCode::Cancelled);
        worker.join().unwrap();
        assert_eq!(gate.snapshot().waiting, 0);
        drop(other_host);
    }

    #[test]
    fn cooperative_retry_waits_handle_zero_and_cancel_overflow_sized_delays() {
        assert!(wait_cooperatively(Duration::ZERO, None).is_ok());
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        for duration in [Duration::ZERO, Duration::from_secs(u64::MAX)] {
            assert_eq!(
                wait_cooperatively(duration, Some(&cancelled))
                    .unwrap_err()
                    .code,
                SourceErrorCode::Cancelled
            );
        }

        let cancellation = CancellationToken::new();
        let worker_token = cancellation.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            sender
                .send(wait_cooperatively(
                    Duration::from_secs(u64::MAX),
                    Some(&worker_token),
                ))
                .unwrap();
        });
        cancellation.cancel();
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap_err()
                .code,
            SourceErrorCode::Cancelled
        );
        worker.join().unwrap();
    }

    #[test]
    fn content_policy_rejects_html_and_maps_image_failures_to_stable_codes() {
        assert!(!ExpectedContent::Script.accepts("text/html; charset=utf-8"));
        assert!(!ExpectedContent::Image.accepts("text/html"));
        assert!(ExpectedContent::Image.accepts("image/webp"));
        assert!(ExpectedContent::Index.accepts("application/octet-stream"));
        assert!(ExpectedContent::Index.accepts("text/plain; charset=utf-8"));
        assert!(!ExpectedContent::Index.accepts("text/html"));
        assert_eq!(
            invalid_response(ExpectedContent::Image, "HTML error body").code,
            SourceErrorCode::ImageResponseInvalid
        );
        assert_eq!(
            invalid_response(ExpectedContent::Script, "HTML error body").code,
            SourceErrorCode::InvalidData
        );
    }

    #[test]
    fn transport_failures_track_offline_state_until_an_http_response_arrives() {
        let gate = RequestGate::new(1, 1, Duration::ZERO);
        gate.record_failure(SourceErrorCode::Transport);
        gate.record_failure(SourceErrorCode::Transport);
        let offline = gate.snapshot();
        assert!(offline.offline);
        assert_eq!(offline.consecutive_transport_failures, 2);

        gate.record_retry();
        gate.record_online();
        let online = gate.snapshot();
        assert!(!online.offline);
        assert_eq!(online.consecutive_transport_failures, 0);
        assert_eq!(online.retries_scheduled, 1);
        assert_eq!(online.active, 0);
    }
}
