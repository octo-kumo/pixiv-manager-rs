use std::sync::Arc;
use thiserror::Error;

/// Top-level error type for all pixivj-rs operations.
#[derive(Debug, Clone, Error)]
pub enum PixivError {
    #[error("auth error: {0}")]
    Auth(#[from] AuthError),

    #[error("api error: {0}")]
    Api(#[from] ApiError),

    #[error("http error: {0}")]
    Http(Arc<reqwest::Error>),

    #[error("http status {0}: {1}")]
    HttpStatus(u16, String),

    #[error("json error: {0}")]
    Json(Arc<serde_json::Error>),

    #[error("io error: {0}")]
    Io(Arc<std::io::Error>),

    /// A response body failed to decode as the expected JSON shape.
    #[error("decode error: {0}")]
    Decode(String),

    /// The illust has no downloadable image URLs (page data missing).
    #[error("no downloadable image URLs")]
    NoImageUrl,

    /// An internal/client-side failure that isn't any of the above (e.g. a
    /// refused pagination URL or a client construction failure).
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<reqwest::Error> for PixivError {
    fn from(e: reqwest::Error) -> Self {
        PixivError::Http(Arc::new(e))
    }
}

impl From<serde_json::Error> for PixivError {
    fn from(e: serde_json::Error) -> Self {
        PixivError::Json(Arc::new(e))
    }
}

impl From<std::io::Error> for PixivError {
    fn from(e: std::io::Error) -> Self {
        PixivError::Io(Arc::new(e))
    }
}

/// Authentication error returned by the OAuth endpoint.
#[derive(Debug, Clone, serde::Deserialize, thiserror::Error)]
#[error("auth error: {}", errors.message.as_deref().unwrap_or("unknown"))]
pub struct AuthError {
    #[serde(rename = "has_error")]
    pub has_error: bool,
    pub errors: AuthErrorDetails,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AuthErrorDetails {
    pub system: AuthSystem,
    #[serde(rename = "message")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AuthSystem {
    pub code: i32,
    pub message: String,
}

/// API error returned by the app API endpoints.
#[derive(Debug, Clone, serde::Deserialize, thiserror::Error)]
#[error("api error: {}", errors.message.as_deref().unwrap_or("unknown"))]
pub struct ApiError {
    #[serde(default)]
    pub error: bool,
    pub errors: ApiErrorDetails,
}

impl ApiError {
    /// Whether this API error is a Pixiv rate-limit response (429 / "Rate Limit").
    pub fn is_rate_limit(&self) -> bool {
        let haystack = format!(
            "{} {}",
            self.errors.message.as_deref().unwrap_or(""),
            self.errors.user_message.as_deref().unwrap_or("")
        )
        .to_lowercase();
        self.errors.code == 429
            || haystack.contains("rate limit")
            || haystack.contains("too many requests")
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ApiErrorDetails {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub user_message: Option<String>,
    #[serde(default)]
    pub code: i32,
    #[serde(default)]
    pub user_message_details: Option<UserMessageDetails>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UserMessageDetails {
    #[serde(default)]
    pub message: Option<String>,
}
