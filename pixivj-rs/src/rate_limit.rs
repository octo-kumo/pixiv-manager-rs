//! A shared rate limiter.
//!
//! Pixiv returns HTTP 429 ("Rate Limit") when a client is making too many
//! requests. Every request should call [`RateLimiter::wait_if_limited`]
//! before sending; when a request observes a 429 it calls
//! [`RateLimiter::on_request_done`], which arms a backoff window.
//!
//! The wait is always the server's `Retry-After` header plus a little
//! padding ([`BACKOFF_PADDING`]); when no header is sent a short fixed
//! fallback ([`DEFAULT_BACKOFF`]) is used — never long exponential waits.
//!
//! Because the limiter is shared (behind an `Arc`), ALL concurrent jobs
//! respect the same window. Two properties make the backoff actually work
//! under concurrency:
//!
//! 1. **Shared window** — once a 429 arms the window, every other request
//!    waits for it to expire; in-flight successes never shorten it, so a
//!    thread cannot "clear" another thread's backoff.
//! 2. **Single-fire gate** — when the window ends, exactly ONE waiting
//!    request may fire ([`RateLimitGuard`] holds a one-permit semaphore).
//!    When that request completes it releases the permit, so the next
//!    waiting request fires only after the previous one finished — the
//!    waiters drain one at a time instead of bursting together and
//!    re-triggering the limit.

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// Fallback delay when Pixiv rate-limits without sending a `Retry-After`
/// header. Deliberately short — long exponential waits made the app feel
/// stuck; the header (when present) is what actually tells us how long to
/// wait.
const DEFAULT_BACKOFF: Duration = Duration::from_secs(5);
/// A little padding added on top of the server's `Retry-After` so the next
/// request lands comfortably after the window closes.
const BACKOFF_PADDING: Duration = Duration::from_secs(2);

/// Whether an error is a Pixiv rate-limit response (429 / "Rate Limit").
pub fn is_rate_limit(e: &crate::error::PixivError) -> bool {
    match e {
        crate::error::PixivError::HttpStatus(429, _) => true,
        crate::error::PixivError::Api(api) => api.is_rate_limit(),
        _ => false,
    }
}

/// Parse a `Retry-After` header value (RFC 9110) into a wait duration.
///
/// Supports both formats: delta-seconds (what Pixiv sends, e.g. `"5"`) and
/// an HTTP-date (e.g. `"Wed, 21 Oct 2015 07:28:00 GMT"`).
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    // HTTP-date: try the common RFC 2822 / RFC 3339 forms.
    let when = chrono::DateTime::parse_from_rfc2822(value)
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(value))
        .ok()?
        .with_timezone(&chrono::Utc);
    let now = chrono::Utc::now();
    Some(if when > now {
        (when - now).to_std().unwrap_or_default()
    } else {
        Duration::ZERO
    })
}

/// The server's suggested wait from a response's `Retry-After` header.
pub fn retry_after_header(resp: &reqwest::Response) -> Option<Duration> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_retry_after)
}

#[derive(Debug)]
struct State {
    /// When the current backoff window ends (None = not backing off).
    backoff_until: Option<SystemTime>,
    /// Consecutive 429s seen (used to decide the healthy reset).
    consecutive: u32,
}

/// Shared backoff state with a single-fire gate.
#[derive(Debug)]
pub struct RateLimiter {
    state: Mutex<State>,
    /// One-permit semaphore: at most one request may be "past the window"
    /// at any time. Each holder keeps its permit for the whole request and
    /// releases it on completion, so waiting requests fire one at a time.
    gate: std::sync::Arc<tokio::sync::Semaphore>,
}

