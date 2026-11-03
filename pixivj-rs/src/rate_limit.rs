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
//! Windows are measured on the monotonic clock ([`std::time::Instant`]):
//! once armed, only the window itself expiring (or a newer 429 re-arming it)
//! changes the wait. Wall-clock jumps (NTP corrections, the user changing the
//! system time, suspend/resume) can neither extend nor cancel it. The wall
//! clock is read exactly once, in [`parse_retry_after`], to convert an
//! HTTP-date header into a `Duration`.
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
use std::time::{Duration, Instant};

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
/// an HTTP-date (e.g. `"Wed, 21 Oct 2015 07:28:00 GMT"`). This is the only
/// place where the wall clock is consulted: an HTTP-date is interpreted
/// against "now" once and immediately converted to a `Duration`, so later
/// clock adjustments cannot affect the armed window.
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
    ///
    /// Monotonic: `Instant` cannot jump backwards or forwards when the
    /// system wall clock changes, so an armed window always means exactly
    /// the delay that was granted.
    backoff_until: Option<Instant>,
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
        // One monotonic reading drives both branches below, so the armed
        // window and the "has it expired?" test cannot disagree.
        let now = Instant::now();
        if rate_limited {
            // Always honor the server's `Retry-After` header, plus a little
            // padding so the next attempt lands just after the window closes.
            // Without a header, fall back to a short fixed delay — never the
            // old exponential growth (up to 120s) that made the app feel stuck.
            let base = retry_after.unwrap_or(DEFAULT_BACKOFF);
            let delay = base + BACKOFF_PADDING;
            state.consecutive += 1;
            state.backoff_until = Some(now + delay);
            log::warn!(
                "[rate_limit] rate limited; retrying in {:.1}s (header={})",
                delay.as_secs_f64(),
                retry_after.is_some()
            );
        } else if state.backoff_until.map_or(true, |deadline| deadline <= now) {
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
    ///
    /// Measured against the monotonic clock, so a wall-clock adjustment can
    /// neither lengthen nor be mistaken for the end of an armed window.
    pub fn remaining(&self) -> Option<Duration> {
        let state = self.state.lock().unwrap();
        let now = Instant::now();
        state
            .backoff_until
            .map(|deadline| deadline.saturating_duration_since(now))
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

    /// Tolerance for "the clock kept running while we did something else".
    const SLOP: Duration = Duration::from_millis(250);
    /// Upper bound for the fixed fallback window.
    const FALLBACK_WINDOW: Duration =
        Duration::from_secs(DEFAULT_BACKOFF.as_secs() + BACKOFF_PADDING.as_secs());

    #[track_caller]
    fn assert_within(actual: Duration, min: Duration, max: Duration, what: &str) {
        assert!(
            (min..=max).contains(&actual),
            "{what}: expected {min:?}..={max:?}, got {actual:?}"
        );
    }

    /// Arm a short window straight into the shared state.
    ///
    /// Every window armed through the public 429 path is at least
    /// [`BACKOFF_PADDING`] (2s) long, which is too much to sleep through in a
    /// unit test. Expiry tests therefore shorten an already-armed window; the
    /// padding/fallback arithmetic itself is covered by the public-path tests,
    /// which never sleep on it.
    fn arm_short_window(rl: &RateLimiter, after: Duration) {
        rl.state.lock().unwrap().backoff_until = Some(Instant::now() + after);
    }

    #[test]
    fn backoff_uses_fixed_fallback_with_padding() {
        let rl = RateLimiter::new();
        // No header → the fixed short delay (5s + 2s padding), never
        // exponential growth. Four consecutive 429s must ALL re-arm the same
        // short window: a growing window would show 7s, 9s, 11s, 13s…
        for attempt in 1..=4 {
            rl.on_rate_limit(None);
            let remaining = rl.remaining().expect("backing off after a 429");
            assert_within(
                remaining,
                FALLBACK_WINDOW - Duration::from_millis(500),
                FALLBACK_WINDOW,
                &format!("429 #{attempt} must use the fixed fallback + padding"),
            );
        }
        assert!(rl.is_backing_off(), "still armed after repeated 429s");
    }

    #[test]
    fn retry_after_header_is_honoured_with_padding() {
        let rl = RateLimiter::new();
        // A short server-suggested wait (1s) is honored plus 2s padding.
        rl.on_request_done(true, Some(Duration::from_secs(1)));
        assert_within(
            rl.remaining().expect("backing off"),
            Duration::from_secs(2),
            Duration::from_secs(3),
            "1s header + 2s padding",
        );

        // A longer header replaces the window instead of accumulating on it.
        rl.on_request_done(true, Some(Duration::from_secs(30)));
        assert_within(
            rl.remaining().expect("backing off"),
            Duration::from_secs(31) + Duration::from_millis(500),
            Duration::from_secs(32),
            "30s header + 2s padding",
        );

        // A later, shorter header re-arms down to that header (not 30s + 2s
        // on top), so the wait always reflects the latest server hint.
        rl.on_request_done(true, Some(Duration::from_secs(1)));
        assert_within(
            rl.remaining().expect("backing off"),
            Duration::from_secs(2),
            Duration::from_secs(3),
            "newer 1s header must replace the older 30s window",
        );
    }

    #[test]
    fn success_during_window_does_not_shorten_it() {
        let rl = RateLimiter::new();
        rl.on_request_done(true, Some(Duration::from_secs(30)));
        let before = rl.remaining().expect("armed");
        // In-flight successes while a window is armed must NOT cancel or
        // shorten it — otherwise one thread's backoff would be "fixed" by
        // another thread's unrelated success and the limit would never hold.
        rl.on_request_done(false, None);
        let after = rl
            .remaining()
            .expect("window survives an in-window success");
        assert!(rl.is_backing_off(), "a success must not clear the window");
        assert!(
            after + SLOP >= before,
            "in-window success shortened the window: before {before:?}, after {after:?}"
        );
    }

    #[test]
    fn healthy_success_resets_before_next_429() {
        let rl = RateLimiter::new();
        // A long server window, then collapsed to a short deadline so the
        // test does not have to wait out the server's 30s.
        rl.on_request_done(true, Some(Duration::from_secs(30)));
        arm_short_window(&rl, Duration::from_millis(20));

        // Wait for the shortened window to lapse (~2x its length).
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            !rl.is_backing_off(),
            "the shortened window should have lapsed already"
        );

        // A success while healthy resets state; the next headerless 429 must
        // use the fixed fallback again (not a stale, longer window).
        rl.on_success();
        rl.on_rate_limit(None);
        assert_within(
            rl.remaining().expect("backing off"),
            FALLBACK_WINDOW - Duration::from_millis(500),
            FALLBACK_WINDOW,
            "healthy success -> next 429 uses the fixed fallback",
        );
    }

    #[test]
    fn window_expires_on_its_own() {
        let rl = RateLimiter::new();
        // The public path arms a padded window (7s here); collapse it to a
        // short deadline so the test only sleeps milliseconds.
        rl.on_rate_limit(None);
        assert!(rl.is_backing_off(), "should be armed");
        arm_short_window(&rl, Duration::from_millis(40));
        assert!(rl.is_backing_off(), "short window is armed");

        // Sleep ~2x the armed window: no success is needed to clear it.
        std::thread::sleep(Duration::from_millis(90));
        assert!(
            !rl.is_backing_off(),
            "the window must expire without a success"
        );
        assert!(rl.remaining().is_none(), "no remaining time once expired");
    }

    #[tokio::test]
    async fn wait_returns_immediately_when_healthy() {
        let rl = RateLimiter::new();
        let start = Instant::now();
        assert!(rl.wait_if_limited().await.is_none());
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "healthy path must not sleep, took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn only_one_waiter_fires_after_window() {
        let rl = std::sync::Arc::new(RateLimiter::new());
        // Arm a short window directly: this test is about the gate draining
        // one waiter at a time, not about the padding arithmetic.
        arm_short_window(&rl, Duration::from_millis(40));
        let start = Instant::now();

        // Each waiter holds its guard for the whole "request" and records
        // when the gate actually let it through.
        async fn gated_request(rl: std::sync::Arc<RateLimiter>) -> Instant {
            let guard = rl.wait_if_limited().await.expect("gated");
            let fired = Instant::now();
            tokio::time::sleep(Duration::from_millis(20)).await;
            rl.on_request_done(false, None);
            drop(guard);
            fired
        }

        let (h1, h2, h3) = tokio::join!(
            tokio::spawn(gated_request(rl.clone())),
            tokio::spawn(gated_request(rl.clone())),
            tokio::spawn(gated_request(rl.clone())),
        );
        let mut fire_times = [h1.unwrap(), h2.unwrap(), h3.unwrap()];
        fire_times.sort_by_key(|t| *t - start);

        // The first waiter fires only after the window ended...
        assert!(
            fire_times[0].duration_since(start) >= Duration::from_millis(30),
            "first fire should wait out the window: {:?}",
            fire_times[0].duration_since(start)
        );
        assert!(
            fire_times[0].duration_since(start) <= Duration::from_secs(2),
            "first fire should not wait far past the window: {:?}",
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
            "third fire must wait for the second request: {:?}",
            fire_times[2].duration_since(fire_times[1])
        );
    }

    #[test]
    fn parse_retry_after_formats() {
        // Delta-seconds, with and without surrounding whitespace.
        assert_eq!(parse_retry_after("5"), Some(Duration::from_secs(5)));
        assert_eq!(parse_retry_after(" 2 "), Some(Duration::from_secs(2)));
        assert_eq!(parse_retry_after("0"), Some(Duration::ZERO));
        // HTTP-date (RFC 2822 form, which is what servers emit).
        let when = chrono::Utc::now() + chrono::Duration::seconds(60);
        let value = when.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        let parsed = parse_retry_after(&value).expect("http-date parses");
        assert!(
            (Duration::from_secs(50)..=Duration::from_secs(70)).contains(&parsed),
            "http-date should be ~60s ahead, got {parsed:?}"
        );
        // HTTP-date (RFC 3339 form).
        let value = when.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let parsed = parse_retry_after(&value).expect("rfc3339 date parses");
        assert!(
            (Duration::from_secs(50)..=Duration::from_secs(70)).contains(&parsed),
            "rfc3339 date should be ~60s ahead, got {parsed:?}"
        );
        // A date in the past means "retry now", not a negative wait.
        let past = (chrono::Utc::now() - chrono::Duration::seconds(60))
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        assert_eq!(parse_retry_after(&past), Some(Duration::ZERO));
        assert!(parse_retry_after("garbage").is_none());
    }
}
