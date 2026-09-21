//! Viewer + artist overlays: detail, comments, full-image streaming.

use super::gallery::{fetch_comments, fetch_illust_detail, fetch_user_detail, fetch_user_illusts};
use super::{fetch_pfp, spawn_thumbnail_fetches, Outcome};
use crate::app::message::Message;
use crate::app::state::AppState;
use crate::image::cache::ImageCache;
use pixivj_rs::error::PixivError;
use pixivj_rs::model::*;
use std::sync::Arc;

fn full_image_url(illust: &Illustration, page: usize) -> Option<String> {
    if let Some(meta_page) = illust.meta_pages.get(page) {
        if let Some(ref url) = meta_page.image_urls.original {
            return Some(url.clone());
        }
        if let Some(ref url) = meta_page.image_urls.large {
            return Some(url.clone());
        }
    }
    if page == 0 {
        if let Some(ref single) = illust.meta_single_page {
            if let Some(ref url) = single.original_image_url {
                return Some(url.clone());
            }
        }
        if let Some(ref urls) = illust.image_urls {
            if let Some(ref url) = urls.large {
                return Some(url.clone());
            }
        }
    }
    None
}

/// Whether `bytes` (a download prefix) is a **progressive** JPEG — the only
/// format whose partial prefix the image crate can decode for display.
///
/// Walks JPEG segment headers from SOI looking for an SOF2 marker
/// (`0xFF 0xC2`). Any other SOF variant means baseline/lossless → `false`;
/// reaching Start-of-Scan without a SOF means the headers are still
/// incomplete or exotic → `false` (conservative: no partial previews).
fn is_progressive_jpeg(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return false;
    }
    let limit = bytes.len().min(1 << 20);
    let mut i = 2usize;
    while i + 4 <= limit {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1; // fill byte
            continue;
        }
        // Standalone markers carry no length segment.
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        match marker {
            // SOF2 = progressive. Any other SOFn is not decodable partially.
            0xC2 => return true,
            0xC0 | 0xC1 | 0xC3..=0xCF => return false,
            // SOS: entropy-coded data starts — no SOF seen before it.
            0xDA => return false,
            _ => {}
        }
        let seg_len = (((bytes[i + 2] as usize) << 8) | bytes[i + 3] as usize).max(2);
        i += 2 + seg_len;
    }
    false
}

/// The stream emits `FullImageProgress` while downloading (throttled to a
/// few updates per second) and ends with `FullImageLoaded`. A cache hit
/// short-circuits straight to `FullImageLoaded` with no progress messages.
fn full_image_task(
    id: u64,
    page: usize,
    url: String,
    cache: Arc<ImageCache>,
    fetcher: crate::app::state::ImageFetcher,
    limiter: Arc<pixivj_rs::rate_limit::RateLimiter>,
) -> iced::Task<Message> {
    iced::Task::run(
        FullImageFetcher::new(id, page, url, cache, fetcher, limiter).into_stream(),
        |m| m,
    )
}

/// Incremental full-image downloader. Drives itself one message at a time so
/// the iced runtime stays responsive (no blocking the UI thread).
struct FullImageFetcher {
    id: u64,
    page: usize,
    url: String,
    cache: Arc<ImageCache>,
    fetcher: crate::app::state::ImageFetcher,
    phase: Option<FullImagePhase>,
    /// Held for the whole fetch: while a window is active only one fetch is
    /// past it at a time. Dropped when the fetch ends, freeing the next one.
    gate: Option<pixivj_rs::rate_limit::RateLimitGuard>,
    done: bool,
    /// Off-thread preview worker for progressive JPEGs (`None` until the
    /// network transfer starts; cache/disk hits never spawn one).
    previewer: Option<PartialPreviewer>,
    /// The rate limiter for THIS fetch — `page_limiter` for the page the user
    /// is looking at, `preload_limiter` for background preloads. Separate
    /// lanes so a preload burst's 429 backoff can't delay the current page.
    limiter: Arc<pixivj_rs::rate_limit::RateLimiter>,
}

/// The preview worker refreshes its decode at this interval. It runs entirely
/// off the download path — the reader never waits for a decode.
const PARTIAL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
/// Longest side (px) of the partial image shown while a full download streams.
const PARTIAL_MAX_PX: u32 = 1400;
/// How many times a rate-limited full-image request is retried (like the
/// download queue does) before giving up.
const MAX_FULL_RETRIES: u32 = 4;

/// A dedicated preview thread fed by the download loop.
///
/// The thread wakes every [`PARTIAL_INTERVAL`], snapshots the bytes received
/// so far and — only if the payload is a **progressive JPEG** (the sole
/// format whose prefix can be rendered) — decodes + downsizes the snapshot
/// and ships it back over a channel. The download loop hands chunks in and
/// drains finished handles non-blockingly between chunk reads; a slow decode
/// can never stall the transfer itself (that throttling was why the detail
/// view streamed slower than the download queue).
struct PartialPreviewer {
    /// Bytes received so far, shared with the worker thread.
    buffer: Arc<std::sync::Mutex<Vec<u8>>>,
    /// Tells the worker to exit; set on completion/drop of the stream.
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    rx: std::sync::mpsc::Receiver<iced::widget::image::Handle>,
}

