use reqwest::StatusCode;
use thiserror::Error;

/// Error categories for actionable diagnostics
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Authentication/authorization issues - check API key
    Auth,
    /// Rate limiting - slow down requests
    RateLimit,
    /// Quota exceeded - upgrade plan or wait
    Quota,
    /// Network connectivity - check internet connection
    Network,
    /// Server-side error - retry later
    Server,
    /// Client-side error - fix request
    Client,
    /// Configuration error - fix config file
    Config,
    /// Cache error - check disk space/permissions
    Cache,
    /// Unknown error
    Unknown,
}

impl ErrorCategory {
    /// Get actionable advice for this error category
    pub fn advice(&self) -> &'static str {
        match self {
            Self::Auth => "Check your API credentials or authentication setup",
            Self::RateLimit => "Too many requests. Wait and retry with backoff",
            Self::Quota => "API quota exceeded. Wait for reset or upgrade plan",
            Self::Network => "Check internet connection and firewall settings",
            Self::Server => "Google Translate service issue. Retry in a few minutes",
            Self::Client => "Invalid request. Check input text encoding",
            Self::Config => "Fix configuration file syntax or values",
            Self::Cache => "Check disk space and file permissions for cache directory",
            Self::Unknown => "Unexpected error. Check logs for details",
        }
    }
}

#[derive(Error, Debug)]
pub enum TokenSaverError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Rate limited (HTTP 429){retry_msg}. {}", ErrorCategory::RateLimit.advice(), retry_msg = .retry_after_secs.map(|s| format!(", retry after {}s", s)).unwrap_or_default())]
    RateLimited {
        /// Server-suggested retry delay from Retry-After header
        retry_after_secs: Option<u64>,
    },

    #[error("HTTP {status} (retryable). {}", ErrorCategory::Server.advice())]
    RetryableHttp { status: StatusCode },

    #[error("Authentication failed (HTTP {status}). {}", ErrorCategory::Auth.advice())]
    AuthError { status: StatusCode },

    #[error("Quota exceeded (HTTP {status}). {}", ErrorCategory::Quota.advice())]
    QuotaExceeded { status: StatusCode },

    #[error("Translation failed: {0}")]
    Translation(String),

    #[error("Config error: {}. Fix configuration file syntax or values", .0)]
    Config(String),

    #[error("Cache error: {}. Check disk space and file permissions for cache directory", .0)]
    Cache(String),

    #[error(
        "Circuit breaker open. Translation service temporarily unavailable. Retry in {0} seconds"
    )]
    CircuitOpen(u64),

    #[error("Connection timeout. {}", ErrorCategory::Network.advice())]
    Timeout,

    #[error("Connection failed. {}", ErrorCategory::Network.advice())]
    ConnectionFailed,
}