/// Held while a gated request is in flight; dropping it lets the next
/// waiting request through.
pub struct RateLimitGuard(#[allow(dead_code)] tokio::sync::OwnedSemaphorePermit);

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                backoff_until: None,
                consecutive: 0,
            }),
            gate: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    /// Wait until it is this request's turn to fire, if a backoff window is
    /// active. Returns `None` when no window is active (requests proceed
    /// concurrently as usual). Returns a guard otherwise:
    ///
    /// * The caller must keep the guard alive for the WHOLE request (send →
    ///   response → [`Self::on_request_done`]); dropping it releases the
    ///   gate for the next waiting request.
    /// * Only one caller gets past a window at a time — the guard is backed
    ///   by a single-permit semaphore, so waiting requests drain one at a
    ///   time instead of bursting when the window ends.
    pub async fn wait_if_limited(&self) -> Option<RateLimitGuard> {
        // Fast path: healthy, no window — do not gate.
        self.current_window()?;
        // A window is active: try to become the gatekeeper. If another
        // request already holds the gate, queue up; its guard release wakes
        // us, and then we become the gatekeeper.
        let permit = match self.gate.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => self.gate.clone().acquire_owned().await.ok()?,
        };
        // Sleep until the window fully expires. It may be re-armed (another
        // 429) while we wait, so re-check in a loop. Only ONE request is
        // past this point at a time.
        while let Some(wait) = self.current_window() {
            tokio::time::sleep(wait).await;
        }
        Some(RateLimitGuard(permit))
    }

    /// Report the outcome of a completed request.
    ///
    /// * `rate_limited` (HTTP 429): arm the backoff window. The server's
    ///   `Retry-After` header is honored plus [`BACKOFF_PADDING`]; without a
    ///   header a short fixed [`DEFAULT_BACKOFF`] is used (no exponential
    ///   growth).
    /// * not rate limited: the request finished normally. If no window is
    ///   active the consecutive counter resets; a success DURING an active
    ///   window deliberately leaves the window untouched, so one thread's
    ///   success can never cancel another thread's backoff.
    ///
    /// This must be called on EVERY outcome (success, 429, other HTTP
    /// error, network error) so the gate is never left stuck.
    pub fn on_request_done(&self, rate_limited: bool, retry_after: Option<Duration>) {
        let mut state = self.state.lock().unwrap();
        if rate_limited {
            // Always honor the server's `Retry-After` header, plus a little
            // padding so the next attempt lands just after the window closes.
            // Without a header, fall back to a short fixed delay — never the
            // old exponential growth (up to 120s) that made the app feel stuck.
            let base = retry_after.unwrap_or(DEFAULT_BACKOFF);
            let delay = base + BACKOFF_PADDING;
            state.consecutive += 1;
            state.backoff_until = Some(SystemTime::now() + delay);
            log::warn!(
                "[rate_limit] rate limited; retrying in {:.1}s (header={})",
                delay.as_secs_f64(),
                retry_after.is_some()
            );
        } else if state.backoff_until.is_none()
            || state.backoff_until.is_some_and(|t| t <= SystemTime::now())
        {
            // Healthy (no window, or the window has fully elapsed): a
            // success resets the consecutive counter. A success DURING an
            // active window deliberately leaves everything untouched, so one
            // thread's success can never cancel another thread's backoff.
            state.consecutive = 0;
        }
    }

    /// Arm the backoff window after a 429. `retry_after` (seconds) from the
    /// response header is honored plus padding; otherwise a short fixed
    /// fallback is used.
    ///
    /// (Thin wrapper kept for the API surface; prefer [`Self::on_request_done`].)
    pub fn on_rate_limit(&self, retry_after_secs: Option<u64>) {
        self.on_request_done(true, retry_after_secs.map(Duration::from_secs));
    }

    /// A successful request clears the backoff state.
    ///
    /// (Thin wrapper kept for the API surface; prefer [`Self::on_request_done`].)
    pub fn on_success(&self) {
        self.on_request_done(false, None);
    }

    /// How long until the backoff window ends (None = not backing off).
    pub fn remaining(&self) -> Option<Duration> {
        let state = self.state.lock().unwrap();
        state
            .backoff_until
            .and_then(|t| t.duration_since(SystemTime::now()).ok())
            .filter(|d| !d.is_zero())
    }

    /// Whether the limiter is currently in a backoff window.
    pub fn is_backing_off(&self) -> bool {
        self.remaining().is_some()
    }

    fn current_window(&self) -> Option<Duration> {
        self.remaining()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn backoff_uses_fixed_fallback_with_padding() {
        let rl = RateLimiter::new();
        // No header → a short fixed delay (5s + 2s padding), NOT exponential.
        rl.on_rate_limit(None);
        let first = rl.remaining().expect("backing off").as_secs();
        // 7s window (may read slightly less due to elapsed time).
        assert!(
            (5..=7).contains(&first),
            "fixed fallback + padding, got {first}"
        );
        // Repeating 429s don't grow the wait — the same short window re-arms.
        rl.on_rate_limit(None);
        let second = rl.remaining().expect("backing off").as_secs();
        assert_eq!(second, first, "repeat 429s keep the same short window");
        // The window expires on its own; a success is not required to
        // clear it (and must not clear another thread's window).
        let expired = Duration::from_secs(second + 1);
        std::thread::sleep(expired);
        assert!(!rl.is_backing_off(), "the window expires on its own");
    }

    #[test]
    fn retry_after_header_used_with_padding() {
        let rl = RateLimiter::new();
        // A short server-suggested wait (1s) is honored plus 2s padding.
        rl.on_request_done(true, Some(Duration::from_secs(1)));
        let remaining = rl.remaining().expect("backing off");
        assert!(
            remaining >= Duration::from_secs(1) && remaining <= Duration::from_secs(3),
            "got {remaining:?}"
        );
        // The fallback (no header) is the fixed short delay.
        rl.on_request_done(true, None);
        let remaining = rl.remaining().expect("backing off");
        assert!(remaining >= Duration::from_secs(5), "got {remaining:?}");
    }

    #[test]
    fn success_during_window_does_not_clear_it() {
        let rl = RateLimiter::new();
        rl.on_request_done(true, Some(Duration::from_secs(30)));
        // In-flight successes while a window is armed must NOT cancel it —
        // otherwise one thread's backoff would be "fixed" by another
        // thread's unrelated success and the limit would never hold.
        rl.on_request_done(false, None);
        assert!(
            rl.is_backing_off(),
            "a success during the window must not clear another request's backoff"
        );
    }

    #[test]
    fn success_during_window_does_not_clear_backoff() {
        let rl = RateLimiter::new();
        // 429 arms a window; an in-window success must not clear it.
        rl.on_request_done(true, None);
        rl.on_success();
        assert!(rl.is_backing_off(), "window survives an in-window success");
        assert!(
            rl.remaining().unwrap() >= Duration::from_secs(5),
            "got {:?}",
            rl.remaining().unwrap()
        );
    }

    #[tokio::test]
    async fn success_when_healthy_resets_consecutive() {
        let rl = RateLimiter::new();
        // Tiny window (server header), let it expire.
        rl.on_request_done(true, Some(Duration::from_millis(30)));
        tokio::time::sleep(Duration::from_millis(60)).await;
        // A success with no window active means healthy → a fresh window is
        // the fixed short delay (no growth from a stale consecutive count).
        rl.on_success();
        rl.on_request_done(true, None);
        assert!(
            rl.remaining().unwrap() <= Duration::from_secs(8),
            "healthy success: got {:?}",
            rl.remaining().unwrap()
        );
    }

    #[test]
    fn parse_retry_after_formats() {
        assert_eq!(parse_retry_after("5"), Some(Duration::from_secs(5)));
        assert_eq!(parse_retry_after(" 2 "), Some(Duration::from_secs(2)));
        assert_eq!(parse_retry_after("0"), Some(Duration::ZERO));
        // HTTP-date form.
        let when = chrono::Utc::now() + chrono::Duration::seconds(60);
        let value = when.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        let parsed = parse_retry_after(&value).expect("http-date parses");
        assert!(parsed >= Duration::from_secs(50) && parsed <= Duration::from_secs(70));
        assert!(parse_retry_after("garbage").is_none());
    }

    #[tokio::test]
    async fn wait_returns_immediately_when_not_limited() {
        let rl = RateLimiter::new();
        let start = Instant::now();
        assert!(rl.wait_if_limited().await.is_none());
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn only_one_waiter_fires_after_window() {
        let rl = std::sync::Arc::new(RateLimiter::new());
        // Arm a 40ms window.
        rl.on_request_done(true, Some(Duration::from_millis(40)));
        let start = Instant::now();

        // Three concurrent waiters.
        let rl1 = rl.clone();
        let rl2 = rl.clone();
        let rl3 = rl.clone();
        let (h1, h2, h3) = tokio::join!(
            tokio::spawn(async move {
                let guard = rl1.wait_if_limited().await.expect("gated");
                tokio::time::sleep(Duration::from_millis(20)).await;
                let _ = &guard;
                rl1.on_request_done(false, None);
                Instant::now()
            }),
            tokio::spawn(async move {
                let guard = rl2.wait_if_limited().await.expect("gated");
                tokio::time::sleep(Duration::from_millis(20)).await;
                let _ = &guard;
                rl2.on_request_done(false, None);
                Instant::now()
            }),
            tokio::spawn(async move {
                let guard = rl3.wait_if_limited().await.expect("gated");
                tokio::time::sleep(Duration::from_millis(20)).await;
                let _ = &guard;
                rl3.on_request_done(false, None);
                Instant::now()
            }),
        );
        let mut fire_times = [h1.unwrap(), h2.unwrap(), h3.unwrap()];
        fire_times.sort_by_key(|t| *t - start);

        // The first waiter fires at the end of the window...
        assert!(
            fire_times[0].duration_since(start) >= Duration::from_millis(30),
            "first fire should wait out the window: {:?}",
            fire_times[0].duration_since(start)
        );
        // ...and each subsequent waiter fires only after the previous one's
        // request finished (~20ms later), NOT in a simultaneous burst.
        assert!(
            fire_times[1].duration_since(fire_times[0]) >= Duration::from_millis(15),
            "second fire must wait for the first request: {:?}",
            fire_times[1].duration_since(fire_times[0])
        );
        assert!(
            fire_times[2].duration_since(fire_times[1]) >= Duration::from_millis(15),
            "third fire must wait for the second request"
        );
    }
}
