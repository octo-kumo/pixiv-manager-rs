use crate::auth::TokenProvider;
use crate::error::PixivError;
use crate::model::*;
use crate::rate_limit::RateLimiter;
use std::sync::Arc;

const API_BASE_URL: &str = "https://app-api.pixiv.net";
const USER_AGENT: &str = "PixivAndroidApp/5.0.234 (Android 9.0; Pixel 3)";

pub struct PixivApiClient {
    base_url: String,
    user_agent: String,
    http: reqwest::Client,
    token_provider: Arc<dyn TokenProvider>,
    /// Shared rate limiter (all clones share it, so concurrent jobs back off
    /// together when Pixiv rate-limits).
    rate_limiter: Arc<RateLimiter>,
}

impl Clone for PixivApiClient {
    fn clone(&self) -> Self {
        Self {
            base_url: self.base_url.clone(),
            user_agent: self.user_agent.clone(),
            http: self.http.clone(),
            token_provider: Arc::clone(&self.token_provider),
            rate_limiter: Arc::clone(&self.rate_limiter),
        }
    }
}

impl std::fmt::Debug for PixivApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PixivApiClient")
            .field("base_url", &self.base_url)
            .field("user_agent", &self.user_agent)
            .finish_non_exhaustive()
    }
}

impl PixivApiClient {
    /// The logged-in user id, if known (captured from the OAuth response).
    pub fn user_id(&self) -> Option<u64> {
        self.token_provider.get_user_id()
    }

    pub fn new(token_provider: Arc<dyn TokenProvider>) -> Result<Self, PixivError> {
        Self::new_with_proxy(token_provider, None)
    }

