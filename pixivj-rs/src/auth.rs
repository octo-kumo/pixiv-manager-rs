use crate::error::PixivError;
use crate::model::{AuthResult, GrantType};
use chrono::{DateTime, Utc};
use md5::{Digest, Md5};
use std::sync::Arc;
use std::time::Duration;

// ── Credential ─────────────────────────────────────────────────────────
// note that this is from the android app, it is public, this is not a leak
pub const CLIENT_ID: &str = "MOBrBDS8blbauoSck0ZfDbtuzpyT";
pub const CLIENT_SECRET: &str = "lsACyCD94FhDUtGTXi3QzcFE2uU1hqtDaKeqrdwj";
pub const DEFAULT_HASH_SECRET: &str =
    "28c1fdd170a5204386cb1313c7077b34f83e4aaf4aa829ce78c231e05b0bae2c";

#[derive(Debug, Clone)]
pub struct Credential {
    pub client_id: String,
    pub client_secret: String,
    pub grant_type: GrantType,
    pub refresh_token: Option<String>,
    pub code: Option<String>,
    pub code_verifier: Option<String>,
    pub redirect_uri: Option<String>,
    pub include_policy: Option<bool>,
    pub hash_secret: String,
}

impl Default for Credential {
    fn default() -> Self {
        Self {
            client_id: CLIENT_ID.into(),
            client_secret: CLIENT_SECRET.into(),
            grant_type: GrantType::RefreshToken,
            refresh_token: None,
            code: None,
            code_verifier: None,
            redirect_uri: None,
            include_policy: None,
            hash_secret: DEFAULT_HASH_SECRET.into(),
        }
    }
}

impl Credential {
    pub fn refresh(refresh_token: String) -> Self {
        Self {
            refresh_token: Some(refresh_token),
            ..Default::default()
        }
    }
}

// ── OAuth Client ───────────────────────────────────────────────────────

const OAUTH_BASE_URL: &str = "https://oauth.secure.pixiv.net";
const USER_AGENT: &str = "PixivAndroidApp/5.0.234 (Android 9.0; Pixel 3)";

#[derive(Debug, Clone)]
pub struct PixivOAuthClient {
    base_url: String,
    user_agent: String,
    http: reqwest::Client,
}

impl Default for PixivOAuthClient {
    fn default() -> Self {
        Self::new().expect("failed to build HTTP client")
    }
}

impl PixivOAuthClient {
    pub fn new() -> Result<Self, PixivError> {
        Self::new_with_proxy(None)
    }