impl PartialPreviewer {
    fn new(id: u64, page: usize) -> Self {
        let buffer = Arc::new(std::sync::Mutex::new(Vec::new()));
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let buf = buffer.clone();
        let stop = shutdown.clone();
        std::thread::Builder::new()
            .name(format!("preview-{id}-p{page}"))
            .spawn(move || {
                let mut last_len = 0usize;
                loop {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(PARTIAL_INTERVAL);
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    // Snapshot under a short lock; the decode runs on the copy.
                    let snap = {
                        let b = buf.lock().unwrap_or_else(|e| e.into_inner());
                        if b.len() == last_len || b.len() < 4 {
                            continue;
                        }
                        b.clone()
                    };
                    last_len = snap.len();
                    // Only progressive JPEGs can render partial prefixes;
                    // everything else would be wasted CPU on the hot path.
                    if !is_progressive_jpeg(&snap) {
                        continue;
                    }
                    let handle = crate::app::update::resize_to_thumbnail(&snap, PARTIAL_MAX_PX)
                        .ok()
                        .map(iced::widget::image::Handle::from_bytes);
                    match handle {
                        Some(handle) => {
                            if tx.send(handle).is_err() {
                                break; // stream gone
                            }
                        }
                        None => break, // undecodable even progressively
                    }
                }
            })
            .expect("spawn preview thread");
        Self {
            buffer,
            shutdown,
            rx,
        }
    }

    /// Hand a freshly-read chunk to the worker's buffer (cheap lock).
    fn extend(&self, chunk: &[u8]) {
        self.buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(chunk);
    }

    /// Non-blocking: the next finished preview, if the worker produced one.
    fn poll(&mut self) -> Option<iced::widget::image::Handle> {
        self.rx.try_recv().ok()
    }

