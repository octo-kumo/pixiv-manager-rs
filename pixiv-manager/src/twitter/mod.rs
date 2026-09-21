//! Cookie-authenticated Twitter (X) GraphQL client.
//!
//! X has no supported API for third-party clients; this uses the same
//! internal GraphQL endpoints the web app calls, authenticated with the
//! user's exported browser cookies (`auth_token` + `ct0`). Query ids live in
//! [`graphql`] and rotate occasionally — a stale id surfaces as a plain
//! "Twitter API error" with the server's message.
//!
//! Everything the UI consumes is converted to the app's universal
//! [`Illustration`] (see [`convert`]) namespaced with [`crate::media`], so
//! the rest of the app treats tweets exactly like Pixiv illusts.

pub mod convert;
pub mod cookies;
pub mod error;
pub mod graphql;
mod query_ids;

pub use error::TwitterError;

use crate::media::native_id;
use cookies::TwitterCredentials;
use graphql as gql;
use pixivj_rs::model::{Comment, Illustration};
use pixivj_rs::rate_limit::{retry_after_header, RateLimiter};
use serde_json::{json, Value};
use std::sync::Arc;

/// A browser-shaped User-Agent: pbs.twimg.com and the API reject some
/// library clients outright.
pub const USER_AGENT: &str = crate::source::BROWSER_USER_AGENT;

/// How many timeline entries to request per page.
const PAGE_SIZE: u32 = 20;

/// Which home timeline the Twitter tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TwitterFeedMode {
    /// The algorithmic home timeline ("For You").
    #[default]
    ForYou,
    /// The chronological following timeline ("Following").
    Following,
}

impl TwitterFeedMode {
    pub fn label(self) -> &'static str {
        match self {
            TwitterFeedMode::ForYou => "For You",
            TwitterFeedMode::Following => "Following",
        }
    }
}

/// One page of converted media items plus the cursor for the next page.
#[derive(Debug, Clone, Default)]
pub struct TwitterPage {
    pub items: Vec<Illustration>,
    pub next_cursor: Option<String>,
}

/// The logged-in account, resolved by [`TwitterApiClient::viewer`].
#[derive(Debug, Clone)]
pub struct TwitterProfile {
    pub user_id: u64,
    pub handle: String,
}

/// The Twitter API client. Cheap to clone (shares the HTTP client, cookies,
/// GraphQL config and rate limiter).
#[derive(Clone)]
pub struct TwitterApiClient {
    http: reqwest::Client,
    base: Arc<String>,
    /// Runtime GraphQL metadata (query ids, features, bearer). Loaded from
    /// the compiled defaults plus an optional `twitter_graphql.json`, so
    /// query-id rotation never needs a rebuild.
    graphql: Arc<gql::GraphQlConfig>,
    creds: Arc<TwitterCredentials>,
    limiter: Arc<RateLimiter>,
}

impl std::fmt::Debug for TwitterApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TwitterApiClient")
            .field("base", &self.base)
            .field("user_id", &self.creds.user_id)
            .field("queries", &self.graphql.queries.len())
            .finish_non_exhaustive()
    }
}

impl TwitterApiClient {
    /// Build a client from parsed cookies with the compiled GraphQL defaults,
    /// routing through `proxy` when set.
    pub fn new(creds: TwitterCredentials, proxy: Option<String>) -> Result<Self, TwitterError> {
        Self::new_with_base(creds, proxy, gql::API_BASE.to_string())
    }

    /// Build a client with a runtime-updatable GraphQL config (query ids /
    /// features / bearer loaded from overrides on top of the defaults).
    pub fn with_config(
        creds: TwitterCredentials,
        proxy: Option<String>,
        graphql: gql::GraphQlConfig,
    ) -> Result<Self, TwitterError> {
        Self::build(creds, proxy, gql::API_BASE.to_string(), graphql)
    }

    /// Build against an explicit API base (used by tests) with the compiled
    /// GraphQL defaults.
    pub fn new_with_base(
        creds: TwitterCredentials,
        proxy: Option<String>,
        base: String,
    ) -> Result<Self, TwitterError> {
        Self::build(creds, proxy, base, gql::GraphQlConfig::compiled())
    }

    fn build(
        creds: TwitterCredentials,
        proxy: Option<String>,
        base: String,
        graphql: gql::GraphQlConfig,
    ) -> Result<Self, TwitterError> {
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
            .map_err(|e| TwitterError::Internal(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            http,
            base: Arc::new(base),
            graphql: Arc::new(graphql),
            creds: Arc::new(creds),
            limiter: Arc::new(RateLimiter::new()),
        })
    }

    /// The logged-in account's numeric id (from the `twid` cookie).
    pub fn user_id(&self) -> u64 {
        self.creds.user_id
    }

    /// The shared rate limiter (for UI readouts / debugging).
    pub fn rate_limiter(&self) -> Arc<RateLimiter> {
        Arc::clone(&self.limiter)
    }

