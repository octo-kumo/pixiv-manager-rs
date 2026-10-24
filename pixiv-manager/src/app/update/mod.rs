use crate::app::message::Message;
use crate::app::state::{AppPhase, AppState, JobsState};
use crate::image::cache::ImageCache;
use crate::io::config::{cache_dir, fix_config_permissions, load_config};
use crate::io::library::BookmarkLibrary;
use onboarding::{build_client_async, start_onboarding};
use pixivj_rs::error::PixivError;
use pixivj_rs::model::*;
use std::sync::Arc;

/// Minimum number of tiles a filtered grid must have before we stop
/// auto-loading more pages after a filter/search change.
pub(super) const AUTO_LOAD_MIN_TILES: usize = 20;
/// Maximum number of consecutive auto-loads triggered by a single
/// filter/search change (prevents infinite loading).
pub(super) const AUTO_LOAD_MAX_PAGES: u32 = 3;

// ── Boot ───────────────────────────────────────────────────────────────

/// Application boot.
///
/// * No saved token → start onboarding immediately.
/// * Saved token → enter the main gallery directly. The token is validated
///   lazily by the API client on the first request; if it's rejected, the
///   first fetch fails and we fall back to onboarding.
pub fn boot() -> (AppState, iced::Task<Message>) {
    log::info!("[boot] === boot() called ===");
    // Default to dark until the OS reports its actual dark/light preference.
    let mut state = AppState {
        theme_mode: iced::theme::Mode::Dark,
        ..AppState::default()
    };
    // The image caches' RAM budgets follow the memory preset.
    let budget = crate::image::cache::DEFAULT_MEMORY_BUDGET
        * state.settings.memory_preset.cache_budget_multiplier();
    state.images.image_cache = Arc::new(ImageCache::new(cache_dir(), budget));
    state.images.thumbnail_cache = Arc::new(ImageCache::new(
        cache_dir().join("thumb"),
        budget.max(crate::image::cache::DEFAULT_MEMORY_BUDGET),
    ));
    state.jobs = JobsState {
        library: BookmarkLibrary::load(&crate::io::library::bookmarks_library_path()),
        library_path: crate::io::library::bookmarks_library_path(),
        download_queue_path: crate::io::library::download_queue_path(),
        ..Default::default()
    };

    // Query the OS theme so the app matches the system dark/light preference.
    let system_theme = iced::system::theme().map(Message::SystemThemeFetched);
    let mut boot_tasks = vec![system_theme];

    // Load a system emoji font so emoji in comments render (best-effort).
    boot_tasks.push(emoji_font_task());

    // Restrict any pre-existing config to owner-only (it holds the token).
    fix_config_permissions();

    state.view_transforms = crate::viewer_tools::load();

    // Load the centralized color map (palettes for the sRGB/Hue sorts).
    state.images.palettes = crate::io::colors::load_colors();
    log::info!(
        "[boot] Loaded {} cached color palettes",
        state.images.palettes.len()
    );

    // Try to load saved config
    if let Some(config) = load_config() {
        log::info!("[boot] Config loaded from disk");
        state.settings = config;
    } else {
        log::info!("[boot] No config file found");
    }
    // Apply the configured memory preset to the freshly loaded settings
    // (the caches were sized from the default preset above).
    let budget = crate::image::cache::DEFAULT_MEMORY_BUDGET
        * state.settings.memory_preset.cache_budget_multiplier();
    state.images.image_cache = Arc::new(ImageCache::new(cache_dir(), budget));
    state.images.thumbnail_cache = Arc::new(ImageCache::new(
        cache_dir().join("thumb"),
        budget.max(crate::image::cache::DEFAULT_MEMORY_BUDGET),
    ));
    // The "hide bookmarked" option seeds every tab's Bookmarked tristate.
    gallery::apply_bookmark_filter_defaults(&mut state);
    // Apply the configured animation frame-rate limit.
    crate::frame::set_limit(state.settings.fps_limit);
    // Wire the configured proxy into the shared image-CDN client (stored in
    // state so tests can substitute it).
    state.images.fetcher.http = Arc::new(build_image_http_client(state.settings.proxy.as_deref()));
    // Remember what the running clients were built with, so later proxy
    // edits can be hot-applied.
    state.applied_proxy = state.settings.proxy.clone();

    let Some(token) = state.settings.refresh_token.clone() else {
        log::info!("[boot] No refresh token in config → starting onboarding");
        let task = start_onboarding(&mut state);
        boot_tasks.push(task);
        return (state, iced::Task::batch(boot_tasks));
    };

    // ── Token exists → go straight to the main gallery ──
    log::info!(
        "[boot] Token found (len={}) → entering main app",
        token.len()
    );
    state.phase = AppPhase::Main;
    // Rescan the gallery dir for downloaded illusts (drives the Missing
    // filter + tile checkmarks).
    boot_tasks.push(iced::Task::done(Message::RefreshDownloaded));
    boot_tasks.push(iced::Task::perform(
        build_client_async(token, state.settings.proxy.clone()),
        Message::ClientBuilt,
    ));
    log::info!("[boot] Returning state with client build task");
    (state, iced::Task::batch(boot_tasks))
}

/// Load a system emoji font (if present) so emoji render instead of boxes.
fn emoji_font_task() -> iced::Task<Message> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/google-noto-color-emoji-fonts/Noto-COLRv1.ttf",
        "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
        "/usr/share/fonts/opentype/noto/NotoColorEmoji.ttf",
        "/usr/share/fonts/noto/NotoColorEmoji.ttf",
    ];
    for path in CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            log::info!("[boot] Loading emoji font from {}", path);
            return iced::font::load(bytes).map(|_| Message::Noop);
        }
    }
    log::info!("[boot] No system emoji font found");
    iced::Task::none()
}

pub(super) fn handle_api_error(
    state: &mut AppState,
    e: &PixivError,
) -> Option<iced::Task<Message>> {
    if matches!(e, PixivError::Auth(_)) {
        log::error!(
            "[update] Auth error ({}); falling back to onboarding — saved token left intact",
            e
        );
        // Clear the in-memory token only; do NOT write it back to disk.
        state.settings.refresh_token = None;
        return Some(start_onboarding(state));
    }
    None
}

// ── Shared image-CDN infrastructure ─────────────────────────────────────

/// The largest usable thumbnail dimension: Pixiv's `medium` image is a
/// 540×540 crop, so there's no point generating sharper than that.
const THUMB_SOURCE_MAX: u32 = 540;
/// The `large` image's longest side (1200px) — used when the tile is big
/// enough that a 540px thumbnail would be upscaled.
const THUMB_LARGE_MAX: u32 = 1200;