    /// Build a client that routes traffic through `proxy` (e.g.
    /// `http://127.0.0.1:7890` or `socks5://…`). Invalid proxy URLs are
    /// silently ignored (the client falls back to a direct connection).
    pub fn new_with_proxy(proxy: Option<String>) -> Result<Self, PixivError> {
        let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
        if let Some(proxy) = proxy {
            if let Ok(proxy) = reqwest::Proxy::all(proxy) {
                builder = builder.proxy(proxy);
            }
        }
        let http = builder
            .build()
            .map_err(|e| PixivError::Internal(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            base_url: OAUTH_BASE_URL.into(),
            user_agent: USER_AGENT.into(),
            http,
        })
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    pub fn with_user_agent(mut self, ua: String) -> Self {
        self.user_agent = ua;
        self
    }

    pub async fn authenticate(&self, credential: &Credential) -> Result<AuthResult, PixivError> {
        let url = format!("{}/auth/token", self.base_url);

        let time_str = chrono::Utc::now().format("%+").to_string();
        let hash_input = format!("{}{}", time_str, credential.hash_secret);
        let client_hash: String = Md5::digest(hash_input.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();

        let mut form = vec![
            ("client_id".to_string(), credential.client_id.clone()),
            (
                "client_secret".to_string(),
                credential.client_secret.clone(),
            ),
            (
                "grant_type".to_string(),
                serde_json::to_string(&credential.grant_type)
                    .unwrap_or_else(|_| "\"refresh_token\"".into())
                    .trim_matches('"')
                    .to_string(),
            ),
        ];

        if let Some(ref rt) = credential.refresh_token {
            form.push(("refresh_token".to_string(), rt.clone()));
        }
        if let Some(ref code) = credential.code {
            form.push(("code".to_string(), code.clone()));
        }
        if let Some(ref cv) = credential.code_verifier {
            form.push(("code_verifier".to_string(), cv.clone()));
        }
        if let Some(ref ru) = credential.redirect_uri {
            form.push(("redirect_uri".to_string(), ru.clone()));
        }
        if let Some(ip) = credential.include_policy {
            form.push(("include_policy".to_string(), ip.to_string()));
        }

        let resp = self
            .http
            .post(&url)
            .header("User-Agent", &self.user_agent)
            .header("X-Client-Time", &time_str)
            .header("X-Client-Hash", &client_hash)
            .form(&form)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;

        if status.is_success() {
            let result: AuthResult = serde_json::from_str(&body)?;
            Ok(result)
        } else {
            let auth_err: crate::error::AuthError = serde_json::from_str(&body)?;
            Err(PixivError::Auth(auth_err))
        }
    }
}

// ── Token Provider ─────────────────────────────────────────────────────

/// Callback invoked with the newest refresh token whenever tokens are refreshed.
pub type OnTokenRefreshed = Arc<dyn Fn(String) + Send + Sync>;

// async-trait's macro adds `#[must_use]` to the generated method; its return
// type (a future) is already `#[must_use]`, so clippy's double_must_use is a
// false positive from the macro expansion.
#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
pub trait TokenProvider: Send + Sync {
    async fn get_token(&self) -> Result<String, PixivError>;

    /// The logged-in Pixiv user id (used e.g. to fetch one's own bookmarks).
    fn get_user_id(&self) -> Option<u64> {
        None
    }
}

pub struct FixedTokenProvider {
    token: String,
}

impl FixedTokenProvider {
    pub fn new(token: String) -> Self {
        Self { token }
    }
}

#[async_trait::async_trait]
impl TokenProvider for FixedTokenProvider {
    async fn get_token(&self) -> Result<String, PixivError> {
        Ok(self.token.clone())
    }
}

pub struct LazyTokenRefresher {
    client: PixivOAuthClient,
    access_token: tokio::sync::Mutex<Option<String>>,
    refresh_token: tokio::sync::Mutex<String>,
    expiry: tokio::sync::Mutex<Option<DateTime<Utc>>>,
    tolerance: Duration,
    /// Called with the newest refresh token after every refresh. Pixiv issues
    /// a fresh refresh token on each refresh, so callers should persist it.
    on_refresh: std::sync::Mutex<Option<OnTokenRefreshed>>,
    /// The logged-in user id, captured from the OAuth response.
    user_id: std::sync::Mutex<Option<u64>>,
    /// Serializes token refreshes (single-flight). Pixiv ROTATES the refresh
    /// token on every refresh; without this, concurrent requests that all see
    /// an expiring token would refresh in parallel, and the losers would
    /// authenticate with a token the winner just rotated — failing with a
    /// spurious auth error. Holding the lock makes the losers wait, then
    /// reuse the winner's freshly-refreshed token.
    refresh_lock: tokio::sync::Mutex<()>,
}

impl LazyTokenRefresher {
    pub fn new(client: PixivOAuthClient, refresh_token: String) -> Self {
        Self {
            client,
            access_token: tokio::sync::Mutex::new(None),
            refresh_token: tokio::sync::Mutex::new(refresh_token),
            expiry: tokio::sync::Mutex::new(None),
            tolerance: Duration::from_secs(600),
            on_refresh: std::sync::Mutex::new(None),
            user_id: std::sync::Mutex::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn with_tolerance(mut self, tolerance: Duration) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// Register a callback invoked with the newest refresh token whenever the
    /// token is refreshed (used to persist it across restarts).
    pub fn set_on_refresh(&self, f: impl Fn(String) + Send + Sync + 'static) {
        *self.on_refresh.lock().unwrap() = Some(Arc::new(f));
    }

    /// Record the logged-in user id from the OAuth response.
    pub fn set_user_id(&self, user_id: u64) {
        *self.user_id.lock().unwrap() = Some(user_id);
    }

    pub async fn update_tokens(
        &self,
        access_token: String,
        refresh_token: String,
        expires_in: i64,
    ) {
        *self.access_token.lock().await = Some(access_token);
        *self.refresh_token.lock().await = refresh_token.clone();
        *self.expiry.lock().await = Some(Utc::now() + chrono::Duration::seconds(expires_in));
        if let Some(cb) = self.on_refresh.lock().unwrap().clone() {
            cb(refresh_token);
        }
    }

    /// Authenticate immediately with the stored refresh token (fail-fast path,
    /// used by the CLI). Stores the access token and the ROTATED refresh
    /// token — Pixiv rotates it on every refresh — firing the persist
    /// callback with the new one. A no-op when the stored token is still
    /// within its validity window.
    pub async fn refresh_now(&self) -> Result<(), PixivError> {
        if !self.is_near_expire().await {
            return Ok(());
        }
        let refresh_token = self.refresh_token.lock().await.clone();
        let credential = Credential::refresh(refresh_token);
        let result = self.client.authenticate(&credential).await?;
        if let Some(user) = result.user.as_ref() {
            self.set_user_id(user.id);
        }
        self.update_tokens(result.access_token, result.refresh_token, result.expires_in)
            .await;
        Ok(())
    }

    async fn is_near_expire(&self) -> bool {
        let expiry = self.expiry.lock().await;
        match *expiry {
            None => true,
            Some(exp) => Utc::now() + chrono::Duration::from_std(self.tolerance).unwrap() >= exp,
        }
    }
}

#[async_trait::async_trait]
impl TokenProvider for LazyTokenRefresher {
    async fn get_token(&self) -> Result<String, PixivError> {
        if self.is_near_expire().await {
            // Single-flight refresh: the first caller to acquire the lock
            // refreshes; concurrent callers block until it finishes, then find
            // a freshly-refreshed (non-expiring) token below instead of racing
            // a second refresh against a rotated refresh token.
            let _guard = self.refresh_lock.lock().await;
            if self.is_near_expire().await {
                self.refresh_now().await?;
            }
        }
        let token = self.access_token.lock().await;
        token.clone().ok_or_else(|| {
            PixivError::Auth(crate::error::AuthError {
                has_error: true,
                errors: crate::error::AuthErrorDetails {
                    system: crate::error::AuthSystem {
                        code: -1,
                        message: "No access token available".into(),
                    },
                    message: Some("Not authenticated".into()),
                },
            })
        })
    }

    fn get_user_id(&self) -> Option<u64> {
        *self.user_id.lock().unwrap()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_hash_computation() {
        let time_str = "2024-01-01T00:00:00+00:00";
        let hash_secret = DEFAULT_HASH_SECRET;
        let input = format!("{}{}", time_str, hash_secret);
        let hash: String = Md5::digest(input.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        // Verify it's a 32-char hex string
        assert_eq!(hash.len(), 32);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn credential_default_uses_mobile_client() {
        let c = Credential::default();
        assert_eq!(c.client_id, CLIENT_ID);
        assert_eq!(c.client_secret, CLIENT_SECRET);
        assert_eq!(c.grant_type, GrantType::RefreshToken);
    }

    #[test]
    fn credential_refresh_sets_token() {
        let c = Credential::refresh("test_token".into());
        assert_eq!(c.refresh_token.as_deref(), Some("test_token"));
        assert!(c.code.is_none());
    }

    #[tokio::test]
    async fn on_refresh_callback_fires_with_new_token() {
        let refresher = LazyTokenRefresher::new(PixivOAuthClient::new().unwrap(), "old".into());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let seen2 = seen.clone();
        refresher.set_on_refresh(move |token| {
            *seen2.lock().unwrap() = Some(token);
        });

        refresher
            .update_tokens("access".into(), "new_token".into(), 3600)
            .await;

        assert_eq!(seen.lock().unwrap().as_deref(), Some("new_token"));
    }

    /// Serve `n` OAuth token responses on 127.0.0.1:0 and return the bound
    /// URL plus a counter of how many requests actually arrived.
    async fn mock_oauth_server(
        n: usize,
    ) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let counter2 = counter.clone();

        tokio::spawn(async move {
            for _ in 0..n {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await.unwrap();
                counter2.fetch_add(1, Ordering::SeqCst);
                let body = r#"{
                    "access_token": "access_1",
                    "expires_in": 3600,
                    "token_type": "Bearer",
                    "refresh_token": "rotated_1",
                    "user": {"id": 42, "name": "tester"}
                }"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            }
        });

        (format!("http://{addr}"), counter)
    }

    /// Regression test for the concurrent-refresh race: Pixiv ROTATES the
    /// refresh token on every refresh, so two requests that both see an
    /// expiring token and refresh in parallel would have one loser that
    /// authenticates with the freshly-rotated-away token and fails. The
    /// single-flight lock must collapse those to ONE refresh.
    #[tokio::test]
    async fn concurrent_get_token_refreshes_only_once() {
        let (url, counter) = mock_oauth_server(1).await;
        let client = PixivOAuthClient::new().unwrap().with_base_url(url);
        let refresher = std::sync::Arc::new(LazyTokenRefresher::new(client, "old".into()));

        let r1 = refresher.clone();
        let r2 = refresher.clone();
        let (a, b) = tokio::join!(
            tokio::spawn(async move { r1.get_token().await }),
            tokio::spawn(async move { r2.get_token().await }),
        );

        assert_eq!(a.unwrap().unwrap(), "access_1");
        assert_eq!(b.unwrap().unwrap(), "access_1");
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "two concurrent callers must share a single refresh"
        );
    }
}