    /// Scrape the current query ids from X's web bundles (in-app port of the
    /// old refresh script) and return this client's GraphQL config with them
    /// applied. Operations missing from the bundles keep their current value.
    pub async fn refresh_query_ids(&self) -> Result<gql::GraphQlConfig, TwitterError> {
        let ids = query_ids::fetch(&self.http, &self.creds.cookie_header).await?;
        let mut config = (*self.graphql).clone();
        for (operation, id) in ids {
            config.queries.insert(operation, id);
        }
        Ok(config)
    }

    // ── Endpoints ─────────────────────────────────────────────────────

    /// Resolve the logged-in account (validates the cookies on first use).
    pub async fn viewer(&self) -> Result<TwitterProfile, TwitterError> {
        let root = self
            .get_graphql(
                "Viewer",
                json!({
                    "withCommunitiesMemberships": true,
                    "withSubscribedTab": true,
                    "withCommunitiesCreation": true
                }),
            )
            .await?;
        let result = root.pointer("/data/viewer/user_results/result");
        let handle = result
            .and_then(|r| r.pointer("/core/screen_name"))
            .or_else(|| result.and_then(|r| r.pointer("/legacy/screen_name")))
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(TwitterProfile {
            user_id: self.creds.user_id,
            handle: handle.unwrap_or_else(|| format!("user{}", self.creds.user_id)),
        })
    }

    /// One page of the home timeline (For You or Following).
    pub async fn home_timeline(
        &self,
        mode: TwitterFeedMode,
        cursor: Option<&str>,
    ) -> Result<TwitterPage, TwitterError> {
        let (operation, variables) = match mode {
            TwitterFeedMode::ForYou => (
                "HomeTimeline",
                json!({
                    "count": PAGE_SIZE,
                    "cursor": cursor,
                    "includePromotedContent": true,
                    "latestControlAvailable": true,
                    "requestContext": "launch",
                    "withCommunity": true,
                    "seenTweetIds": []
                }),
            ),
            TwitterFeedMode::Following => (
                "HomeLatestTimeline",
                json!({
                    "count": PAGE_SIZE,
                    "cursor": cursor,
                    "includePromotedContent": false,
                    "latestControlAvailable": false,
                    "withCommunity": false,
                    "seenTweetIds": []
                }),
            ),
        };
        let root = self.get_graphql(operation, variables).await?;
        let entries = convert::home_entries(&root);
        Ok(page_from_entries(&entries))
    }

    /// One page of the account's liked tweets.
    pub async fn likes(&self, cursor: Option<&str>) -> Result<TwitterPage, TwitterError> {
        let root = self
            .get_graphql(
                "Likes",
                json!({
                    "userId": self.creds.user_id.to_string(),
                    "count": PAGE_SIZE,
                    "cursor": cursor,
                    "includePromotedContent": false,
                    "withClientEventToken": false,
                    "withBirdwatchNotes": false,
                    "withVoice": true
                }),
            )
            .await?;
        let entries = convert::likes_entries(&root);
        Ok(page_from_entries(&entries))
    }

    /// One page of a user's own tweets (the author page, photo tweets only).
    pub async fn user_tweets(
        &self,
        native_user_id: u64,
        cursor: Option<&str>,
    ) -> Result<TwitterPage, TwitterError> {
        let root = self
            .get_graphql(
                "UserTweets",
                json!({
                    "userId": native_user_id.to_string(),
                    "count": PAGE_SIZE,
                    "cursor": cursor,
                    "includePromotedContent": false,
                    "withQuickPromoteEligibilityTweetFields": true,
                    "withVoice": true,
                    "withV2Timeline": true
                }),
            )
            .await?;
        let entries = convert::user_tweets_entries(&root);
        Ok(page_from_entries(&entries))
    }

    /// A single tweet by native id.
    pub async fn tweet(&self, native_tweet_id: u64) -> Result<Illustration, TwitterError> {
        let root = self
            .get_graphql(
                "TweetResultByRestId",
                json!({
                    "tweetId": native_tweet_id.to_string(),
                    "withCommunity": false,
                    "includePromotedContent": false,
                    "withVoice": false
                }),
            )
            .await?;
        let result = root
            .pointer("/data/tweetResult/result")
            .ok_or(TwitterError::NotFound)?;
        let wrapper = json!({
            "content": {"itemContent": {"tweet_results": {"result": result}}}
        });
        let Some(tweet) = convert::entry_tweets(&wrapper).into_iter().next() else {
            return Err(TwitterError::NotFound);
        };
        convert::tweet_to_illust(tweet).ok_or(TwitterError::NotFound)
    }