/// The pixel size thumbnails should be generated at for a sharp display.
///
/// The parallax tile draws the image at `cover × (1 + PARALLAX_EXTRA)` where
/// `cover` is the larger of `tile_w/tile_h` relative to the source — in
/// LOGICAL pixels. On a hi-DPI display that's `× scale_factor` physical
/// pixels, so the thumbnail must be generated at the physical size or it's
/// upscaled (blurry). The **high-res** option multiplies the target further
/// and prefers the 1200px `large` source. Target = `max(tile_w, tile_h) ×
/// factor × scale`, capped at the 1200px source.
pub fn thumbnail_target(state: &AppState) -> u32 {
    let width = if state.window_size.0 > 0.0 {
        state.window_size.0
    } else {
        1600.0
    };
    let grid = state.settings.grid_size.max(1) as f32;
    let cc = ((width / grid).floor() as u32).clamp(1, 12).max(1) as usize;
    let tile_w = width / cc as f32;
    let longest = tile_w.max(grid);
    let scale = state.scale_factor.max(1.0);
    let factor = if state.settings.high_res_thumbnails {
        2.5
    } else {
        1.0 + crate::widgets::parallax_image::PARALLAX_EXTRA
    };
    ((longest * factor * scale).ceil() as u32).clamp(1, THUMB_LARGE_MAX)
}

/// The source URL to fetch for a thumbnail at `target` px: the lightweight
/// 540px `medium` when that's enough, the 1200px `large` when the tile is
/// big enough to need it (intelligent source selection). The high-res option
/// always prefers `large`.
pub fn thumbnail_source_url(illust: &Illustration, target: u32, high_res: bool) -> Option<String> {
    let urls = illust.image_urls.as_ref()?;
    if high_res || target > THUMB_SOURCE_MAX {
        urls.large.clone().or_else(|| urls.medium.clone())
    } else {
        urls.medium.clone().or_else(|| urls.large.clone())
    }
}

/// Build the connection-pooled image-CDN client (with keep-alive), applying
/// the configured proxy. Called once at boot; stored in [`ImageState`].
pub(super) fn build_image_http_client(proxy: Option<&str>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        // Reuse keep-alive connections instead of re-handshaking per file.
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(16);
    if let Some(proxy) = proxy {
        if let Ok(proxy) = reqwest::Proxy::all(proxy) {
            builder = builder.proxy(proxy);
        }
    }
    builder.build().expect("failed to build shared HTTP client")
}

/// Build the image-CDN GET for a provider. The headers come from the
/// provider itself ([`crate::media::Provider::image_headers`]): Pixiv's CDN
/// requires the app's Referer/User-Agent, twimg rejects some non-browser
/// agents (and must not receive a foreign Referer).
pub(super) fn image_request(
    fetcher: &crate::app::state::ImageFetcher,
    url: &str,
    provider: crate::media::Provider,
) -> reqwest::RequestBuilder {
    let (user_agent, referer) = provider.image_headers();
    let req = fetcher.http.get(url).header("User-Agent", user_agent);
    match referer {
        Some(referer) => req.header("Referer", referer),
        None => req,
    }
}

/// Average RGB of a (small) PNG thumbnail, sampled on a coarse grid. Used by
/// the gallery's "invert" text style; runs on a blocking thread.
fn average_rgb(png: &[u8]) -> [u8; 3] {
    let Ok(image) = image::load_from_memory(png) else {
        return [128, 128, 128];
    };
    let rgb = image.to_rgb8();
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let step = (w.max(h) / 32).max(1);
    let mut sum = [0u64; 3];
    let mut n = 0u64;
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let p = rgb.get_pixel(x as u32, y as u32).0;
            sum[0] += p[0] as u64;
            sum[1] += p[1] as u64;
            sum[2] += p[2] as u64;
            n += 1;
            x += step;
        }
        y += step;
    }
    if n == 0 {
        return [128, 128, 128];
    }
    [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]
}

/// Record a thumbnail's average color (once per illust per session) for the
/// "invert" tile-text style. Cheap no-op when already known.
async fn record_thumbnail_avg(fetcher: &crate::app::state::ImageFetcher, id: u64, png: &[u8]) {
    if fetcher
        .thumbnail_avg
        .lock()
        .map(|m| m.contains_key(&id))
        .unwrap_or(false)
    {
        return;
    }
    let png = png.to_vec();
    let avg = tokio::task::spawn_blocking(move || average_rgb(&png))
        .await
        .ok();
    if let Some(avg) = avg {
        if let Ok(mut map) = fetcher.thumbnail_avg.lock() {
            map.insert(id, avg);
        }
    }
}

/// Fetch a thumbnail image from a URL, resize it to a small thumbnail using
/// fast_image_resize (SIMD-accelerated, very low CPU), and return as PNG bytes.
///
/// Checks the two-layer [`ImageCache`] first; only hits the network on a miss.
///
/// Pixiv's image CDN (i.pximg.net) rejects requests without the `Referer`
/// header, so it must be set — otherwise every fetch returns HTTP 403.
#[allow(clippy::too_many_arguments)]
async fn fetch_thumbnail(
    id: u64,
    url: String,
    target: u32,
    raw_cache: Arc<ImageCache>,
    thumb_cache: Arc<ImageCache>,
    fetcher: crate::app::state::ImageFetcher,
    provider: crate::media::Provider,
) -> Result<Vec<u8>, PixivError> {
    crate::profiling::timed_task("thumbnail_fetch", async move {
        // The PROCESSED thumbnail cache (`url#target`) is invalidated when
        // the grid size or quality option changes. The RAW URL cache (`url` →
        // downloaded bytes) always survives, so a re-size reuses the
        // downloaded bytes instead of hitting the network.
        let thumb_key = format!("{url}#{target}");
        if let Some(bytes) = thumb_cache.get(&thumb_key) {
            log::debug!("[fetch_thumbnail] Processed cache hit: {}", url);
            fetcher
                .thumbnail_cache_hits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            record_thumbnail_avg(&fetcher, id, &bytes).await;
            return Ok(bytes);
        }

        // Raw URL bytes: from the persistent URL cache or the network.
        let raw = match raw_cache.get(&url) {
            Some(bytes) => bytes,
            None => {
                log::debug!("[fetch_thumbnail] Fetching: {}", url);
                fetcher
                    .thumbnail_attempts
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // Thumbnails use their own rate limiter so a full-image
                // download or pfp fetch backing off on a 429 can't stall the
                // thumbnail pool.
                let _gate = fetcher.thumbnail_limiter.wait_if_limited().await;
                #[cfg(feature = "profiling")]
                let start = std::time::Instant::now();
                let resp = match image_request(&fetcher, &url, provider).send().await {
                    Ok(resp) => {
                        fetcher.thumbnail_limiter.on_request_done(
                            resp.status().as_u16() == 429,
                            pixivj_rs::rate_limit::retry_after_header(&resp),
                        );
                        #[cfg(feature = "profiling")]
                        crate::profiling::tracing::record_http(
                            "GET",
                            url.clone(),
                            resp.status().as_u16(),
                            start.elapsed(),
                            resp.content_length().unwrap_or(0) as usize,
                        );
                        resp
                    }
                    Err(e) => {
                        fetcher.thumbnail_limiter.on_request_done(false, None);
                        #[cfg(feature = "profiling")]
                        crate::profiling::tracing::record_http(
                            "GET",
                            url.clone(),
                            0,
                            start.elapsed(),
                            0,
                        );
                        return Err(e.into());
                    }
                };
                if !resp.status().is_success() {
                    return Err(PixivError::HttpStatus(
                        resp.status().as_u16(),
                        "thumbnail fetch failed".into(),
                    ));
                }
                let bytes = resp.bytes().await?.to_vec();
                log::debug!(
                    "[fetch_thumbnail] Got {} bytes from {}, resizing...",
                    bytes.len(),
                    url
                );
                // Cache the RAW bytes under the plain URL — these survive
                // grid/quality changes forever.
                raw_cache.insert_async(&url, bytes.clone()).await;
                bytes
            }
        };

        // Resize to the target pixel size using fast_image_resize
        // (SIMD-accelerated, Lanczos3 by default — sharp but fast).
        let thumbnail = tokio::task::spawn_blocking(move || resize_to_thumbnail(&raw, target))
            .await
            .map_err(|e| PixivError::Internal(format!("join error: {e}")))??;

        // Persist the PROCESSED thumbnail under `url#target`.
        thumb_cache
            .insert_async(&thumb_key, thumbnail.clone())
            .await;
        record_thumbnail_avg(&fetcher, id, &thumbnail).await;
        Ok(thumbnail)
    })
    .await
}