impl TokenSaverError {
    /// Classify error into category for handling decisions
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::Io(_) => ErrorCategory::Cache,
            Self::Json(_) => ErrorCategory::Client,
            Self::Http(e) => {
                if e.is_timeout() || e.is_connect() {
                    ErrorCategory::Network
                } else if let Some(status) = e.status() {
                    Self::category_from_status(status)
                } else {
                    ErrorCategory::Unknown
                }
            }
            Self::RateLimited { .. } => ErrorCategory::RateLimit,
            Self::RetryableHttp { status } => Self::category_from_status(*status),
            Self::AuthError { .. } => ErrorCategory::Auth,
            Self::QuotaExceeded { .. } => ErrorCategory::Quota,
            Self::Translation(_) => ErrorCategory::Client,
            Self::Config(..) => ErrorCategory::Config,
            Self::Cache(..) => ErrorCategory::Cache,
            Self::CircuitOpen(_) => ErrorCategory::Server,
            Self::Timeout => ErrorCategory::Network,
            Self::ConnectionFailed => ErrorCategory::Network,
        }
    }

    /// Determine if this error should trigger a retry
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.category(),
            ErrorCategory::RateLimit | ErrorCategory::Server | ErrorCategory::Network
        )
    }

    /// Classify HTTP status code into error category
    fn category_from_status(status: StatusCode) -> ErrorCategory {
        match status.as_u16() {
            401 | 403 => ErrorCategory::Auth,
            429 => ErrorCategory::RateLimit,
            402 | 451 => ErrorCategory::Quota,
            400..=499 => ErrorCategory::Client,
            500..=599 => ErrorCategory::Server,
            _ => ErrorCategory::Unknown,
        }
    }

    /// Create appropriate error from HTTP status code
    pub fn from_status(status: StatusCode) -> Self {
        Self::from_status_with_retry_after(status, None)
    }

    /// Create error from HTTP status with optional Retry-After value
    pub fn from_status_with_retry_after(status: StatusCode, retry_after_secs: Option<u64>) -> Self {
        match status.as_u16() {
            401 | 403 => Self::AuthError { status },
            429 => Self::RateLimited { retry_after_secs },
            402 | 451 => Self::QuotaExceeded { status },
            500..=599 => Self::RetryableHttp { status },
            _ => Self::Translation(format!("HTTP {status}")),
        }
    }

    /// Extract retry_after_secs from RateLimited error
    pub fn retry_after_secs(&self) -> Option<u64> {
        match self {
            Self::RateLimited { retry_after_secs } => *retry_after_secs,
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, TokenSaverError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_categories() {
        assert_eq!(
            TokenSaverError::RateLimited {
                retry_after_secs: None
            }
            .category(),
            ErrorCategory::RateLimit
        );
        assert_eq!(
            TokenSaverError::RetryableHttp {
                status: StatusCode::SERVICE_UNAVAILABLE
            }
            .category(),
            ErrorCategory::Server
        );
        assert_eq!(
            TokenSaverError::AuthError {
                status: StatusCode::UNAUTHORIZED
            }
            .category(),
            ErrorCategory::Auth
        );
    }

    #[test]
    fn test_retryable_errors() {
        assert!(TokenSaverError::RateLimited {
            retry_after_secs: None
        }
        .is_retryable());
        assert!(TokenSaverError::RetryableHttp {
            status: StatusCode::BAD_GATEWAY
        }
        .is_retryable());
        assert!(TokenSaverError::Timeout.is_retryable());
        assert!(!TokenSaverError::Config("bad config".into()).is_retryable());
    }

    #[test]
    fn test_from_status() {
        assert!(matches!(
            TokenSaverError::from_status(StatusCode::UNAUTHORIZED),
            TokenSaverError::AuthError { .. }
        ));
        assert!(matches!(
            TokenSaverError::from_status(StatusCode::TOO_MANY_REQUESTS),
            TokenSaverError::RateLimited { .. }
        ));
        assert!(matches!(
            TokenSaverError::from_status(StatusCode::BAD_GATEWAY),
            TokenSaverError::RetryableHttp { .. }
        ));
    }

    #[test]
    fn test_error_messages_include_advice() {
        let err = TokenSaverError::RateLimited {
            retry_after_secs: None,
        };
        let msg = err.to_string();
        assert!(msg.contains("Wait and retry"));

        let err = TokenSaverError::RateLimited {
            retry_after_secs: Some(30),
        };
        let msg = err.to_string();
        assert!(msg.contains("retry after 30s"));

        let err = TokenSaverError::CircuitOpen(60);
        let msg = err.to_string();
        assert!(msg.contains("60 seconds"));
    }

    #[test]
    fn test_retry_after_extraction() {
        let err = TokenSaverError::RateLimited {
            retry_after_secs: Some(60),
        };
        assert_eq!(err.retry_after_secs(), Some(60));

        let err = TokenSaverError::RateLimited {
            retry_after_secs: None,
        };
        assert_eq!(err.retry_after_secs(), None);

        let err = TokenSaverError::Timeout;
        assert_eq!(err.retry_after_secs(), None);
    }
}
