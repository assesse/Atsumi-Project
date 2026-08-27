use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
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
    source::{
        map_http_status, map_transport_failure, SourceContractError, SourceErrorCode,
        TransportFailureKind,
    },
    thumbnail::{CancellationToken, ThumbnailPriority},
};

const USER_AGENT_VALUE: &str = concat!(
    "Atsumi/",
    env!("CARGO_PKG_VERSION"),
    " (+desktop source adapter)"
);
const HITOMI_REFERER: &str = "https://hitomi.la/";
const DEFAULT_RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(30);
const DEFAULT_UNAVAILABLE_COOLDOWN: Duration = Duration::from_secs(2);
const MAX_RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(5 * 60);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(50);

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
    Image,
}

impl ExpectedContent {
    fn accept(self) -> &'static str {
        match self {
            Self::Script => "text/javascript, application/javascript;q=0.9, text/plain;q=0.5",
            Self::Html => "text/html, application/xhtml+xml;q=0.9",
            Self::Nozomi => "application/x-nozomi, application/octet-stream;q=0.5",
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
        })
    }

    fn execute_once(
        &self,
        request: &HttpRequest,
        url: &Url,
        host: &str,
    ) -> Result<HttpPayload, SourceContractError> {
        ensure_not_cancelled(request.cancellation.as_ref())?;
        let _permit = self
            .gate
            .acquire(host, request.priority, request.cancellation.as_ref())?;
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
            let duration = retry_after
                .map(Duration::from_secs)
                .unwrap_or(fallback)
                .min(MAX_RATE_LIMIT_COOLDOWN);
            self.gate.cool_down(host, duration);
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

        read_payload(
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
        })
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
        .map(|delay| delay.min(MAX_RATE_LIMIT_COOLDOWN))
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
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        ensure_not_cancelled(cancellation)?;
        thread::sleep((deadline - Instant::now()).min(CANCELLATION_POLL_INTERVAL));
    }
    ensure_not_cancelled(cancellation)
}

fn retry_after_seconds(response: &Response) -> Option<u64> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
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
struct GateState {
    active: usize,
    active_by_host: HashMap<String, usize>,
    last_started: Option<Instant>,
    cooldown_until: HashMap<String, Instant>,
    waiting: Vec<WaitingRequest>,
    next_ticket: u64,
    offline: bool,
    consecutive_transport_failures: u64,
    requests_started: u64,
    retries_scheduled: u64,
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
                active_by_host: HashMap::new(),
                last_started: None,
                cooldown_until: HashMap::new(),
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
            let spacing_until = state
                .last_started
                .and_then(|last| last.checked_add(self.start_interval));
            let host_cooldown = state.cooldown_until.get(host).copied();
            let next_start = [spacing_until, host_cooldown].into_iter().flatten().max();
            let delay = next_start.and_then(|deadline| deadline.checked_duration_since(now));
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
                        .cooldown_until
                        .get(&candidate.host)
                        .is_some_and(|deadline| *deadline > now);
                    candidate_host_active < self.max_active_per_host && !candidate_cooled_down
                })
                .min_by_key(|candidate| (candidate.priority.rank(), candidate.ticket))
                .map(|candidate| candidate.ticket);
            if next_ticket == Some(ticket)
                && state.active < self.max_active
                && host_active < self.max_active_per_host
                && delay.is_none()
            {
                state.waiting.retain(|candidate| candidate.ticket != ticket);
                state.active += 1;
                *state.active_by_host.entry(host.to_owned()).or_default() += 1;
                state.last_started = Some(now);
                state.requests_started = state.requests_started.saturating_add(1);
                return Ok(RequestPermit {
                    gate: Arc::clone(self),
                    host: host.to_owned(),
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

    fn cool_down(&self, host: &str, duration: Duration) {
        let mut state = unpoison(self.state.lock());
        let deadline = Instant::now() + duration;
        let current = state.cooldown_until.get(host).copied();
        if current.is_none_or(|current| deadline > current) {
            state.cooldown_until.insert(host.to_owned(), deadline);
        }
        self.wake.notify_all();
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

    fn release(&self, host: &str) {
        let mut state = unpoison(self.state.lock());
        state.active = state.active.saturating_sub(1);
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
}

impl Drop for RequestPermit {
    fn drop(&mut self) {
        self.gate.release(&self.host);
    }
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
    }

    #[test]
    fn content_policy_rejects_html_and_maps_image_failures_to_stable_codes() {
        assert!(!ExpectedContent::Script.accepts("text/html; charset=utf-8"));
        assert!(!ExpectedContent::Image.accepts("text/html"));
        assert!(ExpectedContent::Image.accepts("image/webp"));
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