/// Fetch a profile picture (cached) — the image CDN requires the Referer for
/// Pixiv and a browser agent for Twitter.
pub(super) async fn fetch_pfp(
    url: String,
    cache: Arc<ImageCache>,
    fetcher: crate::app::state::ImageFetcher,
    provider: crate::media::Provider,
) -> Result<Vec<u8>, PixivError> {
    crate::profiling::timed_task("pfp_fetch", async move {
        if let Some(bytes) = cache.get(&url) {
            return Ok(bytes);
        }
        let _gate = fetcher.limiter.wait_if_limited().await;
        #[cfg(feature = "profiling")]
        let start = std::time::Instant::now();
        let resp = match image_request(&fetcher, &url, provider).send().await {
            Ok(resp) => {
                fetcher.limiter.on_request_done(
                    resp.status().as_u16() == 429,
                    pixivj_rs::rate_limit::retry_after_header(&resp),
                );
                #[cfg(feature = "profiling")]
                crate::profiling::tracing::record_http(
                    "GET",
                    url.clone(),
                    resp.status().as_u16(),
                    start.elapsed(),
                    resp.content_length().unwrap_or(0) as usize,
                );
                resp
            }
            Err(e) => {
                fetcher.limiter.on_request_done(false, None);
                #[cfg(feature = "profiling")]
                crate::profiling::tracing::record_http("GET", url.clone(), 0, start.elapsed(), 0);
                return Err(e.into());
            }
        };
        if !resp.status().is_success() {
            return Err(PixivError::HttpStatus(
                resp.status().as_u16(),
                "pfp fetch failed".into(),
            ));
        }
        let bytes = resp.bytes().await?.to_vec();
        cache.insert_async(&url, bytes.clone()).await;
        Ok(bytes)
    })
    .await
}

/// Resize raw image bytes to a small thumbnail using fast_image_resize.
/// Returns PNG bytes suitable for iced::widget::image.
pub(super) fn resize_to_thumbnail(data: &[u8], max_size: u32) -> Result<Vec<u8>, PixivError> {
    use fast_image_resize as fr;
    use image::codecs::png::PngEncoder;
    use image::ImageEncoder;

    // Decode the source image
    let img = image::load_from_memory(data).map_err(|e| PixivError::Decode(e.to_string()))?;

    let (src_w, src_h) = (img.width(), img.height());
    log::debug!(
        "[resize_to_thumbnail] Source: {}x{}, target max: {}px",
        src_w,
        src_h,
        max_size
    );

    // Calculate target dimensions
    let (dst_w, dst_h) = if src_w >= src_h {
        let ratio = max_size as f64 / src_w as f64;
        (max_size, (src_h as f64 * ratio).round() as u32)
    } else {
        let ratio = max_size as f64 / src_h as f64;
        ((src_w as f64 * ratio).round() as u32, max_size)
    };
    let dst_w = dst_w.max(1);
    let dst_h = dst_h.max(1);

    // Convert to RGBA8 for fast_image_resize
    let rgba = img.to_rgba8();
    let src_image =
        fr::images::Image::from_vec_u8(src_w, src_h, rgba.into_raw(), fr::PixelType::U8x4)
            .map_err(|e| PixivError::Internal(format!("fir src: {e}")))?;

    // Create resizer and destination
    let mut dst_image = fr::images::Image::new(dst_w, dst_h, fr::PixelType::U8x4);
    let mut resizer = fr::Resizer::new();
    resizer
        .resize(&src_image, &mut dst_image, None)
        .map_err(|e| PixivError::Internal(format!("fir resize: {e}")))?;

    // Encode back to PNG
    let mut png_buf = Vec::with_capacity(dst_w as usize * dst_h as usize * 4 / 2);
    PngEncoder::new(&mut png_buf)
        .write_image(
            dst_image.buffer(),
            dst_w,
            dst_h,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| PixivError::Internal(format!("png encode: {e}")))?;

    log::debug!(
        "[resize_to_thumbnail] Resized to {}x{}, PNG {} bytes",
        dst_w,
        dst_h,
        png_buf.len()
    );
    Ok(png_buf)
}

/// How many thumbnail fetches may be in flight at once. Bounding concurrency
/// (instead of the old unbounded bursts of up to 48 per window report) keeps
/// the image CDN from being hammered into connection resets / 429s — the
/// root cause of thumbnails "dying" mid-load — while still streaming the
/// window in quickly.
const THUMBNAIL_CONCURRENCY: usize = 8;

/// How many times a failed thumbnail fetch is retried automatically before
/// it is dropped from the pool this session (the next scroll re-enqueues it).
pub(super) const THUMBNAIL_MAX_RETRIES: u32 = 5;

/// Lazy thumbnail loading via a **bounded worker pool**: scan the gallery's
/// visible window, enqueue every missing `(id, url)` into the shared pool,
/// then start as many workers as the concurrency budget allows.
///
/// The pool is what keeps a 6000-item bookmark tab responsive: at most
/// [`THUMBNAIL_CONCURRENCY`] fetches are ever in flight, failures are
/// retried a bounded number of times, and a single failed URL can never
/// stall the rest (workers refill from the queue on every completion).
/// How far (in viewport spans) beyond the visible window a queued or
/// in-flight thumbnail stays "wanted". Deliberately much wider than the
/// ±0.5-span enqueue padding: small scrolls and drifts must never abort a
/// started fetch (that would churn), only real jumps out of a region do.
const THUMBNAIL_CANCEL_PAD_SPANS: f32 = 2.0;

