use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceErrorCategory {
    Input,
    Missing,
    Remote,
    Contract,
}

impl SourceErrorCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Missing => "missing",
            Self::Remote => "remote",
            Self::Contract => "contract",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceErrorCode {
    Cancelled,
    Validation,
    NotFound,
    Protocol,
    InvalidData,
    RateLimited,
    TemporarilyUnavailable,
    Timeout,
    Unauthorized,
    Transport,
    ImageCandidatesExhausted,
    ImageResponseInvalid,
    ImageDecodeFailed,
    ImageFormatUnsupported,
}

impl SourceErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Validation => "validation",
            Self::NotFound => "not_found",
            Self::Protocol => "protocol",
            Self::InvalidData => "invalid_data",
            Self::RateLimited => "rate_limited",
            Self::TemporarilyUnavailable => "temporarily_unavailable",
            Self::Timeout => "timeout",
            Self::Unauthorized => "unauthorized",
            Self::Transport => "transport",
            Self::ImageCandidatesExhausted => "image_candidates_exhausted",
            Self::ImageResponseInvalid => "image_response_invalid",
            Self::ImageDecodeFailed => "image_decode_failed",
            Self::ImageFormatUnsupported => "image_format_unsupported",
        }
    }

    pub const fn category(self) -> SourceErrorCategory {
        match self {
            Self::Cancelled | Self::Validation => SourceErrorCategory::Input,
            Self::NotFound | Self::ImageCandidatesExhausted => SourceErrorCategory::Missing,
            Self::Protocol
            | Self::InvalidData
            | Self::ImageResponseInvalid
            | Self::ImageDecodeFailed
            | Self::ImageFormatUnsupported => SourceErrorCategory::Contract,
            Self::RateLimited
            | Self::TemporarilyUnavailable
            | Self::Timeout
            | Self::Unauthorized
            | Self::Transport => SourceErrorCategory::Remote,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCandidateDiagnostic {
    pub candidate_index: u32,
    pub format: String,
    pub http_status: Option<u16>,
    pub content_type: Option<String>,
    pub bytes_received: Option<u64>,
    pub error_code: Option<SourceErrorCode>,
    pub retryable: bool,
}

impl fmt::Display for SourceErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("{code}: {message}")]
#[serde(rename_all = "camelCase")]
pub struct SourceContractError {
    pub code: SourceErrorCode,
    pub category: SourceErrorCategory,
    pub message: String,
    pub retryable: bool,
    pub http_status: Option<u16>,
    pub retry_after_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_diagnostics: Vec<SourceCandidateDiagnostic>,
    #[serde(skip)]
    pub diagnostic_content_type: Option<String>,
    #[serde(skip)]
    pub diagnostic_bytes_received: Option<u64>,
}

impl SourceContractError {
    pub fn validation(field: impl AsRef<str>, message: impl AsRef<str>) -> Self {
        Self::new(
            SourceErrorCode::Validation,
            format!("{}: {}", field.as_ref(), message.as_ref()),
            false,
            None,
            None,
        )
    }

    pub fn cancelled() -> Self {
        Self::new(
            SourceErrorCode::Cancelled,
            "source request was cancelled",
            false,
            None,
            None,
        )
    }

    pub fn not_found(resource: impl Into<String>, http_status: Option<u16>) -> Self {
        Self::new(
            SourceErrorCode::NotFound,
            format!("{} was not found", resource.into()),
            false,
            http_status,
            None,
        )
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(SourceErrorCode::Protocol, message, false, None, None)
    }

    pub fn invalid_data(context: impl AsRef<str>, message: impl AsRef<str>) -> Self {
        Self::new(
            SourceErrorCode::InvalidData,
            format!("{}: {}", context.as_ref(), message.as_ref()),
            false,
            None,
            None,
        )
    }

    pub fn image_candidates_exhausted() -> Self {
        Self::new(
            SourceErrorCode::ImageCandidatesExhausted,
            "all supported image candidates were exhausted",
            false,
            None,
            None,
        )
    }

    pub fn image_response_invalid(message: impl Into<String>) -> Self {
        Self::new(
            SourceErrorCode::ImageResponseInvalid,
            message,
            false,
            None,
            None,
        )
    }

    pub fn image_decode_failed(message: impl Into<String>) -> Self {
        Self::new(
            SourceErrorCode::ImageDecodeFailed,
            message,
            false,
            None,
            None,
        )
    }

    pub fn image_format_unsupported(format: impl AsRef<str>) -> Self {
        Self::new(
            SourceErrorCode::ImageFormatUnsupported,
            format!("image format {} is unsupported", format.as_ref()),
            false,
            None,
            None,
        )
    }

    pub fn image_candidates_exhausted_with(
        retryable: bool,
        candidate_diagnostics: Vec<SourceCandidateDiagnostic>,
    ) -> Self {
        let mut error = Self::new(
            SourceErrorCode::ImageCandidatesExhausted,
            "all supported image candidates were exhausted",
            retryable,
            None,
            None,
        );
        error.candidate_diagnostics = candidate_diagnostics;
        error
    }

    fn new(
        code: SourceErrorCode,
        message: impl Into<String>,
        retryable: bool,
        http_status: Option<u16>,
        retry_after_seconds: Option<u64>,
    ) -> Self {
        Self {
            category: code.category(),
            code,
            message: message.into(),
            retryable,
            http_status,
            retry_after_seconds,
            candidate_diagnostics: Vec::new(),
            diagnostic_content_type: None,
            diagnostic_bytes_received: None,
        }
    }
}

/// Converts a completed HTTP response into the source contract used by callers.
/// Redirects are treated as protocol failures because the HTTP client is expected
/// to resolve allowed redirects before this boundary.
pub fn map_http_status(
    status: u16,
    retry_after_seconds: Option<u64>,
) -> Result<(), SourceContractError> {
    if !(100..=599).contains(&status) {
        return Err(SourceContractError::validation(
            "httpStatus",
            format!("must be between 100 and 599, got {status}"),
        ));
    }

    match status {
        200..=299 => Ok(()),
        404 | 410 => Err(SourceContractError::not_found(
            "remote source resource",
            Some(status),
        )),
        408 => Err(SourceContractError::new(
            SourceErrorCode::Timeout,
            "remote source request timed out",
            true,
            Some(status),
            retry_after_seconds,
        )),
        429 => Err(SourceContractError::new(
            SourceErrorCode::RateLimited,
            "remote source rate limit was reached",
            true,
            Some(status),
            retry_after_seconds,
        )),
        401 | 403 => Err(SourceContractError::new(
            SourceErrorCode::Unauthorized,
            format!("remote source rejected the request with HTTP {status}"),
            false,
            Some(status),
            None,
        )),
        500..=599 => Err(SourceContractError::new(
            SourceErrorCode::TemporarilyUnavailable,
            format!("remote source is temporarily unavailable (HTTP {status})"),
            true,
            Some(status),
            retry_after_seconds,
        )),
        _ => Err(SourceContractError::new(
            SourceErrorCode::Protocol,
            format!("remote source returned unexpected HTTP {status}"),
            false,
            Some(status),
            None,
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportFailureKind {
    Timeout,
    Connection,
    Dns,
    Other,
}

/// Maps transport-library-specific failures without coupling this module to a
/// particular HTTP client.
pub fn map_transport_failure(
    kind: TransportFailureKind,
    detail: impl AsRef<str>,
) -> SourceContractError {
    let detail = detail.as_ref().trim();
    let detail = if detail.is_empty() {
        "no transport detail was provided"
    } else {
        detail
    };

    match kind {
        TransportFailureKind::Timeout => SourceContractError::new(
            SourceErrorCode::Timeout,
            format!("remote source request timed out: {detail}"),
            true,
            None,
            None,
        ),
        TransportFailureKind::Connection
        | TransportFailureKind::Dns
        | TransportFailureKind::Other => SourceContractError::new(
            SourceErrorCode::Transport,
            format!("remote source transport failed: {detail}"),
            true,
            None,
            None,
        ),
    }
}
