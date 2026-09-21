//! Cookie-authenticated Bilibili (B站) API client.
//!
//! Uses the web APIs directly with the user's exported cookies (`SESSDATA` +
//! `bili_jct` + `DedeUserID`): the followed-users "dynamic" feed
//! (t.bilibili.com), individual user feeds, post comments and the like
//! endpoint (Bilibili has no liked-posts feed, so bookmarks are tracked
//! locally — see [`crate::app::update::jobs`]).
//!
//! Only image posts are converted to the app's universal
//! [`Illustration`](pixivj_rs::model::Illustration) (see [`convert`]).

pub mod convert;
pub mod cookies;
pub mod error;
pub mod wbi;

pub use error::BilibiliError;

use cookies::BilibiliCredentials;
use pixivj_rs::model::{Comments, Illustration};
use pixivj_rs::rate_limit::{retry_after_header, RateLimiter};
use serde_json::{json, Value};
use std::sync::Arc;

/// A browser User-Agent: Bilibili's risk control rejects some library
/// clients outright.
pub const USER_AGENT: &str = crate::source::BROWSER_USER_AGENT;

/// The API host.
pub const API_BASE: &str = "https://api.bilibili.com";

/// Which dynamic feed the Bilibili tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BilibiliFeedMode {
    /// Everyone the account follows (t.bilibili.com).
    #[default]
    Following,
    /// The configured fixed list of user ids.
    Users,
}

impl BilibiliFeedMode {
    pub fn label(self) -> &'static str {
        match self {
            BilibiliFeedMode::Following => "Following",
            BilibiliFeedMode::Users => "Users",
        }
    }
}

/// One page of converted media items plus the cursor for the next page.
#[derive(Debug, Clone, Default)]
pub struct BilibiliPage {
    pub items: Vec<Illustration>,
    pub next_cursor: Option<String>,
}

/// The logged-in account, resolved by [`BilibiliApiClient::nav`].
#[derive(Debug, Clone)]
pub struct BilibiliProfile {
    pub user_id: u64,
    pub name: String,
}

/// The Bilibili API client. Cheap to clone (shares the HTTP client, cookies
/// and rate limiter).
#[derive(Clone)]
pub struct BilibiliApiClient {
    http: reqwest::Client,
    base: Arc<String>,
    creds: Arc<BilibiliCredentials>,
    limiter: Arc<RateLimiter>,
    /// Cached WBI keys (fetched from `nav` on first signing need).
    wbi: Arc<tokio::sync::RwLock<Option<wbi::WbiKeys>>>,
}

impl std::fmt::Debug for BilibiliApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BilibiliApiClient")
            .field("base", &self.base)
            .field("user_id", &self.creds.user_id)
            .finish_non_exhaustive()
    }
}

impl BilibiliApiClient {
    /// Build a client from parsed cookies, routing through `proxy` when set.
    pub fn new(creds: BilibiliCredentials, proxy: Option<String>) -> Result<Self, BilibiliError> {
        Self::new_with_base(creds, proxy, API_BASE.to_string())
    }

    /// Build against an explicit API base (used by tests).
    pub fn new_with_base(
        creds: BilibiliCredentials,
        proxy: Option<String>,
        base: String,
    ) -> Result<Self, BilibiliError> {
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            // Never follow redirects: the Cookie header must not leak to
            // another host.
            .redirect(reqwest::redirect::Policy::none());
        if let Some(proxy) = proxy {
            if let Ok(proxy) = reqwest::Proxy::all(proxy) {
                builder = builder.proxy(proxy);
            }
        }
        let http = builder
            .build()
            .map_err(|e| BilibiliError::Internal(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            http,
            base: Arc::new(base),
            creds: Arc::new(creds),
            limiter: Arc::new(RateLimiter::new()),
            wbi: Arc::new(tokio::sync::RwLock::new(None)),
        })
    }

    /// The logged-in account's uid (from the `DedeUserID` cookie).
    pub fn user_id(&self) -> u64 {
        self.creds.user_id
    }

    /// Whether write actions (likes) are possible.
    pub fn can_write(&self) -> bool {
        self.creds.can_write()
    }

    /// The shared rate limiter (for UI readouts / debugging).
    pub fn rate_limiter(&self) -> Arc<RateLimiter> {
        Arc::clone(&self.limiter)
    }

    // ── Endpoints ─────────────────────────────────────────────────────