/// Padded visible index range `(i0, i1)` into the active tab's filtered
/// list, or `None` when there is nothing to show (empty content, or a
/// window that maps past it). `pad` extends the range by that fraction of
/// the viewport span on each side.
fn window_index_range(state: &AppState, pad: f32) -> Option<(usize, usize)> {
    let illusts = active_tab_illusts(state);
    let width = if state.window_size.0 > 0.0 {
        state.window_size.0
    } else {
        1600.0
    };
    let cc = ((width / state.settings.grid_size.max(1) as f32).floor() as u32)
        .clamp(1, 12)
        .max(1) as usize;
    let row_height = state.settings.grid_size.max(1) as f32;

    // When the gallery hasn't reported a window yet (no scroll happened),
    // assume the top of the content is visible.
    let (mut start, mut end) = state
        .gallery_window
        .unwrap_or((0.0, viewport_estimate(state)));

    // A stale window from a previous tab/list can point past the (new,
    // shorter) filtered content — e.g. after a filter change or tab switch
    // the gallery resets to the top but `gallery_window` still holds the old
    // scroll depth, so the index range below would be empty and NO thumbnails
    // would spawn until the user scrolls. Clamp: if the window is beyond the
    // content, fall back to the top.
    let content_h = (illusts.len() as f32 / cc.max(1) as f32) * row_height;
    if start >= content_h {
        start = 0.0;
        end = viewport_estimate(state);
    }

    let span = (end - start).max(1.0);
    start = (start - span * pad).max(0.0);
    end += span * pad;

    let i0 = ((start / row_height).floor() as usize).saturating_mul(cc);
    let i1 = (((end / row_height).ceil() as usize + 1).saturating_mul(cc)).min(illusts.len());
    (i0 < i1).then_some((i0, i1))
}

/// Abort queued + in-flight thumbnails that left the generously padded
/// visible window (or belong to content no longer shown at all, e.g. after
/// a tab switch or filter change): a fetch for a tile nobody sees must not
/// occupy one of the [`THUMBNAIL_CONCURRENCY`] slots nor stream bytes in
/// the background. Cancelled ids lose their `requested` marker, so
/// scrolling back simply re-enqueues them.
pub(super) fn cancel_out_of_view_thumbnails(state: &mut AppState) {
    let keep: std::collections::HashSet<u64> =
        window_index_range(state, THUMBNAIL_CANCEL_PAD_SPANS)
            .map(|(i0, i1)| {
                active_tab_illusts(state)[i0..i1]
                    .iter()
                    .map(|i| i.id)
                    .collect()
            })
            .unwrap_or_default();

    let images = &mut state.images;
    images
        .thumbnail_queue
        .retain(|(id, _, _)| keep.contains(id));
    let stale: Vec<u64> = images
        .thumbnail_in_flight
        .iter()
        .copied()
        .filter(|id| !keep.contains(id))
        .collect();
    for id in stale {
        images.thumbnail_in_flight.remove(&id);
        images.thumbnail_requested.remove(&id);
        if let Some(handle) = images.thumbnail_aborts.remove(&id) {
            log::debug!("[thumbnails] cancelled out-of-view fetch for {}", id);
            handle.abort();
        }
    }
}

pub(super) fn spawn_thumbnails_for_window(state: &mut AppState) -> Vec<iced::Task<Message>> {
    // Drop fetches that left the view FIRST, so their slots refill with the
    // fresh window below in the same pass.
    cancel_out_of_view_thumbnails(state);

    let Some((i0, i1)) = window_index_range(state, 0.5) else {
        return Vec::new();
    };
    let illusts = active_tab_illusts(state);

    // Enqueue the window's not-yet-requested thumbnails. The `thumbnail_requested`
    // marker stays set once enqueued, so repeated window reports never add the
    // same id twice; a failed fetch is re-queued by the `ThumbnailLoaded`
    // handler instead (bounded retries), and after giving up it is dropped from
    // the pool and only returns on the next window report.
    let target = thumbnail_target(state);
    let mut to_enqueue: Vec<(u64, String, u32)> = Vec::new();
    {
        let images = &state.images;
        for illust in &illusts[i0..i1] {
            let id = illust.id;
            if images.thumbnail_handles.contains_key(&id)
                || images.thumbnail_requested.contains(&id)
                || images.thumbnail_in_flight.contains(&id)
            {
                continue;
            }
            let Some(url) =
                thumbnail_source_url(illust, target, state.settings.high_res_thumbnails)
            else {
                continue;
            };
            to_enqueue.push((id, url, target));
        }
    }
    for (id, url, target) in to_enqueue {
        state.images.thumbnail_queue.push((id, url, target));
        state.images.thumbnail_requested.insert(id);
        state.images.thumbnail_targets.insert(id, target);
    }
    spawn_thumbnail_workers(state)
}

/// Start thumbnail fetches from the pool queue, up to the concurrency budget
/// (items already in flight or already cached are skipped). Returns the
/// spawned fetch tasks; the caller batches them with whatever else it is
/// already doing.
pub(super) fn spawn_thumbnail_workers(state: &mut AppState) -> Vec<iced::Task<Message>> {
    let images = &mut state.images;
    let queue = std::mem::take(&mut images.thumbnail_queue);
    let mut tasks: Vec<iced::Task<Message>> = Vec::new();
    let mut kept: Vec<(u64, String, u32)> = Vec::new();
    // `started` tracks the TOTAL in-flight count (existing workers + new
    // ones), so refills never push the pool past the concurrency budget.
    let mut started = images.thumbnail_in_flight.len();
    for (id, url, target) in queue {
        if images.thumbnail_handles.contains_key(&id) || images.thumbnail_in_flight.contains(&id) {
            // Already cached or already being fetched — nothing to do.
            continue;
        }
        // Ids that exhausted their retries are dropped, not fetched again
        // (they can return via the next window report).
        if images.thumbnail_retries.get(&id).copied().unwrap_or(0) >= THUMBNAIL_MAX_RETRIES {
            continue;
        }
        if started >= THUMBNAIL_CONCURRENCY {
            kept.push((id, url, target));
            continue;
        }
        started += 1;
        images.thumbnail_in_flight.insert(id);
        let raw_cache = images.image_cache.clone();
        let thumb_cache = images.thumbnail_cache.clone();
        let fetcher = images.fetcher.clone();
        let url_for_msg = url.clone();
        let provider = crate::media::provider_of(id);
        // Abortable: scrolling the tile out of view cancels the fetch
        // mid-flight (see `cancel_out_of_view_thumbnails`).
        let (task, handle) = iced::Task::perform(
            fetch_thumbnail(id, url, target, raw_cache, thumb_cache, fetcher, provider),
            move |result| Message::ThumbnailLoaded(id, url_for_msg, target, result),
        )
        .abortable();
        images.thumbnail_aborts.insert(id, handle);
        tasks.push(task);
    }
    images.thumbnail_queue = kept;
    tasks
}