    /// One page of replies to a tweet (the focal tweet itself is excluded).
    pub async fn replies(
        &self,
        native_tweet_id: u64,
        cursor: Option<&str>,
    ) -> Result<(Vec<Comment>, Option<String>), TwitterError> {
        let root = self
            .get_graphql(
                "TweetDetail",
                json!({
                    "focalTweetId": native_tweet_id.to_string(),
                    "cursor": cursor,
                    "referrer": "tweet",
                    "with_rux_injections": false,
                    "rankingMode": "Relevance",
                    "includePromotedContent": true,
                    "withCommunity": true,
                    "withQuickPromoteEligibilityTweetFields": true,
                    "withBirdwatchNotes": true,
                    "withVoice": true
                }),
            )
            .await?;
        let entries = convert::detail_entries(&root);
        let mut comments = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for entry in &entries {
            for tweet in convert::entry_tweets(entry) {
                let Some(comment) = convert::tweet_to_comment(tweet) else {
                    continue;
                };
                if native_id(comment.id) == native_tweet_id {
                    continue;
                }
                if seen.insert(comment.id) {
                    comments.push(comment);
                }
            }
        }
        Ok((comments, convert::next_cursor(&entries)))
    }

    /// Like (`liked == true`) or unlike a tweet.
    pub async fn set_like(&self, native_tweet_id: u64, liked: bool) -> Result<(), TwitterError> {
        let operation = if liked {
            "FavoriteTweet"
        } else {
            "UnfavoriteTweet"
        };
        self.post_graphql(operation, json!({"tweet_id": native_tweet_id.to_string()}))
            .await
            .map(|_| ())
    }

    // ── Request plumbing ──────────────────────────────────────────────

    /// The query id for an operation, or an explanatory error when the
    /// runtime config has no entry for it.
    fn query_id(&self, operation: &str) -> Result<&str, TwitterError> {
        self.graphql.query_id(operation).ok_or_else(|| {
            TwitterError::Internal(format!(
                "no GraphQL query id for {operation}; update twitter_graphql.json"
            ))
        })
    }

    async fn get_graphql(&self, operation: &str, variables: Value) -> Result<Value, TwitterError> {
        let query_id = self.query_id(operation)?;
        let url = format!("{}/graphql/{}/{}", self.base, query_id, operation);
        let _gate = self.limiter.wait_if_limited().await;
        let resp = self
            .request(reqwest::Method::GET, &url)
            .query(&[
                ("variables", variables.to_string()),
                ("features", self.graphql.features.clone()),
            ])
            .send()
            .await?;
        self.handle_response(resp).await
    }

    async fn post_graphql(&self, operation: &str, variables: Value) -> Result<Value, TwitterError> {
        let query_id = self.query_id(operation)?;
        let url = format!("{}/graphql/{}/{}", self.base, query_id, operation);
        let _gate = self.limiter.wait_if_limited().await;
        let body = json!({"variables": variables, "queryId": query_id});
        let resp = self
            .request(reqwest::Method::POST, &url)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await?;
        self.handle_response(resp).await
    }

    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, url)
            .header(
                "authorization",
                format!("Bearer {}", self.graphql.bearer_token),
            )
            .header("x-csrf-token", self.creds.ct0.as_str())
            .header("x-twitter-auth-type", "OAuth2Session")
            .header("x-twitter-active-user", "yes")
            .header("x-twitter-client-language", "en")
            .header("accept", "*/*")
            .header("user-agent", USER_AGENT)
            .header("cookie", self.creds.cookie_header.as_str())
    }

    /// Shared response handling: rate-limit reporting, status mapping and
    /// GraphQL error extraction.
    async fn handle_response(&self, resp: reqwest::Response) -> Result<Value, TwitterError> {
        let status = resp.status();
        self.limiter.on_request_done(
            status == reqwest::StatusCode::TOO_MANY_REQUESTS,
            retry_after_header(&resp),
        );
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(TwitterError::Auth(format!("HTTP {}", status.as_u16())));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(TwitterError::RateLimited("HTTP 429".into()));
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(TwitterError::Api(format!(
                "HTTP 404 — a Twitter query id may have rotated; update it in \
                 Settings → Twitter → \"Update query ids\" (HTTP {})",
                status.as_u16()
            )));
        }
        let body = resp.text().await?;
        let value: Value = serde_json::from_str(&body)
            .map_err(|e| TwitterError::Decode(format!("HTTP {}: {}", status.as_u16(), e)))?;
        if !status.is_success() {
            let message =
                graphql_message(&value).unwrap_or_else(|| body.chars().take(200).collect());
            return Err(TwitterError::HttpStatus(status.as_u16(), message));
        }
        // `errors` can accompany usable `data` (soft errors); only fail when
        // there is no data to work with.
        let data = value.get("data").filter(|d| !d.is_null());
        if data.is_none() {
            if let Some(message) = graphql_message(&value) {
                return Err(TwitterError::Api(message));
            }
        }
        Ok(value)
    }
}

/// The first message of a GraphQL `errors` array, if present.
fn graphql_message(value: &Value) -> Option<String> {
    value
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Convert entries into a page (photo-only, de-duplicated, order preserved).
fn page_from_entries(entries: &[&Value]) -> TwitterPage {
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        for tweet in convert::entry_tweets(entry) {
            let Some(illust) = convert::tweet_to_illust(tweet) else {
                continue;
            };
            if seen.insert(illust.id) {
                items.push(illust);
            }
        }
    }
    TwitterPage {
        items,
        next_cursor: convert::next_cursor(entries),
    }
}
