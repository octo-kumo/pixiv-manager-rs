//! Twitter (X) API errors, kept separate from [`pixivj_rs::error::PixivError`]
//! so the Pixiv client never has to know about the other provider.

use std::sync::Arc;

/// A failure from the cookie-authenticated Twitter GraphQL API.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TwitterError {
    /// Cookies are missing/expired or the session was revoked (401/403).
    #[error("Twitter login expired — update the cookie file in Settings ({0})")]
    Auth(String),
    /// The account hit X's rate limit.
    #[error("Twitter rate limited ({0})")]
    RateLimited(String),
    /// A GraphQL-level error (bad query id, deleted tweet, …).
    #[error("Twitter API error: {0}")]
    Api(String),
    /// A non-success HTTP response with no parseable error body.
    #[error("Twitter HTTP {0}: {1}")]
    HttpStatus(u16, String),
    /// The response body did not match the expected GraphQL shape.
    #[error("Twitter response decode failed: {0}")]
    Decode(String),
    /// The cookies file itself is missing or malformed.
    #[error("Twitter cookies: {0}")]
    Cookies(String),
    /// The requested tweet no longer exists / is not visible.
    #[error("tweet not found")]
    NotFound,
    /// Local file I/O (cookie file reads/writes).
    #[error("Twitter I/O: {0}")]
    Io(Arc<std::io::Error>),
    /// Anything else (join errors, malformed URLs, …).
    #[error("Twitter internal: {0}")]
    Internal(String),
}

impl From<reqwest::Error> for TwitterError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            TwitterError::Internal(format!("request timed out: {e}"))
        } else if e.is_connect() {
            TwitterError::Internal(format!("connection failed: {e}"))
        } else {
            TwitterError::Internal(e.to_string())
        }
    }
}

impl From<std::io::Error> for TwitterError {
    fn from(e: std::io::Error) -> Self {
        TwitterError::Io(Arc::new(e))
    }
}

/// Whether the error is a rate-limit response (retryable with backoff).
pub fn is_rate_limit(e: &TwitterError) -> bool {
    matches!(e, TwitterError::RateLimited(_))
}