/// Extract an MMCQ palette from thumbnail bytes (decoded → quantized) on a
/// blocking thread, so palette building never stalls the UI thread.
pub(super) async fn extract_palette_result(bytes: Vec<u8>) -> Vec<[u8; 3]> {
    tokio::task::spawn_blocking(move || {
        let img = image::load_from_memory(&bytes).ok()?;
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        let palette = crate::image::mmcq::quantize(rgba.as_raw(), w, h);
        Some(
            palette
                .into_iter()
                .map(|(r, g, b)| [r, g, b])
                .collect::<Vec<_>>(),
        )
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

/// Extract a palette for one illust outside the "process all" pass (the
/// viewer does this for the illust it shows).
pub(super) async fn extract_palette(id: u64, bytes: Vec<u8>) -> Message {
    Message::PaletteExtracted(id, extract_palette_result(bytes).await)
}

/// Fetch + extract a palette for one illust (used by "process all").
async fn fetch_and_extract_palette(
    id: u64,
    url: String,
    target: u32,
    raw_cache: Arc<ImageCache>,
    thumb_cache: Arc<ImageCache>,
    fetcher: crate::app::state::ImageFetcher,
) -> Message {
    match fetch_thumbnail(
        id,
        url,
        target,
        raw_cache,
        thumb_cache,
        fetcher,
        crate::media::provider_of(id),
    )
    .await
    {
        Ok(bytes) => Message::ColorPassPalette(id, extract_palette_result(bytes).await),
        Err(e) => {
            log::warn!("[colors] {} palette fetch failed: {}", id, e);
            Message::ColorPassPalette(id, Vec::new())
        }
    }
}

/// Spawn "process all colors" workers from the pending queue, honoring the
/// pool budget. Each worker fetches (cached) + extracts a palette.
///
/// In-flight workers are counted (`color_in_flight`), so completions only
/// refill the free slots instead of spawning a new burst of eight on every
/// result — the old unbounded refill ballooned a long pass into thousands of
/// concurrent fetches (all queueing behind the shared limiter) and flooded
/// the UI thread. The pass is finished when the queue is empty and nothing is
/// in flight.
pub(super) fn spawn_color_workers(state: &mut AppState) -> Vec<iced::Task<Message>> {
    let images = &mut state.images;
    if images.color_queue.is_empty() {
        if images.color_in_flight == 0 {
            images.color_processing = false;
        }
        return Vec::new();
    }
    let budget = THUMBNAIL_CONCURRENCY.saturating_sub(images.color_in_flight);
    let take = budget.min(images.color_queue.len());
    let mut tasks: Vec<iced::Task<Message>> = Vec::with_capacity(take);
    // Drain only the slots we start — the rest of the queue stays put instead
    // of being rebuilt (and re-allocated) on every completion.
    for (id, url, target) in images.color_queue.drain(..take) {
        let raw_cache = images.image_cache.clone();
        let thumb_cache = images.thumbnail_cache.clone();
        let fetcher = images.fetcher.clone();
        tasks.push(iced::Task::perform(
            fetch_and_extract_palette(id, url, target, raw_cache, thumb_cache, fetcher),
            |m| m,
        ));
    }
    images.color_in_flight += tasks.len();
    tasks
}

/// Read a full-size image from the download directory and emit it as a
/// `FullImageLoaded` (the highest-quality source — the original file).
pub(super) async fn read_full_from_disk(id: u64, page: usize, path: std::path::PathBuf) -> Message {
    let result = tokio::task::spawn_blocking(move || std::fs::read(&path))
        .await
        .map_err(|e| PixivError::Internal(format!("disk read join: {e}")));
    match result {
        Ok(Ok(bytes)) => Message::FullImageLoaded(id, page, Ok(bytes)),
        Ok(Err(e)) => {
            Message::FullImageLoaded(id, page, Err(PixivError::Io(std::sync::Arc::new(e))))
        }
        Err(e) => Message::FullImageLoaded(id, page, Err(e)),
    }
}

/// Drop every cached thumbnail so the next window report re-fetches at the
/// new grid size (used when the user changes the grid — stale-resolution
/// thumbnails are regenerated sharp). Returns the respawned fetch tasks.
pub fn reset_thumbnail_cache(state: &mut AppState) -> Vec<iced::Task<Message>> {
    let images = &mut state.images;
    images.thumbnail_handles.clear();
    images.thumbnail_requested.clear();
    images.thumbnail_queue.clear();
    images.thumbnail_in_flight.clear();
    // Every live fetch is now stale (wrong resolution): stop them too.
    for (_, handle) in images.thumbnail_aborts.drain() {
        handle.abort();
    }
    images.thumbnail_retries.clear();
    images.thumbnail_targets.clear();
    // Invalidate ONLY the processed-thumbnail cache (and the in-memory
    // handles). The raw URL→bytes cache (`image_cache`) survives so the next
    // pass re-resizes from already-downloaded bytes instead of re-fetching.
    images.thumbnail_cache.clear();
    spawn_thumbnails_for_window(state)
}

/// The illusts shown in the active tab's gallery — the FILTERED list, which
/// is exactly what the virtual gallery renders. Using the full (unfiltered)
/// list here mismatched the content-coordinate window when a filter shrank
/// the grid, so thumbnails for the wrong illusts got requested (the filtered
/// tiles stayed placeholders).
fn active_tab_illusts(state: &AppState) -> &[Illustration] {
    &state.active_tab_data().filtered
}

/// Estimated gallery viewport height before the first scroll report.
pub(super) fn viewport_estimate(state: &AppState) -> f32 {
    (state.window_size.1 - 180.0).max(200.0)
}

/// Spawn thumbnail fetch tasks for all illustrations that have thumbnail URLs
/// (used by the artist page). `target` is the pixel size to generate.
pub(super) fn spawn_thumbnail_fetches(
    illusts: &[Illustration],
    target: u32,
    high_res: bool,
    raw_cache: Arc<ImageCache>,
    thumb_cache: Arc<ImageCache>,
    fetcher: crate::app::state::ImageFetcher,
) -> Vec<iced::Task<Message>> {
    let mut tasks = Vec::new();
    for illust in illusts {
        if let Some(url) = thumbnail_source_url(illust, target, high_res) {
            let id = illust.id;
            let raw_cache = raw_cache.clone();
            let thumb_cache = thumb_cache.clone();
            let fetcher = fetcher.clone();
            let url_for_msg = url.clone();
            let provider = crate::media::provider_of(id);
            tasks.push(iced::Task::perform(
                fetch_thumbnail(id, url, target, raw_cache, thumb_cache, fetcher, provider),
                move |result| Message::ThumbnailLoaded(id, url_for_msg, target, result),
            ));
        }
    }
    tasks
}

pub(super) enum Outcome<T> {
    Progress(f32),
    Continue,
    Finished(Result<T, PixivError>),
}

/// Merge a newly fetched page into a date-sorted (newest first) item list.
///
/// Lazily paginated feeds append pages over time; this keeps the list ordered
/// without re-sorting it on every arrival: the incoming page is sorted once
/// (feeds are roughly chronological, but unwrapped reposts can break that),
/// de-duplicated against what is already shown, then merged in O(n+m). The
/// destination is expected to be sorted already; if it is not (first page),
/// it is sorted once and subsequent pages stay on the fast path.
pub(super) fn merge_page_by_date(
    dst: &mut Vec<pixivj_rs::model::Illustration>,
    mut page: Vec<pixivj_rs::model::Illustration>,
) {
    let mut seen: std::collections::HashSet<u64> = dst.iter().map(|i| i.id).collect();
    page.retain(|i| seen.insert(i.id));
    if page.is_empty() {
        return;
    }
    page.sort_by_key(|i| std::cmp::Reverse(i.create_date));
    if dst.is_empty() {
        *dst = page;
        return;
    }
    if dst.windows(2).any(|w| w[0].create_date < w[1].create_date) {
        // The destination was never sorted (e.g. pre-existing data).
        dst.extend(page);
        dst.sort_by_key(|i| std::cmp::Reverse(i.create_date));
        return;
    }
    let mut merged = Vec::with_capacity(dst.len() + page.len());
    let mut left = std::mem::take(dst).into_iter().peekable();
    let mut right = page.into_iter().peekable();
    loop {
        match (left.peek(), right.peek()) {
            (Some(a), Some(b)) => {
                if a.create_date >= b.create_date {
                    merged.push(left.next().expect("peeked"));
                } else {
                    merged.push(right.next().expect("peeked"));
                }
            }
            (Some(_), None) => merged.push(left.next().expect("peeked")),
            (None, Some(_)) => merged.push(right.next().expect("peeked")),
            (None, None) => break,
        }
    }
    *dst = merged;
}

/// Debug-print a message with any token payloads redacted, so secrets never
/// land in the logs even at debug level.
fn redact_message(message: &Message) -> String {
    match message {
        Message::TokenSubmitted(_) => "TokenSubmitted(<redacted>)".into(),
        Message::TokenInputChanged(_) => "TokenInputChanged(<redacted>)".into(),
        Message::TokenValidated(Ok(_)) => "TokenValidated(Ok(<redacted>))".into(),
        Message::TokenValidated(Err(e)) => format!("TokenValidated(Err({e}))"),
        Message::CodeExchanged(Ok(_)) => "CodeExchanged(Ok(<redacted>))".into(),
        Message::CodeExchanged(Err(e)) => format!("CodeExchanged(Err({e}))"),
        _ => format!("{:?}", message),
    }
}

pub(super) mod bilibili;
pub(super) mod gallery;
pub(super) mod jobs;
pub(super) mod onboarding;
pub(super) mod settings;
pub mod transform;
pub(super) mod twitter;
pub(super) mod viewer;

/// The reducer. After every message the viewer-transform sync starts any
/// render or split probe the (possibly changed) viewer state calls for.
pub fn update(state: &mut AppState, message: Message) -> iced::Task<Message> {
    let task = update_message(state, message);
    if state.viewer.open {
        iced::Task::batch([task, transform::sync(state)])
    } else {
        task
    }
}

fn update_message(state: &mut AppState, message: Message) -> iced::Task<Message> {
    log::debug!("[update] Message: {}", redact_message(&message));
    match message {
        // ── Onboarding: capture server, PKCE, token validation ──
        Message::StartOnboarding
        | Message::ClientBuilt(_)
        | Message::OnboardingServerReady(_)
        | Message::TokenInputChanged(_)
        | Message::OpenUrl(_)
        | Message::PickDirectory
        | Message::TokenSubmitted(_)
        | Message::CodeExchanged(_)
        | Message::TokenValidated(_)
        | Message::DirectorySelected(_)
        | Message::OnboardingComplete => onboarding::handle(state, message),

        // ── Gallery tabs: fetch/paginate/filter/sort/search ──
        Message::TabSelected(_)
        | Message::LoadMore
        | Message::ResetTab
        | Message::RecommendedFetched(_)
        | Message::FollowFeedFetched(_)
        | Message::BookmarksFetched(_)
        | Message::SearchFetched(_)
        | Message::RankedFetched(_)
        | Message::SearchTextChanged(_)
        | Message::SearchTargetChanged(..)
        | Message::SearchSubmitted
        | Message::TagSearchRequested(_)
        | Message::ApiSortChanged(_)
        | Message::ApiTargetChanged(_)
        | Message::ApiTranslatedChanged(_)
        | Message::ToggleAutoLoad
        | Message::AutocompleteLoaded(..)
        | Message::AutocompleteTagClicked(_)
        | Message::AutocompleteMove(_)
        | Message::AutocompleteCommit
        | Message::FocusSearch
        | Message::ScrollPositionChanged(_)
        | Message::GalleryWindowChanged { .. }
        | Message::ThumbnailWindowPulse
        | Message::RefreshDownloaded
        | Message::ScanExtra
        | Message::ExtraScanned(_)
        | Message::LocalThumbnailLoaded(..)
        | Message::PaletteExtracted(..)
        | Message::ColorPassPalette(..)
        | Message::ProcessAllColors
        | Message::ColorSortChanged(_)
        | Message::FilterModeChanged(_)
        | Message::QueryTokenRemoved(_)
        | Message::QuerySuggestionClicked(_)
        | Message::CustomColorSelected
        | Message::CustomColorTextChanged(_)
        | Message::ColorLevelChanged(_)
        | Message::OptionFilterToggled(_)
        | Message::SourceFilterToggled(_)
        | Message::TagAdded(_)
        | Message::TagInputChanged(_)
        | Message::TagRemoved(_)
        | Message::ToolFilterToggled(_)
        | Message::ToggleAdvancedFilters
        | Message::ResetFilters
        | Message::SortKeyClicked(_)
        | Message::MultiSortToggled
        | Message::RankingModeChanged(_)
        | Message::RankingDateChanged(_)
        | Message::TagFocusChanged(_)
        | Message::TagFocusGrace => gallery::handle(state, message),

        // ── Viewer + artist overlays: detail, comments, full image ──
        Message::IllustDetailFetched(_)
        | Message::CommentsFetched(_)
        | Message::CommentPfpLoaded(..)
        | Message::CommentStampLoaded(..)
        | Message::UserDetailFetched(_)
        | Message::UserIllustsFetched(_)
        | Message::ThumbnailLoaded(..)
        | Message::FullImageProgress(..)
        | Message::FullImagePartial(..)
        | Message::FullImageLoaded(..)
        | Message::GalleryItemClicked(_)
        | Message::GalleryItemDoubleClicked(_)
        | Message::ViewerOpened(_)
        | Message::ViewerClosed
        | Message::EscapePressed
        | Message::ViewerPageChanged(_)
        | Message::ViewerPageStep(_)
        | Message::ViewerPageNavigate(_)
        | Message::ViewerPageInputChanged(_)
        | Message::ViewerPageSubmitted
        | Message::ViewerPageSettled
        | Message::ArtistLoadMore
        | Message::CommentsLoadMore
        | Message::CommentsPageFetched(_)
        | Message::ArtistOpened(_)
        | Message::ArtistPfpLoaded(_)
        | Message::ArtistClosed => viewer::handle(state, message),

        // ── Twitter viewer: combine split images, rotate ──
        Message::ViewerMergeToggled(_)
        | Message::ViewerRotate(_)
        | Message::ViewerTransformReset
        | Message::ViewerSplitDetected(..)
        | Message::ViewerTransformReady(..) => transform::handle(state, message),

        // ── Twitter: client, feed, likes, replies ──
        Message::TwitterClientBuilt(_)
        | Message::TwitterCookiePathChanged(_)
        | Message::ReloadTwitterCookies
        | Message::RefreshTwitterQueryIds
        | Message::TwitterQueryIdsFetched(_)
        | Message::TwitterFeedModeChanged(_)
        | Message::TwitterFeedReload
        | Message::TwitterFeedFetched(..)
        | Message::TwitterLikesFetched(_)
        | Message::TwitterArtistLoaded(..)
        | Message::TwitterTweetFetched(..)
        | Message::TwitterRepliesFetched(_)
        | Message::TwitterRepliesPageFetched(_) => twitter::handle(state, message),

        // ── Bilibili: client, dynamic feeds, comments, local likes ──
        Message::BilibiliClientBuilt(_)
        | Message::BilibiliCookiePathChanged(_)
        | Message::BilibiliUserIdsChanged(_)
        | Message::ReloadBilibiliCookies
        | Message::BilibiliFeedModeChanged(_)
        | Message::BilibiliFeedReload
        | Message::BilibiliFeedFetched(..)
        | Message::BilibiliUserFetched(..)
        | Message::BilibiliArtistLoaded(..)
        | Message::BilibiliDynamicFetched(..)
        | Message::BilibiliCommentsFetched(_)
        | Message::BilibiliCommentsPageFetched(_)
        | Message::BilibiliLikeResult(..) => bilibili::handle(state, message),

        // ── Jobs: bookmark/follow/download/sync ──
        Message::BookmarkToggled(_)
        | Message::BookmarkResult(..)
        | Message::IllustDetailRefreshed(..)
        | Message::FollowToggled(_)
        | Message::FollowResult(..)
        | Message::TwitterLikeResult(..)
        | Message::DownloadStarted(_)
        | Message::StartBookmarkSync
        | Message::CancelBookmarkSync
        | Message::CancelDownloadAll
        | Message::BookmarkSyncProgress { .. }
        | Message::BookmarkSyncFinished { .. }
        | Message::StartDownloadAll
        | Message::DownloadProgress(..)
        | Message::DownloadCompleted(..)
        | Message::ToggleDownloadManager
        | Message::DownloadPriorityUp(_)
        | Message::DownloadPriorityDown(_) => jobs::handle(state, message),

        // ── Window / theme / settings / browser / misc ──
        Message::ToggleDebugOverlay
        | Message::ToggleShortcuts
        | Message::WindowRegistered(_)
        | Message::WindowClosed
        | Message::WindowCloseRequested(_)
        | Message::ScaleFactorChanged(_)
        | Message::ModifiersChanged(_)
        | Message::WindowResized { .. }
        | Message::SystemThemeFetched(_)
        | Message::SystemThemeChanged(_)
        | Message::ThemePreferenceChanged(_)
        | Message::OpenInBrowser(_)
        | Message::CopyIllustUrl(_)
        | Message::RevealIllustFiles(_)
        | Message::GridSizeChanged(_)
        | Message::GridSizeStep(_)
        | Message::OpenSettings
        | Message::CloseSettings
        | Message::AutoPreloadChanged(_)
        | Message::DownloadConcurrencyChanged(_)
        | Message::GalleryPathInputChanged(_)
        | Message::ProxyInputChanged(_)
        | Message::ParallaxChanged(_)
        | Message::ParallaxMouseChanged(_)
        | Message::ParallaxScrollChanged(_)
        | Message::ParallaxModeChanged(_)
        | Message::GalleryFontSizeChanged(_)
        | Message::GalleryTextStyleChanged(_)
        | Message::TileRowsChanged(..)
        | Message::BookmarkedDisplayChanged(_)
        | Message::BlurR18Changed(_)
        | Message::TwitterViewerToolsChanged(_)
        | Message::MemoryPresetChanged(_)
        | Message::FpsLimitChanged(_)
        | Message::ThumbnailHighResChanged(_)
        | Message::ToggleHelp
        | Message::ResetSetting(_)
        | Message::ResetAllSettings
        | Message::Error(_)
        | Message::DismissError
        | Message::ApplyProxy
        | Message::ClientProxied(_)
        | Message::Noop
        | Message::FlushDisk => settings::handle(state, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{AppPhase, AppState};

    fn pool_state(n: usize) -> AppState {
        let mut state = AppState {
            phase: AppPhase::Main,
            window_size: (1600.0, 900.0),
            ..Default::default()
        };
        state.tabs.recommend.illustrations = (1..=n)
            .map(|i| crate::test_util::mock_illust(i as u64))
            .collect();
        state.tabs.recommend.filtered = state.tabs.recommend.illustrations.clone();
        // A window covering everything (so the lazy scan enqueues all n).
        state.gallery_window = Some((0.0, 1_000_000.0));
        state
    }

    fn window_url(state: &AppState, id: u64) -> String {
        state
            .tabs
            .recommend
            .illustrations
            .iter()
            .find(|i| i.id == id)
            .and_then(|i| i.image_urls.as_ref())
            .and_then(|u| u.medium.clone())
            .expect("url")
    }

    #[test]
    fn stale_gallery_window_beyond_content_falls_back_to_top() {
        // A filter/tab change leaves `gallery_window` pointing at the OLD,
        // deeper scroll position. The new (filtered) content is short, so
        // that window would map to an empty index range and NO thumbnails
        // would spawn until the user scrolls. It must fall back to the top.
        let mut state = pool_state(10);
        state.gallery_window = Some((1_000_000.0, 1_000_600.0));
        let tasks = spawn_thumbnails_for_window(&mut state);
        assert!(
            !state.images.thumbnail_requested.is_empty(),
            "fell back to the top and enqueued thumbnails"
        );
        assert!(!tasks.is_empty());
    }

    #[test]
    fn thumbnail_pool_bounds_concurrency() {
        let mut state = pool_state(500);
        spawn_thumbnails_for_window(&mut state);
        assert_eq!(
            state.images.thumbnail_in_flight.len(),
            THUMBNAIL_CONCURRENCY
        );
        assert!(!state.images.thumbnail_queue.is_empty());
        assert_eq!(
            state.images.thumbnail_requested.len(),
            500,
            "the whole window is enqueued"
        );
    }

    #[test]
    fn average_rgb_of_a_solid_thumbnail() {
        use image::ImageEncoder;
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([200, 100, 50]));
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(img.as_raw(), 8, 8, image::ExtendedColorType::Rgb8)
            .expect("encode");
        assert_eq!(average_rgb(&png), [200, 100, 50]);
        // Undecodable bytes fall back to the neutral gray.
        assert_eq!(average_rgb(b"not an image"), [128, 128, 128]);
    }

    #[test]
    fn color_pool_is_bounded_and_finishes() {
        // Regression: every completion used to spawn a fresh burst of eight
        // (in-flight was never counted), so a long "process all" pass
        // ballooned into unbounded concurrent fetches and stalled the UI.
        let mut state = pool_state(0);
        state.images.color_total = 25;
        state.images.color_queue = (1..=25u64)
            .map(|i| (i, format!("https://example.com/{i}.jpg"), 240))
            .collect();
        state.images.color_processing = true;

        let tasks = spawn_color_workers(&mut state);
        assert_eq!(tasks.len(), THUMBNAIL_CONCURRENCY, "first wave is bounded");
        assert_eq!(state.images.color_in_flight, THUMBNAIL_CONCURRENCY);
        assert_eq!(state.images.color_queue.len(), 25 - THUMBNAIL_CONCURRENCY);
        assert!(state.images.color_processing);
        drop(tasks);

        // Each completion frees a slot and refills at most one; the pool must
        // never exceed the budget, and the pass only ends when both the queue
        // and the pool are empty.
        for id in 1..=25u64 {
            let _ = update(&mut state, Message::ColorPassPalette(id, vec![[1, 2, 3]]));
            assert!(
                state.images.color_in_flight <= THUMBNAIL_CONCURRENCY,
                "pool stays bounded at id {id}"
            );
        }
        assert!(state.images.color_queue.is_empty());
        assert!(!state.images.color_processing, "the pass finished");
        assert_eq!(state.images.color_done, 25);
        assert_eq!(state.images.palettes.len(), 25);
    }

    #[test]
    fn thumbnail_pool_retries_failures_and_refills() {
        let mut state = pool_state(500);
        spawn_thumbnails_for_window(&mut state);
        assert_eq!(
            state.images.thumbnail_in_flight.len(),
            THUMBNAIL_CONCURRENCY
        );

        // Fail one in-flight fetch: it is re-queued and the pool refills.
        let failed_id = *state
            .images
            .thumbnail_in_flight
            .iter()
            .next()
            .expect("an in-flight id");
        let url = window_url(&state, failed_id);
        let target = thumbnail_target(&state);
        let _ = update(
            &mut state,
            Message::ThumbnailLoaded(
                failed_id,
                url,
                target,
                Err(PixivError::HttpStatus(500, "x".into())),
            ),
        );
        assert_eq!(
            state.images.thumbnail_retries.get(&failed_id),
            Some(&1),
            "failure count tracked"
        );
        assert_eq!(
            state.images.thumbnail_in_flight.len(),
            THUMBNAIL_CONCURRENCY,
            "pool refills the worker slot immediately"
        );
        assert!(
            state
                .images
                .thumbnail_queue
                .iter()
                .any(|(id, _, _)| *id == failed_id)
                || state.images.thumbnail_in_flight.contains(&failed_id),
            "failed id is re-queued for a retry"
        );
    }

    #[test]
    fn thumbnail_pool_gives_up_after_max_retries_without_blocking() {
        let mut state = pool_state(500);
        spawn_thumbnails_for_window(&mut state);

        // Mark an id as having exhausted its retries, then queue it anyway —
        // the pool must NOT fetch it again (no repeated-download loop).
        let gave_up_id = 42u64;
        let url = window_url(&state, gave_up_id);
        state
            .images
            .thumbnail_retries
            .insert(gave_up_id, THUMBNAIL_MAX_RETRIES);
        state
            .images
            .thumbnail_queue
            .push((gave_up_id, url, thumbnail_target(&state)));
        spawn_thumbnail_workers(&mut state);
        assert!(
            !state.images.thumbnail_in_flight.contains(&gave_up_id),
            "a gave-up id is never fetched again"
        );

        // The handler path: once retries are exhausted, a further failure
        // must not re-queue the id.
        let failed_id = *state
            .images
            .thumbnail_in_flight
            .iter()
            .next()
            .expect("an in-flight id");
        let url = window_url(&state, failed_id);
        state
            .images
            .thumbnail_retries
            .insert(failed_id, THUMBNAIL_MAX_RETRIES);
        let target = thumbnail_target(&state);
        let _ = update(
            &mut state,
            Message::ThumbnailLoaded(
                failed_id,
                url,
                target,
                Err(PixivError::HttpStatus(500, "x".into())),
            ),
        );
        assert!(
            !state
                .images
                .thumbnail_queue
                .iter()
                .any(|(id, _, _)| *id == failed_id),
            "given-up id is not re-queued"
        );
        // The pool still has other work going.
        assert!(
            !state.images.thumbnail_in_flight.is_empty(),
            "other thumbnails keep loading after one gives up"
        );
    }

    #[test]
    fn thumbnail_pool_success_clears_retry_state() {
        let mut state = pool_state(500);
        spawn_thumbnails_for_window(&mut state);

        let id = *state
            .images
            .thumbnail_in_flight
            .iter()
            .next()
            .expect("an in-flight id");
        let url = window_url(&state, id);
        let target = thumbnail_target(&state);
        let _ = update(
            &mut state,
            Message::ThumbnailLoaded(
                id,
                url,
                target,
                Err(PixivError::HttpStatus(500, "x".into())),
            ),
        );
        assert_eq!(state.images.thumbnail_retries.get(&id), Some(&1));

        // A success resets the retry counter and stores the handle.
        let url = window_url(&state, id);
        let target = thumbnail_target(&state);
        let _ = update(
            &mut state,
            Message::ThumbnailLoaded(id, url, target, Ok(vec![0x89, 0x50, 0x4E, 0x47])),
        );
        assert!(!state.images.thumbnail_retries.contains_key(&id));
        assert!(state.images.thumbnail_handles.contains_key(&id));
    }

    #[test]
    fn out_of_view_thumbnail_fetches_are_cancelled() {
        const COLS: f32 = 8.0; // 1600px fallback width / 200px grid
        const ROW: f32 = 200.0;

        let mut state = pool_state(500);
        spawn_thumbnails_for_window(&mut state);
        let top_batch: std::collections::HashSet<u64> = state.images.thumbnail_in_flight.clone();
        assert_eq!(top_batch.len(), THUMBNAIL_CONCURRENCY);

        // Fling to the very last rows: everything started for the top is now
        // far outside the (padded) window and must be cancelled, not just
        // left running.
        let rows = (500_f32 / COLS).ceil();
        state.gallery_window = Some(((rows - 1.0) * ROW, rows * ROW));
        spawn_thumbnails_for_window(&mut state);

        for id in &top_batch {
            assert!(
                !state.images.thumbnail_in_flight.contains(id),
                "in-flight fetch for {id} survived the jump"
            );
            assert!(
                !state.images.thumbnail_requested.contains(id),
                "cancelled {id} must be re-enqueueable"
            );
            assert!(
                !state.images.thumbnail_aborts.contains_key(id),
                "abort handle for {id} must be dropped"
            );
        }
        // The freed slots immediately refill with tail fetches.
        assert_eq!(
            state.images.thumbnail_in_flight.len(),
            THUMBNAIL_CONCURRENCY
        );
        let first_tail_row = ((rows - 1.0) - 3.0) * COLS;
        for id in &state.images.thumbnail_in_flight {
            assert!(
                *id as f32 >= first_tail_row,
                "refill {id} is not near the new window"
            );
        }
    }

    #[test]
    fn small_scroll_does_not_cancel_in_flight_thumbnails() {
        let mut state = pool_state(500);
        spawn_thumbnails_for_window(&mut state);
        let batch: std::collections::HashSet<u64> = state.images.thumbnail_in_flight.clone();

        // Drift one viewport down — the cancel pad must absorb it.
        let viewport = viewport_estimate(&state);
        state.gallery_window = Some((viewport, 2.0 * viewport));
        spawn_thumbnails_for_window(&mut state);

        for id in &batch {
            assert!(
                state.images.thumbnail_in_flight.contains(id),
                "small scroll aborted in-flight fetch for {id}"
            );
            assert!(state.images.thumbnail_aborts.contains_key(id));
        }
    }
}