    /// Consume the whole buffer (download finished).
    fn take_bytes(&mut self) -> Vec<u8> {
        std::mem::take(&mut *self.buffer.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl Drop for PartialPreviewer {
    fn drop(&mut self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

enum FullImagePhase {
    Network {
        response: reqwest::Response,
        received: u64,
        total: Option<u64>,
        last_report: Option<f32>,
    },
}

impl FullImageFetcher {
    fn new(
        id: u64,
        page: usize,
        url: String,
        cache: Arc<ImageCache>,
        fetcher: crate::app::state::ImageFetcher,
        limiter: Arc<pixivj_rs::rate_limit::RateLimiter>,
    ) -> Self {
        Self {
            id,
            page,
            url,
            cache,
            fetcher,
            limiter,
            phase: None,
            gate: None,
            done: false,
            previewer: None,
        }
    }

    fn into_stream(self) -> impl iced::futures::Stream<Item = Message> + Send + 'static {
        iced::futures::stream::unfold(self, |mut this| async move {
            this.next().await.map(|msg| (msg, this))
        })
    }

    async fn next(&mut self) -> Option<Message> {
        if self.done {
            return None;
        }

        // Kick off the download on the first step (or serve from cache). A 429 is
        // retried a few times (like the download queue does) instead of giving
        // up — that was why the detail view's high-res image "never loaded"
        // while downloads (which retry) worked.
        if self.phase.is_none() {
            if let Some(bytes) = self.cache.get(&self.url) {
                self.done = true;
                return Some(Message::FullImageLoaded(self.id, self.page, Ok(bytes)));
            }
            let mut attempts = 0;
            let resp = loop {
                // During a backoff window the gate lets one request through
                // at a time and waits out the window.
                self.gate = self.limiter.wait_if_limited().await;
                let resp = match super::image_request(
                    &self.fetcher,
                    &self.url,
                    crate::media::provider_of(self.id),
                )
                .send()
                .await
                {
                    Ok(r) => {
                        self.limiter.on_request_done(
                            r.status().as_u16() == 429,
                            pixivj_rs::rate_limit::retry_after_header(&r),
                        );
                        r
                    }
                    Err(e) => {
                        self.limiter.on_request_done(false, None);
                        self.done = true;
                        return Some(Message::FullImageLoaded(
                            self.id,
                            self.page,
                            Err(PixivError::Internal(e.to_string())),
                        ));
                    }
                };
                // Rate-limited: arm the backoff (done above) and retry until
                // the retry budget is exhausted.
                if resp.status().as_u16() == 429 && attempts < MAX_FULL_RETRIES {
                    attempts += 1;
                    continue;
                }
                break resp;
            };
            if !resp.status().is_success() {
                self.done = true;
                return Some(Message::FullImageLoaded(
                    self.id,
                    self.page,
                    Err(PixivError::HttpStatus(
                        resp.status().as_u16(),
                        "full image fetch failed".into(),
                    )),
                ));
            }
            let total = resp.content_length();
            self.previewer = Some(PartialPreviewer::new(self.id, self.page));
            self.phase = Some(FullImagePhase::Network {
                response: resp,
                received: 0,
                total,
                last_report: None,
            });
        }

        loop {
            let outcome = {
                let FullImagePhase::Network {
                    response,
                    received,
                    total,
                    last_report,
                } = self.phase.as_mut()?;
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        *received += chunk.len() as u64;
                        if let Some(previewer) = self.previewer.as_ref() {
                            previewer.extend(&chunk);
                        }
                        // Emit a finished off-thread decode if one is ready.
                        // Non-blocking — the reader NEVER waits on decodes;
                        // they run on the preview thread's own 1s timer.
                        if let Some(handle) = self.previewer.as_mut().and_then(|p| p.poll()) {
                            return Some(Message::FullImagePartial(self.id, self.page, handle));
                        }
                        match total {
                            Some(total) if *total > 0 => {
                                let frac = (*received as f64 / *total as f64) as f32;
                                if last_report.map_or(true, |last| frac - last >= 0.03) {
                                    *last_report = Some(frac);
                                    Outcome::Progress(frac.min(1.0))
                                } else {
                                    Outcome::Continue
                                }
                            }
                            _ => Outcome::Continue,
                        }
                    }
                    Ok(None) => {
                        let mut bytes = self
                            .previewer
                            .as_mut()
                            .map(|p| p.take_bytes())
                            .unwrap_or_default();
                        bytes.shrink_to_fit();
                        Outcome::Finished(Ok(bytes))
                    }
                    Err(e) => {
                        let err = PixivError::Internal(e.to_string());
                        Outcome::Finished(Err(err))
                    }
                }
            };
            match outcome {
                Outcome::Progress(frac) => {
                    return Some(Message::FullImageProgress(self.id, self.page, frac));
                }
                Outcome::Continue => {}
                Outcome::Finished(result) => {
                    self.phase = None;
                    self.done = true;
                    // Stop the preview thread; the download is over.
                    self.previewer = None;
                    // Release the gate. The request's status was ALREADY
                    // reported to the limiter when the response arrived
                    // (429 arms backoff, everything else is neutral) —
                    // reporting AGAIN here with `result.is_ok()` armed a
                    // fresh 5s+2s backoff window after EVERY successful
                    // load, so the next page sat at 0% waiting it out.
                    self.gate = None;
                    if let Ok(bytes) = &result {
                        self.cache.insert_async(&self.url, bytes.clone()).await;
                    }
                    return Some(Message::FullImageLoaded(self.id, self.page, result));
                }
            }
        }
    }
}

fn open_viewer(state: &mut AppState, id: u64) -> iced::Task<Message> {
    state.viewer.open = true;
    state.viewer.illust_id = id;
    state.viewer.page = 0;
    state.viewer.total_pages = 1;
    state.viewer.illust = None;
    state.viewer.comments.clear();
    state.viewer.comments_next_url = None;
    state.viewer.page_input.clear();
    state.viewer.page_display = "1".into();
    // Show the best image we already have: a cached full-res page from a
    // previous visit, else the gallery thumbnail.
    state.viewer.handle = state
        .viewer
        .page_handles
        .get(&0)
        .cloned()
        .or_else(|| state.images.thumbnail_handles.get(&id).cloned());
    state.viewer.progress = None;
    state.viewer.loading = true;
    state.viewer.author_pfp = None;

    // Tweets and dynamics are resolved by their own source modules, which seed
    // from live state (or the disk cache / a by-id fetch) and load comments.
    match crate::media::provider_of(id) {
        crate::media::Provider::Twitter => return super::twitter::open_viewer(state, id),
        crate::media::Provider::Bilibili => return super::bilibili::open_viewer(state, id),
        crate::media::Provider::Pixiv => {}
    }

    // Seed from the on-disk metadata cache so the detail panel + image show
    // instantly; a fresh fetch replaces it (and re-caches) once it lands.
    let mut cached_task = None;
    if let Some(illust) = crate::io::meta_cache::load_detail(id) {
        state.viewer.total_pages = illust.page_count();
        state.viewer.illust = Some(illust.clone());
        state.viewer.loading = false;
        cached_task = Some(full_image_or_cache(state));
    }
    if let Some(comments) = crate::io::meta_cache::load_comments(id) {
        if !comments.is_empty() {
            state.viewer.comments = comments;
        }
    }

    if let Some(client) = state.client.clone() {
        let detail_task = iced::Task::perform(
            fetch_illust_detail(client.clone(), id),
            Message::IllustDetailFetched,
        );
        let comments_task =
            iced::Task::perform(fetch_comments(client, id, None), Message::CommentsFetched);
        let settle = iced::Task::perform(
            async {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            },
            |_| Message::ViewerPageSettled,
        );
        let mut tasks = vec![detail_task, comments_task, settle];
        // Downloaded on disk? Show the original immediately (highest quality).
        if let Some(task) = load_full_from_disk(state, 0) {
            tasks.push(task);
        }
        if let Some(task) = cached_task {
            tasks.push(task);
        }
        return iced::Task::batch(tasks);
    }
    cached_task.unwrap_or_else(iced::Task::none)
}

/// The author's profile-picture URL for the current viewer item.
pub(super) fn author_pfp_url(illust: &Illustration) -> Option<String> {
    illust
        .user
        .as_ref()?
        .profile_image_urls
        .as_ref()?
        .medium
        .clone()
}

/// Apply a fetched comment page: persist it, then fetch the comment authors'
/// profile pictures and any comment stamps (all cached). Shared by the Pixiv,
/// Twitter and Bilibili comment paths.
pub(super) fn apply_comments(state: &mut AppState, comments: Comments) -> iced::Task<Message> {
    crate::io::meta_cache::save_comments(state.viewer.illust_id, &comments.comments);
    state.viewer.comments_next_url = comments.next_url.clone();
    state.viewer.comments = comments.comments;
    let mut tasks: Vec<iced::Task<Message>> = Vec::new();
    let mut seen_users = std::collections::HashSet::new();
    for comment in &state.viewer.comments {
        if let Some(user) = comment.user.as_ref() {
            if !state.viewer.comment_pfps.contains_key(&user.id) && seen_users.insert(user.id) {
                if let Some(url) = user
                    .profile_image_urls
                    .as_ref()
                    .and_then(|p| p.medium.clone())
                {
                    let cache = state.images.image_cache.clone();
                    let fetcher = state.images.fetcher.clone();
                    let uid = user.id;
                    let provider = crate::media::provider_of(uid);
                    tasks.push(iced::Task::perform(
                        fetch_pfp(url, cache, fetcher, provider),
                        move |r| Message::CommentPfpLoaded(uid, r),
                    ));
                }
            }
        }
        if let Some(url) = comment.stamp.as_ref().and_then(|s| s.stamp_url.clone()) {
            if !state.viewer.comment_stamps.contains_key(&comment.id) {
                let cache = state.images.image_cache.clone();
                let fetcher = state.images.fetcher.clone();
                let cid = comment.id;
                tasks.push(iced::Task::perform(
                    fetch_pfp(url, cache, fetcher, crate::media::Provider::Pixiv),
                    move |r| Message::CommentStampLoaded(cid, r),
                ));
            }
        }
    }
    iced::Task::batch(tasks)
}

/// If the illust's full image is downloaded on disk, load that original file
/// (always highest quality) instead of hitting the network. Returns the task,
/// or `None` when nothing is on disk for the current page.
pub(super) fn load_full_from_disk(state: &AppState, page: usize) -> Option<iced::Task<Message>> {
    let gallery = state
        .settings
        .gallery_path
        .clone()
        .unwrap_or_else(crate::core::default_gallery_dir);
    let id = state.viewer.illust_id;
    let dir = crate::core::migrate_download_dir(&gallery, id)?;
    let path = crate::core::find_page_file(&dir, id, page)?;
    log::info!("[viewer] Loading full image {} p{} from disk", id, page);
    Some(iced::Task::perform(
        super::read_full_from_disk(id, page, path),
        |m| m,
    ))
}

/// Start loading a viewer page's full image as an **abortable** task, so a
/// page navigation can pause (abort) in-flight preloads of other pages and
/// the current page always finishes first. Tracks the handle + page. `limiter`
/// picks the lane: `page_limiter` for the page being looked at,
/// `preload_limiter` for background pages.
fn start_page_load(
    state: &mut AppState,
    page: usize,
    url: String,
    id: u64,
    cache: Arc<ImageCache>,
    fetcher: crate::app::state::ImageFetcher,
    limiter: Arc<pixivj_rs::rate_limit::RateLimiter>,
) -> iced::Task<Message> {
    let (task, handle) = full_image_task(id, page, url, cache, fetcher, limiter).abortable();
    state.viewer.page_aborts.insert(page, handle);
    state.viewer.pages_loading.insert(page);
    task
}

/// Start loading the current viewer page's full image (or use a cached one),
/// returning the appropriate task. Priority: cached page → disk (downloaded
/// originals, highest quality) → network stream.
pub(super) fn full_image_or_cache(state: &mut AppState) -> iced::Task<Message> {
    let page = state.viewer.page;
    if let Some(handle) = state.viewer.page_handles.get(&page).cloned() {
        state.viewer.handle = Some(handle);
        state.viewer.progress = None;
        return iced::Task::none();
    }
    // Already downloaded on disk? Load the original directly — no network.
    if let Some(task) = load_full_from_disk(state, page) {
        let (task, handle) = task.abortable();
        state.viewer.page_aborts.insert(page, handle);
        state.viewer.pages_loading.insert(page);
        return task;
    }
    let Some(illust) = state.viewer.illust.clone() else {
        return iced::Task::none();
    };
    let Some(url) = full_image_url(&illust, page) else {
        state.viewer.progress = None;
        return iced::Task::none();
    };
    let id = illust.id;
    let cache = state.images.image_cache.clone();
    let fetcher = state.images.fetcher.clone();
    state.viewer.progress = Some(0.0);
    let limiter = fetcher.page_limiter.clone();
    start_page_load(state, page, url, id, cache, fetcher, limiter)
}

/// Switch the viewer to `page` (0-based): show a cached full-res page when
/// available, otherwise fall back to the thumbnail and stream the original.
/// Navigating pauses (aborts) every in-flight load of OTHER pages so the new
/// current page gets all the bandwidth.
fn goto_page(state: &mut AppState, page: usize) -> iced::Task<Message> {
    state.viewer.page = page;
    state.viewer.page_display = (page + 1).to_string();
    // Pause everything else: abort in-flight loads of pages we're not
    // navigating to. They'll be re-preloaded after the new current finishes.
    let mut aborts: Vec<iced::task::Handle> = Vec::new();
    state.viewer.page_aborts.retain(|&p, h| {
        if p == page {
            true // keep an already-in-flight load of the target page
        } else {
            aborts.push(h.clone());
            false
        }
    });
    for h in aborts {
        h.abort();
    }
    state.viewer.pages_loading.retain(|&p| p == page);
    let mut tasks = Vec::new();
    // Show the cached full-res page immediately if we have it.
    if let Some(handle) = state.viewer.page_handles.get(&page).cloned() {
        state.viewer.handle = Some(handle);
        state.viewer.progress = None;
    } else {
        // Otherwise fall back to the gallery thumbnail while streaming.
        state.viewer.handle = state
            .images
            .thumbnail_handles
            .get(&state.viewer.illust_id)
            .cloned();
        state.viewer.progress = Some(0.0);
        // Downloaded on disk? Load the original (highest quality) — later
        // pages of a multi-page illust must come from disk too, not the net.
        if let Some(task) = load_full_from_disk(state, page) {
            let (task, handle) = task.abortable();
            state.viewer.page_aborts.insert(page, handle);
            state.viewer.pages_loading.insert(page);
            tasks.push(task);
        } else if let Some(illust) = &state.viewer.illust {
            if let Some(url) = full_image_url(illust, page) {
                let id = illust.id;
                let cache = state.images.image_cache.clone();
                let fetcher = state.images.fetcher.clone();
                let limiter = fetcher.page_limiter.clone();
                tasks.push(start_page_load(
                    state,
                    page,
                    url,
                    id,
                    cache,
                    fetcher.clone(),
                    limiter,
                ));
            } else {
                state.viewer.progress = None;
            }
        } else {
            state.viewer.progress = None;
        }
    }
    // Preload nearby pages once the page index settles (~250ms).
    tasks.push(iced::Task::perform(
        async {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        },
        |_| Message::ViewerPageSettled,
    ));
    iced::Task::batch(tasks)
}

/// Preload the next pages of an illust, subject to the rule that the CURRENT
/// page must finish loading before any other page starts. Preloads are
/// abortable, so navigating pauses them and only the new current page loads.
fn preload_viewer_pages(state: &mut AppState) -> iced::Task<Message> {
    let mut tasks: Vec<iced::Task<Message>> = Vec::new();
    let Some(illust) = state.viewer.illust.clone() else {
        return iced::Task::none();
    };
    let page = state.viewer.page;
    let total = state.viewer.total_pages.max(1);
    let n = state.settings.auto_preload.max(1) as usize;

    // The current page is still loading → no other page may load (always
    // finish the current page before the next). Abort any stray preloads and
    // wait; `ViewerPageSettled` re-runs this once it finishes.
    let current_loaded = state.viewer.page_handles.contains_key(&page);
    let mut aborts: Vec<iced::task::Handle> = Vec::new();
    state.viewer.page_aborts.retain(|&p, h| {
        let in_window = p >= page.saturating_sub(n) && p <= page + n;
        let is_current = p == page;
        let keep = is_current
            || (in_window && current_loaded && !state.viewer.page_handles.contains_key(&p));
        if keep {
            true
        } else {
            aborts.push(h.clone());
            false
        }
    });
    for h in aborts {
        h.abort();
    }
    state
        .viewer
        .pages_loading
        .retain(|&p| p == page || (p >= page.saturating_sub(n) && p <= page + n && current_loaded));

    // Keep page handles within the window.
    state
        .viewer
        .page_handles
        .retain(|&p, _| p >= page.saturating_sub(n) && p <= page + n);

    // Don't start any preload until the current page has finished.
    if !current_loaded {
        return iced::Task::none();
    }

    // Preload the next pages that aren't loaded/loading yet — at most 2 at a
    // time. A burst of concurrent original-size streams just invites 429s
    // whose backoff windows then delay everything else.
    let max_in_flight = 2;
    let mut in_flight = state
        .viewer
        .pages_loading
        .iter()
        .filter(|&&p| p != page)
        .count();
    for p in (page + 1)..=(page + n).min(total.saturating_sub(1)) {
        if in_flight >= max_in_flight {
            break;
        }
        if state.viewer.page_handles.contains_key(&p) || state.viewer.pages_loading.contains(&p) {
            continue;
        }
        if let Some(url) = full_image_url(&illust, p) {
            let id = illust.id;
            let cache = state.images.image_cache.clone();
            let fetcher = state.images.fetcher.clone();
            let limiter = fetcher.preload_limiter.clone();
            in_flight += 1;
            tasks.push(start_page_load(state, p, url, id, cache, fetcher, limiter));
        }
    }
    iced::Task::batch(tasks)
}

/// Called after any filter/search change. Arms a fresh burst of auto-loads
/// and fires the first one if the filtered grid shrank below the minimum.
pub(super) fn handle(state: &mut AppState, message: Message) -> iced::Task<Message> {
    match message {
        Message::IllustDetailFetched(result) => {
            match result {
                Ok(detail) => {
                    // Guard against a stale response: the user may have closed
                    // the viewer or opened a different illust while this fetch
                    // was in flight. Without the check a slow response for an
                    // older illust would overwrite the current one.
                    if !state.viewer.open || state.viewer.illust_id != detail.illust.id {
                        return iced::Task::none();
                    }
                    // Persist the detail to the on-disk meta cache (atomic,
                    // compact — next open shows it instantly).
                    crate::io::meta_cache::save_detail(detail.illust.id, &detail.illust);
                    state.viewer.loading = false;
                    let page_count = detail.illust.page_count();
                    // Fetch the author's profile picture (cached) for the
                    // detail view's author row.
                    if let Some(url) = detail
                        .illust
                        .user
                        .as_ref()
                        .and_then(|u| u.profile_image_urls.as_ref())
                        .and_then(|p| p.medium.clone())
                    {
                        let cache = state.images.image_cache.clone();
                        let fetcher = state.images.fetcher.clone();
                        let provider = crate::media::provider_of(detail.illust.id);
                        let pfp =
                            iced::Task::perform(fetch_pfp(url, cache, fetcher, provider), |r| {
                                Message::ArtistPfpLoaded(r)
                            });
                        state.viewer.illust = Some(detail.illust);
                        state.viewer.total_pages = page_count;
                        let task = full_image_or_cache(state);
                        return iced::Task::batch([pfp, task]);
                    }
                    state.viewer.illust = Some(detail.illust);
                    state.viewer.total_pages = page_count;
                    full_image_or_cache(state)
                }
                Err(e) => {
                    log::error!("[viewer] Failed to load illust: {}", e);
                    iced::Task::done(Message::Error(format!("Failed to load illust: {e}")))
                }
            }
        }

        Message::CommentsFetched(result) => match result {
            Ok(comments) => apply_comments(state, comments),
            Err(e) => {
                log::error!("[viewer] Failed to load comments: {}", e);
                iced::Task::done(Message::Error(format!("Failed to load comments: {e}")))
            }
        },

        Message::CommentPfpLoaded(uid, result) => {
            match result {
                Ok(bytes) => {
                    state
                        .viewer
                        .comment_pfps
                        .insert(uid, iced::widget::image::Handle::from_bytes(bytes));
                }
                Err(e) => log::warn!("[update] Comment pfp failed: {}", e),
            }
            iced::Task::none()
        }

        Message::CommentStampLoaded(cid, result) => {
            match result {
                Ok(bytes) => {
                    state
                        .viewer
                        .comment_stamps
                        .insert(cid, iced::widget::image::Handle::from_bytes(bytes));
                }
                Err(e) => log::warn!("[update] Comment stamp failed: {}", e),
            }
            iced::Task::none()
        }

        Message::UserDetailFetched(result) => {
            state.artist.loading = false;
            match result {
                Ok(detail) => {
                    state.artist.user = Some(detail.user.clone());
                    // Fetch the profile picture (cached) once we know its URL.
                    if let Some(url) = detail
                        .user
                        .profile_image_urls
                        .as_ref()
                        .and_then(|u| u.medium.clone())
                    {
                        let cache = state.images.image_cache.clone();
                        let fetcher = state.images.fetcher.clone();
                        return iced::Task::perform(
                            fetch_pfp(url, cache, fetcher, crate::media::Provider::Pixiv),
                            Message::ArtistPfpLoaded,
                        );
                    }
                }
                Err(e) => {
                    log::error!("[viewer] Failed to load user: {}", e);
                    return iced::Task::done(Message::Error(format!("Failed to load artist: {e}")));
                }
            }
            iced::Task::none()
        }

        Message::UserIllustsFetched(result) => {
            state.artist.loading = false;
            match result {
                Ok(data) => {
                    let new_illusts = data.illusts.clone();
                    state.artist.illusts.extend(data.illusts);
                    state.artist.next_url = data.next_url;
                    // Load thumbnails for the artist's illusts. Register the
                    // generation target FIRST — ThumbnailLoaded drops any
                    // result whose target isn't registered (the staleness
                    // guard), so spawning without registering lost EVERY
                    // artist-page thumbnail.
                    let target = crate::app::update::thumbnail_target(state);
                    for illust in &new_illusts {
                        state.images.thumbnail_targets.insert(illust.id, target);
                    }
                    let fetcher = state.images.fetcher.clone();
                    let tasks = spawn_thumbnail_fetches(
                        &new_illusts,
                        target,
                        state.settings.high_res_thumbnails,
                        state.images.image_cache.clone(),
                        state.images.thumbnail_cache.clone(),
                        fetcher,
                    );
                    iced::Task::batch(tasks)
                }
                Err(e) => {
                    log::error!("[viewer] Failed to load user illusts: {}", e);
                    iced::Task::done(Message::Error(format!(
                        "Failed to load artist illusts: {e}"
                    )))
                }
            }
        }

        Message::ThumbnailLoaded(id, url, target, result) => {
            let images = &mut state.images;
            images.thumbnail_in_flight.remove(&id);
            images.thumbnail_aborts.remove(&id);
            // Stale result: the grid size changed since this fetch started,
            // so the thumbnail was generated at the wrong resolution — drop
            // it (a fresh fetch at the new target is already queued).
            if images.thumbnail_targets.get(&id).copied() != Some(target) {
                return iced::Task::none();
            }
            match result {
                Ok(bytes) => {
                    log::debug!("[update] ThumbnailLoaded: {} ({} bytes)", id, bytes.len());
                    images.thumbnail_retries.remove(&id);
                    let handle = iced::widget::image::Handle::from_bytes(bytes.clone());
                    images.thumbnail_handles.insert(id, handle.clone());
                    // If the viewer is open on this illust and has nothing to
                    // show yet, promote the thumbnail so the user sees the
                    // best available image immediately.
                    if state.viewer.open
                        && state.viewer.illust_id == id
                        && state.viewer.handle.is_none()
                    {
                        state.viewer.handle = Some(handle);
                    }
                    // Extract a color palette (for the sRGB/Hue sorts) off the
                    // UI thread, unless we already have one.
                    let mut tasks: Vec<iced::Task<Message>> = vec![];
                    if !images.palettes.contains_key(&id) {
                        tasks.push(iced::Task::perform(
                            crate::app::update::extract_palette(id, bytes),
                            |m| m,
                        ));
                    }
                    tasks.extend(crate::app::update::spawn_thumbnail_workers(state));
                    return iced::Task::batch(tasks);
                }
                Err(e) => {
                    log::warn!("[update] ThumbnailLoaded error for {}: {}", id, e);
                    // Bounded retry: re-queue the failed (id, url) into the
                    // worker pool. A single failure never stops the pool —
                    // it just goes to the back of the queue — and the URL
                    // is only retried a few times before being dropped for
                    // the session (the next window report re-enqueues it).
                    let retries = images.thumbnail_retries.entry(id).or_insert(0);
                    *retries += 1;
                    if *retries <= crate::app::update::THUMBNAIL_MAX_RETRIES {
                        images.thumbnail_queue.push((id, url, target));
                    }
                    // Either way, drain whatever else is queued.
                }
            }
            iced::Task::batch(crate::app::update::spawn_thumbnail_workers(state))
        }

        Message::FullImageProgress(id, page, frac) => {
            // Only track progress for the page currently being displayed.
            if state.viewer.illust_id == id && state.viewer.page == page {
                state.viewer.progress = Some(frac);
            }
            iced::Task::none()
        }

        Message::FullImagePartial(id, page, handle) => {
            // Show a progressively-decoded portion of a still-downloading
            // image (progressive JPEG). The final FullImageLoaded replaces it.
            if state.viewer.illust_id == id && state.viewer.page == page {
                state.viewer.handle = Some(handle);
            }
            iced::Task::none()
        }

        Message::FullImageLoaded(id, page, result) => {
            state.viewer.pages_loading.remove(&page);
            state.viewer.page_aborts.remove(&page);
            match result {
                Ok(bytes) => {
                    log::debug!(
                        "[update] FullImageLoaded: {} p{} ({} bytes)",
                        id,
                        page,
                        bytes.len()
                    );
                    // Only update the viewer if this page is the one shown
                    // (a stale page fetch must not overwrite the current one).
                    if state.viewer.illust_id == id {
                        let handle = iced::widget::image::Handle::from_bytes(bytes);
                        state.viewer.page_handles.insert(page, handle.clone());
                        if state.viewer.page == page {
                            state.viewer.handle = Some(handle);
                            state.viewer.progress = None;
                            // Now that the current page is ready, preload the
                            // next few after a short settle period.
                            let settle = iced::Task::perform(
                                async {
                                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                                },
                                |_| Message::ViewerPageSettled,
                            );
                            return settle;
                        }
                    }
                }
                Err(e) => {
                    log::warn!("[update] FullImageLoaded error for {}: {}", id, e);
                    if state.viewer.illust_id == id && state.viewer.page == page {
                        // Keep showing the cached image, just stop the bar.
                        state.viewer.progress = None;
                    }
                }
            }
            iced::Task::none()
        }

        // ── Gallery interactions ───────────────────────────────────
        Message::GalleryItemClicked(id) => {
            // Shift-click toggles the bookmark (and its auto-download); a
            // plain click opens the viewer.
            if state.shift_pressed {
                log::info!("[update] Shift-click bookmark toggle: {}", id);
                return super::jobs::handle(state, Message::BookmarkToggled(id));
            }
            log::info!("[update] GalleryItemClicked: {}", id);
            open_viewer(state, id)
        }

        Message::GalleryItemHover(id, entered) => {
            if entered {
                state.hovered_tiles.insert(id);
            } else {
                state.hovered_tiles.remove(&id);
            }
            iced::Task::none()
        }

        Message::GalleryItemDoubleClicked(id) => {
            // Double-click opens the viewer and fetches illust detail + comments
            open_viewer(state, id)
        }

        // ── Viewer ─────────────────────────────────────────────────
        Message::ViewerOpened(id) => open_viewer(state, id),

        Message::ViewerClosed => {
            state.viewer.close();
            iced::Task::none()
        }

        Message::EscapePressed => {
            // Close the topmost overlay:
            // shortcuts → help → viewer → artist → settings → downloads.
            if state.show_shortcuts {
                state.show_shortcuts = false;
            } else if state.show_help {
                state.show_help = false;
            } else if state.viewer.open {
                state.viewer.close();
            } else if state.artist.open {
                state.artist.close();
            } else if state.settings_open {
                state.settings_open = false;
            } else if state.download_manager_open {
                state.download_manager_open = false;
            }
            iced::Task::none()
        }

        Message::ViewerPageChanged(page) => goto_page(state, page),

        Message::ViewerPageStep(delta) => {
            // Browser back/forward style: step by ±1 page (multi-page illusts).
            let total = state.viewer.total_pages.max(1);
            let current = state.viewer.page as i64;
            let next = (current + delta as i64).clamp(0, total as i64 - 1) as usize;
            if next != state.viewer.page {
                goto_page(state, next)
            } else {
                iced::Task::none()
            }
        }

        Message::ViewerPageNavigate(nav) => {
            let total = state.viewer.total_pages.max(1);
            let page = match nav {
                crate::app::state::ViewerPageNav::First => 0,
                crate::app::state::ViewerPageNav::Prev => state.viewer.page.saturating_sub(1),
                crate::app::state::ViewerPageNav::Next => (state.viewer.page + 1).min(total - 1),
                crate::app::state::ViewerPageNav::Last => total - 1,
            };
            if page != state.viewer.page {
                goto_page(state, page)
            } else {
                iced::Task::none()
            }
        }

        Message::ViewerPageInputChanged(text) => {
            // Keep only digits so the box always holds a parseable page number.
            let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
            state.viewer.page_input = digits;
            iced::Task::none()
        }

        Message::ViewerPageSubmitted => {
            let total = state.viewer.total_pages.max(1);
            let parsed = state
                .viewer
                .page_input
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1)) // the box is 1-based
                .filter(|&p| p < total);
            if let Some(page) = parsed {
                state.viewer.page_input.clear();
                goto_page(state, page)
            } else {
                iced::Task::none()
            }
        }

        Message::ViewerPageSettled => preload_viewer_pages(state),

        // ── Window ─────────────────────────────────────────────────
        Message::ArtistOpened(user_id) => {
            // Navigating to the author CLOSES the detail view — the artist
            // layer is stacked UNDER the viewer, so leaving it open made
            // the click appear to do nothing until Esc closed the viewer.
            state.viewer.open = false;
            state.artist.open = true;
            state.artist.id = user_id;
            state.artist.provider = crate::media::provider_of(user_id);
            state.artist.illusts.clear();
            state.artist.user = None;
            state.artist.loading = true;
            state.artist.next_url = None;
            state.artist.pfp = None;

            match state.artist.provider {
                crate::media::Provider::Pixiv => {
                    if let Some(client) = state.client.clone() {
                        let detail_task = iced::Task::perform(
                            fetch_user_detail(client.clone(), user_id),
                            Message::UserDetailFetched,
                        );
                        let illusts_task = iced::Task::perform(
                            fetch_user_illusts(client, user_id, None),
                            Message::UserIllustsFetched,
                        );
                        return iced::Task::batch([detail_task, illusts_task]);
                    }
                    iced::Task::none()
                }
                crate::media::Provider::Twitter => {
                    let Some(client) = state.twitter.client.clone() else {
                        state.artist.loading = false;
                        return iced::Task::none();
                    };
                    let native = crate::media::native_id(user_id);
                    iced::Task::perform(
                        super::twitter::fetch_user_tweets(client, native, None),
                        move |r| Message::TwitterArtistLoaded(user_id, r),
                    )
                }
                crate::media::Provider::Bilibili => {
                    let Some(client) = state.bilibili.client.clone() else {
                        state.artist.loading = false;
                        return iced::Task::none();
                    };
                    let native = crate::media::native_id(user_id);
                    iced::Task::perform(
                        super::bilibili::fetch_user(client, native, None),
                        move |r| Message::BilibiliArtistLoaded(user_id, r),
                    )
                }
            }
        }

        Message::ArtistPfpLoaded(result) => {
            match result {
                Ok(bytes) => {
                    let handle = iced::widget::image::Handle::from_bytes(bytes);
                    state.artist.pfp = Some(handle.clone());
                    // The same fetch is used for the detail view's author row.
                    state.viewer.author_pfp = Some(handle);
                }
                Err(e) => log::warn!("[update] Artist pfp failed: {}", e),
            }
            iced::Task::none()
        }

        // ── Pagination: artist page + comments ─────────────────────
        Message::ArtistLoadMore => {
            // Infinite scroll: fetch the author's next page when one exists.
            if state.artist.loading {
                return iced::Task::none();
            }
            let Some(next_url) = state.artist.next_url.clone() else {
                return iced::Task::none();
            };
            let user_id = state.artist.id;
            match state.artist.provider {
                crate::media::Provider::Pixiv => {
                    let Some(client) = state.client.clone() else {
                        return iced::Task::none();
                    };
                    state.artist.loading = true;
                    iced::Task::perform(
                        fetch_user_illusts(client, user_id, Some(next_url)),
                        Message::UserIllustsFetched,
                    )
                }
                crate::media::Provider::Twitter => {
                    let Some(client) = state.twitter.client.clone() else {
                        return iced::Task::none();
                    };
                    let native = crate::media::native_id(user_id);
                    state.artist.loading = true;
                    iced::Task::perform(
                        super::twitter::fetch_user_tweets(client, native, Some(next_url)),
                        move |r| Message::TwitterArtistLoaded(user_id, r),
                    )
                }
                crate::media::Provider::Bilibili => {
                    let Some(client) = state.bilibili.client.clone() else {
                        return iced::Task::none();
                    };
                    let native = crate::media::native_id(user_id);
                    state.artist.loading = true;
                    iced::Task::perform(
                        super::bilibili::fetch_user(client, native, Some(next_url)),
                        move |r| Message::BilibiliArtistLoaded(user_id, r),
                    )
                }
            }
        }

        Message::CommentsLoadMore => {
            // Dedup: scroll events keep firing while a page is in flight;
            // without this guard the same cursor gets requested repeatedly.
            if state.viewer.comments_loading {
                return iced::Task::none();
            }
            match crate::media::provider_of(state.viewer.illust_id) {
                crate::media::Provider::Twitter => {
                    return super::twitter::comments_load_more(state);
                }
                crate::media::Provider::Bilibili => {
                    return super::bilibili::comments_load_more(state);
                }
                crate::media::Provider::Pixiv => {}
            }
            if let Some(client) = state.client.clone() {
                // Cursor pagination: fetch older comments from the saved
                // cursor (the last fetched comment id).
                if let Some(last) = state.viewer.comments.last().map(|c| c.id) {
                    state.viewer.comments_loading = true;
                    return iced::Task::perform(
                        fetch_comments(client, state.viewer.illust_id, Some(last)),
                        Message::CommentsPageFetched,
                    );
                }
            }
            iced::Task::none()
        }

        Message::CommentsPageFetched(result) => {
            state.viewer.comments_loading = false;
            match result {
                Ok(page) => {
                    state.viewer.comments_next_url = page.next_url.clone();
                    state.viewer.comments.extend(page.comments);
                }
                Err(e) => log::error!("[viewer] Failed to load more comments: {}", e),
            }
            iced::Task::none()
        }

        Message::ArtistClosed => {
            // Single sanctioned reset path — no hand-written field lists.
            state.artist.close();
            iced::Task::none()
        }

        // ── Browser ────────────────────────────────────────────────

        // Any message routed here by the dispatcher must be one of the above.
        _ => unreachable!("message routed to the wrong update handler"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg_with_sof(marker: u8) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8]; // SOI
        b.extend_from_slice(&[0xFF, marker, 0x00, 0x0C]); // SOF seg, len 12
        b.extend_from_slice(&[0x08, 0x04, 0x38, 0x03, 0x20]); // precision/dims
        b.extend_from_slice(&[0x01, 0x01, 0x00, 0x00]); // components
        b
    }

    #[test]
    fn progressive_jpeg_detected() {
        assert!(is_progressive_jpeg(&jpeg_with_sof(0xC2)));
    }

    #[test]
    fn baseline_jpeg_rejected() {
        assert!(!is_progressive_jpeg(&jpeg_with_sof(0xC0)));
        assert!(!is_progressive_jpeg(&jpeg_with_sof(0xC1)));
    }

    #[test]
    fn truncated_headers_rejected() {
        // SOI only — can't tell yet, conservatively not partial-capable.
        assert!(!is_progressive_jpeg(&[0xFF, 0xD8]));
        assert!(!is_progressive_jpeg(&[]));
    }

    #[test]
    fn non_jpeg_rejected() {
        assert!(!is_progressive_jpeg(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_progressive_jpeg(b"GIF89a"));
    }
}