    /// Resolve the logged-in account and cache the WBI keys (also validates
    /// the cookies).
    pub async fn nav(&self) -> Result<BilibiliProfile, BilibiliError> {
        let value = self
            .get_json("/x/web-interface/nav", &[], "https://www.bilibili.com/")
            .await?;
        let data = value.get("data").unwrap_or(&Value::Null);
        let logged_in = data
            .get("isLogin")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !logged_in {
            return Err(BilibiliError::Auth(
                "nav reports isLogin=false (cookies expired)".into(),
            ));
        }
        if let (Some(img), Some(sub)) = (
            data.pointer("/wbi_img/img_url").and_then(Value::as_str),
            data.pointer("/wbi_img/sub_url").and_then(Value::as_str),
        ) {
            *self.wbi.write().await = Some(wbi::WbiKeys::from_urls(img, sub));
        }
        Ok(BilibiliProfile {
            user_id: data
                .get("mid")
                .and_then(parse_u64_any)
                .unwrap_or(self.creds.user_id),
            name: data
                .get("uname")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }

    /// The followed-users feed (`t.bilibili.com`), one page at a time.
    pub async fn following_feed(&self, page: u32) -> Result<BilibiliPage, BilibiliError> {
        let value = self
            .get_json(
                "/x/polymer/web-dynamic/v1/feed/all",
                &[
                    ("type".to_string(), "all".to_string()),
                    ("page".to_string(), page.to_string()),
                ],
                "https://t.bilibili.com/",
            )
            .await?;
        self.page_from(&value, |data| {
            let has_more = data
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            has_more.then(|| (page + 1).to_string())
        })
    }

    /// A single user's dynamic feed (`space.bilibili.com/{uid}/dynamic`).
    pub async fn space_feed(
        &self,
        uid: u64,
        offset: Option<&str>,
    ) -> Result<BilibiliPage, BilibiliError> {
        let mut params = vec![("host_mid".to_string(), uid.to_string())];
        if let Some(offset) = offset.filter(|o| !o.is_empty()) {
            params.push(("offset".to_string(), offset.to_string()));
        } else {
            params.push(("offset".to_string(), String::new()));
        }
        params.push(("timezone_offset".to_string(), "-480".to_string()));
        let value = self
            .get_json(
                "/x/polymer/web-dynamic/v1/feed/space",
                &params,
                &format!("https://space.bilibili.com/{uid}/dynamic"),
            )
            .await?;
        self.page_from(&value, |data| {
            let has_more = data
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let offset = data.get("offset").and_then(Value::as_str);
            if has_more {
                offset.map(str::to_string)
            } else {
                None
            }
        })
    }

    /// A single dynamic by id.
    pub async fn dynamic(&self, native_id: u64) -> Result<Illustration, BilibiliError> {
        let value = self
            .get_json(
                "/x/polymer/web-dynamic/v1/detail",
                &[("id".to_string(), native_id.to_string())],
                "https://t.bilibili.com/",
            )
            .await?;
        let item = value.pointer("/data/item").ok_or(BilibiliError::NotFound)?;
        convert::dynamic_to_illust(item).ok_or(BilibiliError::NotFound)
    }

    /// One page of a dynamic's comments. `oid` is the post's comment id
    /// (`basic.comment_id_str`), `next` the 1-based page cursor.
    pub async fn comments(&self, oid: &str, next: u32) -> Result<Comments, BilibiliError> {
        let params = vec![
            ("oid".to_string(), oid.to_string()),
            ("type".to_string(), "11".to_string()),
            ("mode".to_string(), "3".to_string()),
            ("ps".to_string(), "20".to_string()),
            ("next".to_string(), next.to_string()),
            ("plat".to_string(), "1".to_string()),
        ];
        // Prefer the signed endpoint; fall back to the legacy unsigned one if
        // signing is unavailable or rejected.
        let signed = match self.wbi_keys().await {
            Ok(keys) => Some(wbi::sign(&params, &keys.mixin_key())),
            Err(_) => None,
        };
        let value = match signed {
            Some(query) => {
                match self
                    .get_json(
                        &format!("/x/v2/reply/wbi/main?{query}"),
                        &[],
                        "https://www.bilibili.com/",
                    )
                    .await
                {
                    Ok(value) => value,
                    Err(e) => {
                        log::debug!("[bilibili] signed comments failed ({e}), trying plain");
                        self.get_json("/x/v2/reply/main", &params, "https://www.bilibili.com/")
                            .await?
                    }
                }
            }
            None => {
                self.get_json("/x/v2/reply/main", &params, "https://www.bilibili.com/")
                    .await?
            }
        };
        let data = value.get("data").unwrap_or(&Value::Null);
        let mut comments = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for reply in data
            .get("replies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(comment) = convert::reply_to_comment(reply) {
                if seen.insert(comment.id) {
                    comments.push(comment);
                }
            }
        }
        let is_end = data
            .pointer("/cursor/is_end")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // `cursor.next` is the next page number.
        let next_url = if is_end {
            None
        } else {
            data.pointer("/cursor/next")
                .and_then(parse_u64_any)
                .map(|n| n.to_string())
        };
        Ok(Comments { comments, next_url })
    }

    /// Like or unlike a dynamic. Requires `bili_jct` (CSRF) in the cookies.
    pub async fn set_like(&self, native_id: u64, liked: bool) -> Result<(), BilibiliError> {
        if !self.creds.can_write() {
            return Err(BilibiliError::Cookies(
                "missing bili_jct cookie (cannot like)".into(),
            ));
        }
        let url = format!(
            "{}/x/dynamic/feed/dyn/thumb?csrf={}",
            self.base, self.creds.bili_jct
        );
        let body = json!({
            "dyn_id_str": native_id.to_string(),
            "up": if liked { 1 } else { 2 },
            "spmid": "333.1368.0.0",
        });
        self.post_json(&url, body).await.map(|_| ())
    }

    // ── Request plumbing ──────────────────────────────────────────────

    async fn wbi_keys(&self) -> Result<wbi::WbiKeys, BilibiliError> {
        if let Some(keys) = self.wbi.read().await.clone() {
            return Ok(keys);
        }
        // Fetching nav also refreshes the keys.
        self.nav().await?;
        self.wbi
            .read()
            .await
            .clone()
            .ok_or_else(|| BilibiliError::Internal("nav returned no WBI keys".into()))
    }

    async fn get_json(
        &self,
        path: &str,
        params: &[(String, String)],
        referer: &str,
    ) -> Result<Value, BilibiliError> {
        let url = format!("{}{}", self.base, path);
        let _gate = self.limiter.wait_if_limited().await;
        let resp = self
            .request(reqwest::Method::GET, &url, referer)
            .query(params)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    async fn post_json(&self, url: &str, body: Value) -> Result<Value, BilibiliError> {
        let _gate = self.limiter.wait_if_limited().await;
        let resp = self
            .request(reqwest::Method::POST, url, "https://t.bilibili.com/")
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await?;
        self.handle_response(resp).await
    }

    fn request(
        &self,
        method: reqwest::Method,
        url: &str,
        referer: &str,
    ) -> reqwest::RequestBuilder {
        self.http
            .request(method, url)
            .header("user-agent", USER_AGENT)
            .header("referer", referer)
            .header("accept", "application/json, text/plain, */*")
            .header("cookie", self.creds.cookie_header.as_str())
    }

    async fn handle_response(&self, resp: reqwest::Response) -> Result<Value, BilibiliError> {
        let status = resp.status();
        self.limiter.on_request_done(
            status == reqwest::StatusCode::TOO_MANY_REQUESTS,
            retry_after_header(&resp),
        );
        let body = resp.text().await?;
        let value: Value = serde_json::from_str(&body).map_err(|e| {
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                BilibiliError::Auth(format!("HTTP {}: {}", status.as_u16(), e))
            } else {
                BilibiliError::Decode(format!("HTTP {}: {}", status.as_u16(), e))
            }
        })?;
        if !status.is_success() {
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("request failed")
                .to_string();
            return Err(BilibiliError::HttpStatus(status.as_u16(), message));
        }
        check_api(value)
    }

    /// Convert a feed/detail response's `data` into a page.
    fn page_from(
        &self,
        value: &Value,
        next_cursor: impl Fn(&Value) -> Option<String>,
    ) -> Result<BilibiliPage, BilibiliError> {
        let data = value.get("data").unwrap_or(&Value::Null);
        let mut items = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for item in data
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(illust) = convert::dynamic_to_illust(item) {
                if seen.insert(illust.id) {
                    items.push(illust);
                }
            }
        }
        Ok(BilibiliPage {
            items,
            next_cursor: next_cursor(data),
        })
    }
}

/// Bilibili always answers HTTP 200 with a `code`; map non-zero codes onto
/// the error type.
fn check_api(value: Value) -> Result<Value, BilibiliError> {
    let code = value.get("code").and_then(Value::as_i64).unwrap_or(0);
    if code == 0 {
        return Ok(value);
    }
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Err(match code {
        -101 | -111 => BilibiliError::Auth(format!("{code}: {message}")),
        -352 | -412 => BilibiliError::RateLimited(format!("{code}: {message}")),
        -404 => BilibiliError::NotFound,
        _ => BilibiliError::Api(code, message),
    })
}

fn parse_u64_any(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}