    /// Build a client that routes traffic through `proxy` (e.g.
    /// `http://127.0.0.1:7890` or `socks5://…`). Invalid proxy URLs are
    /// silently ignored (the client falls back to a direct connection).
    pub fn new_with_proxy(
        token_provider: Arc<dyn TokenProvider>,
        proxy: Option<String>,
    ) -> Result<Self, PixivError> {
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
            base_url: API_BASE_URL.into(),
            user_agent: USER_AGENT.into(),
            http,
            token_provider,
            rate_limiter: Arc::new(RateLimiter::new()),
        })
    }

    /// The shared rate limiter (for UI/debugging and for image fetches that
    /// want to respect the same backoff window).
    pub fn rate_limiter(&self) -> Arc<RateLimiter> {
        Arc::clone(&self.rate_limiter)
    }

    /// Ensure the access token is available (refreshing if needed), which also
    /// captures the logged-in user id — call this BEFORE building a request
    /// that needs `user_id`.
    pub async fn user_id_ensured(&self) -> Result<u64, PixivError> {
        self.token_provider.get_token().await?;
        self.user_id()
            .ok_or_else(|| PixivError::Internal("no user id available".into()))
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    async fn get(
        &self,
        path: &str,
        params: &[(String, String)],
    ) -> Result<reqwest::Response, PixivError> {
        // The guard is held for the WHOLE request (send + response) so the
        // shared backoff window lets only one waiting request through at a
        // time (see rate_limit.rs).
        let _gate = self.rate_limiter.wait_if_limited().await;
        let token = self.token_provider.get_token().await?;
        let url = format!("{}/{}", self.base_url, path);
        #[cfg(feature = "profiling")]
        let start = std::time::Instant::now();
        let mut req = self
            .http
            .get(&url)
            .header("User-Agent", &self.user_agent)
            .header("Authorization", format!("Bearer {}", token))
            .header("App-OS", "android")
            .header("Accept-Language", "en-us")
            .header("App-OS-Version", "9.0")
            .header("App-Version", "5.0.234");
        for (k, v) in params {
            req = req.query(&[(k.as_str(), v.as_str())]);
        }
        let resp = match req.send().await {
            Ok(resp) => {
                // Every outcome is reported so the gate is never stuck.
                self.rate_limiter.on_request_done(
                    resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS,
                    crate::rate_limit::retry_after_header(&resp),
                );
                #[cfg(feature = "profiling")]
                crate::tracing::record_http(
                    "GET",
                    url.clone(),
                    resp.status().as_u16(),
                    start.elapsed(),
                    resp.content_length().unwrap_or(0) as usize,
                );
                resp
            }
            Err(e) => {
                self.rate_limiter.on_request_done(false, None);
                #[cfg(feature = "profiling")]
                crate::tracing::record_http("GET", url.clone(), 0, start.elapsed(), 0);
                return Err(e.into());
            }
        };
        check_success(resp).await
    }

    async fn post(
        &self,
        path: &str,
        params: &[(String, String)],
    ) -> Result<reqwest::Response, PixivError> {
        let _gate = self.rate_limiter.wait_if_limited().await;
        let token = self.token_provider.get_token().await?;
        let url = format!("{}/{}", self.base_url, path);
        #[cfg(feature = "profiling")]
        let start = std::time::Instant::now();
        let mut req = self
            .http
            .post(&url)
            .header("User-Agent", &self.user_agent)
            .header("Authorization", format!("Bearer {}", token))
            .header("App-OS", "android")
            .header("Accept-Language", "en-us")
            .header("App-OS-Version", "9.0")
            .header("App-Version", "5.0.234");
        // Pixiv's write endpoints expect the parameters as an
        // `application/x-www-form-urlencoded` BODY, not URL query params.
        // Build ONE form with all params: calling `.form()` once per param
        // would overwrite, sending only the last one (which made bookmark
        // add/delete/follow fail with "Invalid request").
        let form: Vec<(&str, &str)> = params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        req = req.form(&form);
        let resp = match req.send().await {
            Ok(resp) => {
                // Every outcome is reported so the gate is never stuck.
                self.rate_limiter.on_request_done(
                    resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS,
                    crate::rate_limit::retry_after_header(&resp),
                );
                #[cfg(feature = "profiling")]
                crate::tracing::record_http(
                    "POST",
                    url.clone(),
                    resp.status().as_u16(),
                    start.elapsed(),
                    resp.content_length().unwrap_or(0) as usize,
                );
                resp
            }
            Err(e) => {
                self.rate_limiter.on_request_done(false, None);
                #[cfg(feature = "profiling")]
                crate::tracing::record_http("POST", url.clone(), 0, start.elapsed(), 0);
                return Err(e.into());
            }
        };
        check_success(resp).await
    }

    // ── Endpoints ──────────────────────────────────────────────────────

    pub async fn get_ranked_illusts(
        &self,
        filter: &RankedIllustsFilter,
    ) -> Result<RankedIllusts, PixivError> {
        let params = filter.to_query_params();
        let resp = self.get("v1/illust/ranking", &params).await?;
        send_expect_json(resp).await
    }

    pub async fn get_recommended_illusts(
        &self,
        filter: &RecommendedIllustsFilter,
    ) -> Result<RecommendedIllusts, PixivError> {
        let params = filter.to_query_params();
        let resp = self.get("v1/illust/recommended", &params).await?;
        send_expect_json(resp).await
    }

    pub async fn search_illusts(
        &self,
        filter: &SearchedIllustsFilter,
    ) -> Result<SearchedIllusts, PixivError> {
        let params = filter.to_query_params();
        let resp = self.get("v1/search/illust", &params).await?;
        send_expect_json(resp).await
    }

    pub async fn autocomplete(
        &self,
        filter: &SearchedIllustsFilter,
    ) -> Result<Autocomplete, PixivError> {
        let params = filter.to_query_params();
        let resp = self.get("v2/search/autocomplete", &params).await?;
        send_expect_json(resp).await
    }

    pub async fn get_illust_detail(&self, illust_id: u64) -> Result<IllustDetail, PixivError> {
        let params = vec![("illust_id".to_string(), illust_id.to_string())];
        let resp = self.get("v1/illust/detail", &params).await?;
        Ok(resp.json().await?)
    }

    pub async fn get_illust_comments(
        &self,
        filter: &IllustCommentsFilter,
    ) -> Result<Comments, PixivError> {
        let params = filter.to_query_params();
        // Illust comments moved to v3 (the v1/v2 endpoints return 404). The
        // v3 shape matches the `Comments` model used here.
        let resp = self.get("v3/illust/comments", &params).await?;
        send_expect_json(resp).await
    }

    pub async fn get_user_details(&self, user_id: u64) -> Result<UserDetail, PixivError> {
        let params = vec![("user_id".to_string(), user_id.to_string())];
        let resp = self.get("v1/user/detail", &params).await?;
        Ok(resp.json().await?)
    }

    pub async fn get_user_illusts(
        &self,
        query: &UserIllustQuery,
    ) -> Result<UserIllusts, PixivError> {
        let params = query.to_query_params();
        let resp = self.get("v1/user/illusts", &params).await?;
        send_expect_json(resp).await
    }

    pub async fn follow_user(&self, request: &FollowUserRequest) -> Result<(), PixivError> {
        let params = vec![
            ("user_id".to_string(), request.user_id.to_string()),
            (
                "restrict".to_string(),
                restrict_str(&request.restrict).to_string(),
            ),
        ];
        self.post("v1/user/follow/add", &params).await?;
        Ok(())
    }

    pub async fn unfollow_user(&self, request: &FollowUserRequest) -> Result<(), PixivError> {
        let params = vec![
            ("user_id".to_string(), request.user_id.to_string()),
            (
                "restrict".to_string(),
                restrict_str(&request.restrict).to_string(),
            ),
        ];
        self.post("v1/user/follow/delete", &params).await?;
        Ok(())
    }

    pub async fn get_bookmarks(&self, filter: &V2Filter) -> Result<SearchedIllusts, PixivError> {
        let params = filter.to_query_params();
        let resp = self.get("v1/user/bookmarks/illust", &params).await?;
        send_expect_json(resp).await
    }

    pub async fn get_follow_feed(&self, filter: &V2Filter) -> Result<SearchedIllusts, PixivError> {
        let params = filter.to_query_params();
        let resp = self.get("v2/illust/follow", &params).await?;
        send_expect_json(resp).await
    }

    pub async fn add_bookmark(
        &self,
        bookmark: &AddBookmark,
    ) -> Result<AddBookmarkResult, PixivError> {
        let params = vec![
            ("illust_id".to_string(), bookmark.illust_id.to_string()),
            (
                "restrict".to_string(),
                restrict_str(&bookmark.restrict).to_string(),
            ),
        ];
        let resp = self.post("v2/illust/bookmark/add", &params).await?;
        send_expect_json(resp).await
    }

    pub async fn remove_bookmark(&self, illust_id: u64) -> Result<(), PixivError> {
        let params = vec![("illust_id".to_string(), illust_id.to_string())];
        self.post("v1/illust/bookmark/delete", &params).await?;
        Ok(())
    }

    pub async fn download(&self, url: &str) -> Result<reqwest::Response, PixivError> {
        let _gate = self.rate_limiter.wait_if_limited().await;
        #[cfg(feature = "profiling")]
        let start = std::time::Instant::now();
        let resp = match self
            .http
            .get(url)
            .header("User-Agent", &self.user_agent)
            .header("Referer", &self.base_url)
            .send()
            .await
        {
            Ok(resp) => {
                self.rate_limiter.on_request_done(
                    resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS,
                    crate::rate_limit::retry_after_header(&resp),
                );
                #[cfg(feature = "profiling")]
                crate::tracing::record_http(
                    "GET",
                    url.to_string(),
                    resp.status().as_u16(),
                    start.elapsed(),
                    resp.content_length().unwrap_or(0) as usize,
                );
                resp
            }
            Err(e) => {
                self.rate_limiter.on_request_done(false, None);
                #[cfg(feature = "profiling")]
                crate::tracing::record_http("GET", url.to_string(), 0, start.elapsed(), 0);
                return Err(e.into());
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(PixivError::HttpStatus(
                status.as_u16(),
                "download failed".into(),
            ));
        }
        Ok(resp)
    }

    /// Fetch the next pagination page. `T` is the same JSON shape as the
    /// endpoint that produced `next_url` (e.g. `SearchedIllusts`,
    /// `RecommendedIllusts`).
    ///
    /// # Security contract
    ///
    /// The bearer token is attached to the request, so `next_url` is only
    /// honored when it is an `https` URL on `*.pixiv.net`; any other host is
    /// refused with an error. This keeps a malicious/compromised pagination
    /// URL from exfiltrating the access token.
    pub async fn fetch_by_next_url<T: serde::de::DeserializeOwned>(
        &self,
        next_url: &str,
    ) -> Result<T, PixivError> {
        let url = reqwest::Url::parse(next_url)
            .map_err(|_| PixivError::Internal(format!("invalid next_url: {next_url}")))?;
        let host_ok = matches!(
            url.host_str().map(|h| h.to_ascii_lowercase()),
            Some(h) if h == "pixiv.net" || h.ends_with(".pixiv.net")
        );
        if !host_ok || url.scheme() != "https" {
            return Err(PixivError::Internal(format!(
                "refusing to fetch next_url from non-pixiv.net host: {next_url}"
            )));
        }
        let token = self.token_provider.get_token().await?;
        #[cfg(feature = "profiling")]
        let start = std::time::Instant::now();
        let resp = self
            .http
            .get(next_url)
            .header("User-Agent", &self.user_agent)
            .header("Authorization", format!("Bearer {}", token))
            .header("App-OS", "android")
            .header("Accept-Language", "en-us")
            .send()
            .await?;
        #[cfg(feature = "profiling")]
        crate::tracing::record_http(
            "GET",
            next_url.to_string(),
            resp.status().as_u16(),
            start.elapsed(),
            resp.content_length().unwrap_or(0) as usize,
        );
        send_expect_json(resp).await
    }
}

// ── Shared response plumbing ───────────────────────────────────────────

/// Convert a non-success response into the appropriate [`PixivError`]
/// (preferring a parsed [`ApiError`] body), passing successful ones through.
async fn check_success(resp: reqwest::Response) -> Result<reqwest::Response, PixivError> {
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await?;
        if let Ok(api_err) = serde_json::from_str::<crate::error::ApiError>(&body) {
            return Err(PixivError::Api(api_err));
        }
        return Err(PixivError::HttpStatus(status.as_u16(), body));
    }
    Ok(resp)
}

