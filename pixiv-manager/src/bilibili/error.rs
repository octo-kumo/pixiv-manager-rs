//! Bilibili API errors, kept separate from the Pixiv/Twitter error types so
//! each provider's client stays independent.

use std::sync::Arc;

/// A failure from the cookie-authenticated Bilibili API.
#[derive(Debug, Clone, thiserror::Error)]
pub enum BilibiliError {
    /// Not logged in / cookies expired (`code: -101`) or a CSRF failure.
    #[error("Bilibili login expired — update the cookie file in Settings ({0})")]
    Auth(String),
    /// Risk control / rate limit (`-352`, `-412`, HTTP 429).
    #[error("Bilibili rate limited ({0})")]
    RateLimited(String),
    /// Any other API-level error (carries the numeric code).
    #[error("Bilibili API error {0}: {1}")]
    Api(i64, String),
    /// A non-success HTTP response with no parseable API body.
    #[error("Bilibili HTTP {0}: {1}")]
    HttpStatus(u16, String),
    /// The response body did not match the expected shape.
    #[error("Bilibili response decode failed: {0}")]
    Decode(String),
    /// The cookies file itself is missing or malformed.
    #[error("Bilibili cookies: {0}")]
    Cookies(String),
    /// The requested dynamic no longer exists / is not visible.
    #[error("dynamic not found")]
    NotFound,
    /// Local file I/O (cookie file reads).
    #[error("Bilibili I/O: {0}")]
    Io(Arc<std::io::Error>),
    /// Anything else (join errors, malformed URLs, …).
    #[error("Bilibili internal: {0}")]
    Internal(String),
}

impl From<reqwest::Error> for BilibiliError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            BilibiliError::Internal(format!("request timed out: {e}"))
        } else if e.is_connect() {
            BilibiliError::Internal(format!("connection failed: {e}"))
        } else {
            BilibiliError::Internal(e.to_string())
        }
    }
}

impl From<std::io::Error> for BilibiliError {
    fn from(e: std::io::Error) -> Self {
        BilibiliError::Io(Arc::new(e))
    }
}

/// Whether the error is a rate-limit/risk-control response.
pub fn is_rate_limit(e: &BilibiliError) -> bool {
    matches!(e, BilibiliError::RateLimited(_))
}