/// The one place that turns a response body into typed JSON, uniformly
/// applying the non-success checks and decode-error mapping.
async fn send_expect_json<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, PixivError> {
    let resp = check_success(resp).await?;
    resp.json()
        .await
        .map_err(|e| PixivError::Decode(e.to_string()))
}

// ── Query params ───────────────────────────────────────────────────────

/// Convert a filter/request struct into URL query parameters.
///
/// Implemented per type (not via a generic serde roundtrip) so parameter
/// order is deterministic and nested values can never be stringified
/// lossily.
pub trait QueryParams {
    fn to_query_params(&self) -> Vec<(String, String)>;
}

/// The serde wire form of a leaf enum/primitive (e.g. `FilterMode::Day` →
/// `"day"`), following the model's own rename rules.
fn serde_str<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(serde_json::Value::Bool(b)) => b.to_string(),
        Ok(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn param(key: &str, value: impl Into<String>) -> (String, String) {
    (key.to_string(), value.into())
}

impl QueryParams for RankedIllustsFilter {
    fn to_query_params(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(v) = &self.filter {
            out.push(param("filter", serde_str(v)));
        }
        if let Some(v) = &self.mode {
            out.push(param("mode", serde_str(v)));
        }
        if let Some(v) = &self.date {
            out.push(param("date", v.clone()));
        }
        if let Some(v) = self.offset {
            out.push(param("offset", v.to_string()));
        }
        out
    }
}

impl QueryParams for RecommendedIllustsFilter {
    fn to_query_params(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(v) = &self.filter {
            out.push(param("filter", serde_str(v)));
        }
        if let Some(v) = &self.min_bookmark_id_for_recent_illust {
            out.push(param("min_bookmark_id_for_recent_illust", v.clone()));
        }
        if let Some(v) = &self.max_bookmark_id_for_recommend {
            out.push(param("max_bookmark_id_for_recommend", v.clone()));
        }
        if let Some(v) = self.offset {
            out.push(param("offset", v.to_string()));
        }
        if let Some(v) = self.include_ranking_illusts {
            out.push(param("include_ranking_illusts", v.to_string()));
        }
        if let Some(v) = self.include_privacy_policy {
            out.push(param("include_privacy_policy", v.to_string()));
        }
        out
    }
}

impl QueryParams for SearchedIllustsFilter {
    fn to_query_params(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(v) = &self.filter {
            out.push(param("filter", serde_str(v)));
        }
        if let Some(v) = self.include_translated_tag_results {
            out.push(param("include_translated_tag_results", v.to_string()));
        }
        if let Some(v) = self.merge_plain_keyword_results {
            out.push(param("merge_plain_keyword_results", v.to_string()));
        }
        out.push(param("word", self.word.clone()));
        if let Some(v) = &self.sort {
            out.push(param("sort", serde_str(v)));
        }
        if let Some(v) = self.offset {
            out.push(param("offset", v.to_string()));
        }
        if let Some(v) = &self.search_target {
            out.push(param("search_target", serde_str(v)));
        }
        out
    }
}

impl QueryParams for IllustCommentsFilter {
    fn to_query_params(&self) -> Vec<(String, String)> {
        let mut out = vec![param("illust_id", self.illust_id.to_string())];
        if let Some(v) = self.last_comment_id {
            out.push(param("last_comment_id", v.to_string()));
        }
        out
    }
}

impl QueryParams for UserIllustQuery {
    fn to_query_params(&self) -> Vec<(String, String)> {
        let mut out = vec![param("user_id", self.user_id.to_string())];
        if let Some(v) = &self.illust_type {
            out.push(param("type", serde_str(v)));
        }
        if let Some(v) = &self.filter {
            out.push(param("filter", serde_str(v)));
        }
        if let Some(v) = self.offset {
            out.push(param("offset", v.to_string()));
        }
        out
    }
}

impl QueryParams for V2Filter {
    fn to_query_params(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(v) = self.user_id {
            out.push(param("user_id", v.to_string()));
        }
        if let Some(v) = self.offset {
            out.push(param("offset", v.to_string()));
        }
        if let Some(v) = self.limit {
            out.push(param("limit", v.to_string()));
        }
        if let Some(v) = &self.restrict {
            out.push(param("restrict", restrict_str(v).to_string()));
        }
        if let Some(v) = &self.mode {
            out.push(param("mode", v.clone()));
        }
        out
    }
}

/// The wire form of [`Restrict`] ("public"/"private").
fn restrict_str(r: &Restrict) -> &'static str {
    match r {
        Restrict::Public => "public",
        Restrict::Private => "private",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_params_deterministic_and_typed() {
        let mut f = SearchedIllustsFilter::new("猫".into());
        f.sort = Some(SortOption::DateDesc);
        f.offset = Some(30);
        assert_eq!(
            f.to_query_params(),
            vec![
                ("word".to_string(), "猫".to_string()),
                ("sort".to_string(), "date_desc".to_string()),
                ("offset".to_string(), "30".to_string()),
            ]
        );
    }

    #[test]
    fn restrict_wire_forms() {
        assert_eq!(restrict_str(&Restrict::Public), "public");
        assert_eq!(restrict_str(&Restrict::Private), "private");
    }

    #[test]
    fn v2_filter_restrict_uses_wire_form() {
        let f = V2Filter {
            restrict: Some(Restrict::Private),
            limit: Some(10),
            ..Default::default()
        };
        assert_eq!(
            f.to_query_params(),
            vec![
                ("limit".to_string(), "10".to_string()),
                ("restrict".to_string(), "private".to_string()),
            ]
        );
    }
}
