//! Bookmark/follow/download jobs and the bookmark-sync stream.

use super::gallery::{
    bookmark_add, fetch_illust_detail, find_illust_bookmarked, refresh_gallery_tags,
    toggle_bookmark_in_state,
};
use super::settings::schedule_queue_flush;
use super::{spawn_thumbnails_for_window, Outcome};
use crate::app::message::Message;
use crate::app::state::AppState;
use crate::io::library::{BookmarkLibrary, SyncJobState};
use pixivj_rs::api::PixivApiClient;
use pixivj_rs::error::PixivError;
use pixivj_rs::model::*;

/// Where a download resolves its media metadata and page bytes from.
///
/// Each variant owns its provider's behavior (which client resolves the item,
/// how a page is fetched), so the downloader itself has no provider branches.
enum DownloadOrigin {
    /// Resolved and downloaded through the Pixiv API client.
    Pixiv(Option<PixivApiClient>),
    /// Direct-CDN providers: the item may already be in memory (`seed`, e.g.
    /// the gallery/library copy) or is resolved through the provider client.
    Twitter {
        client: Option<crate::twitter::TwitterApiClient>,
        seed: Option<Illustration>,
    },
    Bilibili {
        client: Option<crate::bilibili::BilibiliApiClient>,
        seed: Option<Illustration>,
    },
}

impl DownloadOrigin {
    /// Pick the origin for `id`, seeding direct-CDN downloads with the
    /// gallery/library copy when one is available.
    fn for_id(state: &AppState, id: u64) -> Self {
        match crate::media::provider_of(id) {
            crate::media::Provider::Pixiv => Self::Pixiv(state.client.clone()),
            crate::media::Provider::Twitter => Self::Twitter {
                client: state.twitter.client.clone(),
                seed: find_illust_anywhere(state, id),
            },
            crate::media::Provider::Bilibili => Self::Bilibili {
                client: state.bilibili.client.clone(),
                seed: find_illust_anywhere(state, id),
            },
        }
    }

    fn provider(&self) -> crate::media::Provider {
        match self {
            Self::Pixiv(_) => crate::media::Provider::Pixiv,
            Self::Twitter { .. } => crate::media::Provider::Twitter,
            Self::Bilibili { .. } => crate::media::Provider::Bilibili,
        }
    }

    /// Whether pages come straight from the CDN (no Pixiv API client).
    fn is_direct(&self) -> bool {
        !matches!(self, Self::Pixiv(_))
    }

    /// Resolve the media item to download (consumes any in-memory seed).
    async fn resolve(&mut self, id: u64) -> Result<Illustration, PixivError> {
        match self {
            Self::Pixiv(client) => {
                let client = client.as_ref().ok_or_else(|| {
                    PixivError::Internal("Pixiv client unavailable for download".into())
                })?;
                Ok(client.get_illust_detail(id).await?.illust)
            }
            Self::Twitter { client, seed } => {
                if let Some(illust) = seed.take() {
                    return Ok(illust);
                }
                let client = client.as_ref().ok_or_else(|| {
                    PixivError::Internal("Twitter client unavailable for download".into())
                })?;
                client
                    .tweet(crate::media::native_id(id))
                    .await
                    .map_err(|e| PixivError::Internal(e.to_string()))
            }
            Self::Bilibili { client, seed } => {
                if let Some(illust) = seed.take() {
                    return Ok(illust);
                }
                let client = client.as_ref().ok_or_else(|| {
                    PixivError::Internal("Bilibili client unavailable for download".into())
                })?;
                client
                    .dynamic(crate::media::native_id(id))
                    .await
                    .map_err(|e| PixivError::Internal(e.to_string()))
            }
        }
    }

    /// Fetch one page's bytes. Pixiv pages go through the API client (which
    /// applies its own rate-limit gate); direct-CDN pages use the shared image
    /// client + limiter and the provider's headers.
    async fn fetch_page(
        &self,
        fetcher: &crate::app::state::ImageFetcher,
        url: &str,
    ) -> Result<reqwest::Response, PixivError> {
        match self {
            Self::Pixiv(client) => {
                let client = client.as_ref().ok_or_else(|| {
                    PixivError::Internal("Pixiv client unavailable for download".into())
                })?;
                client.download(url).await
            }
            Self::Twitter { .. } | Self::Bilibili { .. } => {
                let _gate = fetcher.limiter.wait_if_limited().await;
                let resp = super::image_request(fetcher, url, self.provider())
                    .send()
                    .await
                    .map_err(|e| PixivError::Internal(e.to_string()))?;
                fetcher.limiter.on_request_done(
                    resp.status().as_u16() == 429,
                    pixivj_rs::rate_limit::retry_after_header(&resp),
                );
                if !resp.status().is_success() {
                    return Err(PixivError::HttpStatus(
                        resp.status().as_u16(),
                        "media fetch failed".into(),
                    ));
                }
                Ok(resp)
            }
        }
    }
}

fn download_task(
    origin: DownloadOrigin,
    fetcher: crate::app::state::ImageFetcher,
    id: u64,
    gallery_path: std::path::PathBuf,
) -> iced::Task<Message> {
    iced::Task::run(
        IllustDownloader::new(origin, fetcher, id, gallery_path).into_stream(),
        |m| m,
    )
}

/// Incremental illust downloader (multi-page). Drives itself one message at a
/// time so the UI thread never blocks on I/O. All provider-specific behavior
/// lives in [`DownloadOrigin`].
struct IllustDownloader {
    origin: DownloadOrigin,
    fetcher: crate::app::state::ImageFetcher,
    id: u64,
    gallery_dir: std::path::PathBuf,
    urls: Vec<String>,
    /// Lower-quality alternatives used when an original URL 404s.
    fallback_urls: Vec<String>,
    index: usize,
    phase: Option<DownloadPagePhase>,
    done: bool,
    /// When the current page's download started (for speed).
    page_start: std::time::Instant,
    /// Bytes received on the current page (for speed).
    page_bytes: u64,
}

enum DownloadPagePhase {
    Fetch {
        response: reqwest::Response,
        received: u64,
        total: Option<u64>,
        buffer: Vec<u8>,
    },
}

impl IllustDownloader {
    fn new(
        origin: DownloadOrigin,
        fetcher: crate::app::state::ImageFetcher,
        id: u64,
        gallery_path: std::path::PathBuf,
    ) -> Self {
        Self {
            origin,
            fetcher,
            id,
            gallery_dir: gallery_path,
            urls: Vec::new(),
            fallback_urls: Vec::new(),
            index: 0,
            phase: None,
            done: false,
            page_start: std::time::Instant::now(),
            page_bytes: 0,
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

        // Step 0: resolve the media item (Pixiv detail / tweet / dynamic) →
        // collect download URLs, create the target dir.
        if self.urls.is_empty() && self.phase.is_none() {
            let illust: Illustration = match self.origin.resolve(self.id).await {
                Ok(illust) => illust,
                Err(e) => {
                    self.done = true;
                    return Some(Message::DownloadCompleted(self.id, Err(e)));
                }
            };
            let title = illust.title().to_string();
            let urls: Vec<String> = crate::core::collect_download_urls(&illust);
            if urls.is_empty() {
                self.done = true;
                return Some(Message::DownloadCompleted(
                    self.id,
                    Err(PixivError::NoImageUrl),
                ));
            }
            self.fallback_urls = illust
                .meta_pages
                .iter()
                .map(|p| p.image_urls.large.clone().unwrap_or_default())
                .collect();
            // Migrate an old `{id}_{title}` dir to the new `{id}` layout if
            // present, then create the (possibly migrated) download dir.
            let dir = crate::core::migrate_download_dir(&self.gallery_dir, self.id)
                .unwrap_or_else(|| crate::core::download_dir(&self.gallery_dir, self.id, &title));
            let dir_for_fs = dir.clone();
            match tokio::task::spawn_blocking(move || std::fs::create_dir_all(&dir_for_fs)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    self.done = true;
                    return Some(Message::DownloadCompleted(
                        self.id,
                        Err(PixivError::Io(std::sync::Arc::new(e))),
                    ));
                }
                Err(e) => {
                    self.done = true;
                    return Some(Message::DownloadCompleted(
                        self.id,
                        Err(PixivError::Internal(format!("create dir join: {e}"))),
                    ));
                }
            }
            self.urls = urls;
            self.gallery_dir = dir;
            return Some(Message::DownloadProgress(self.id, 0.0, 0.0));
        }

        let total_pages = self.urls.len().max(1) as f32;

        // Start the next page's download if we're between pages.
        if self.phase.is_none() {
            if self.index >= self.urls.len() {
                self.done = true;
                return Some(Message::DownloadCompleted(self.id, Ok(())));
            }
            let url = self.urls[self.index].clone();
            let mut resp = match self.origin.fetch_page(&self.fetcher, &url).await {
                Ok(r) => r,
                Err(e) => {
                    // Direct-CDN originals occasionally 404: fall back to the
                    // `large` variant before giving up on the page.
                    if self.origin.is_direct() {
                        let fallback = self
                            .fallback_urls
                            .get(self.index)
                            .filter(|u| !u.is_empty() && **u != url)
                            .cloned();
                        match fallback {
                            Some(alt) => match self.origin.fetch_page(&self.fetcher, &alt).await {
                                Ok(r) => r,
                                Err(e) => {
                                    self.done = true;
                                    return Some(Message::DownloadCompleted(self.id, Err(e)));
                                }
                            },
                            None => {
                                self.done = true;
                                return Some(Message::DownloadCompleted(self.id, Err(e)));
                            }
                        }
                    } else {
                        self.done = true;
                        return Some(Message::DownloadCompleted(self.id, Err(e)));
                    }
                }
            };
            if !resp.status().is_success() {
                if let Some(alt) = self
                    .fallback_urls
                    .get(self.index)
                    .filter(|u| !u.is_empty() && **u != url)
                {
                    if let Ok(fallback_resp) = self.origin.fetch_page(&self.fetcher, alt).await {
                        if fallback_resp.status().is_success() {
                            resp = fallback_resp;
                        }
                    }
                }
            }
            let total = resp.content_length();

            // Content-size accuracy check: if this page already exists on
            // disk with exactly the remote file's size, keep the local copy
            // and skip the transfer. Re-downloading an owned illust (e.g.
            // via bookmarking or Download-all) then costs one header
            // round-trip per page instead of full transfers.
            if let Some(remote_len) = total.filter(|t| *t > 0) {
                let dir = self.gallery_dir.clone();
                let id = self.id;
                let page = self.index;
                let local_len = tokio::task::spawn_blocking(move || {
                    crate::core::find_page_file(&dir, id, page)
                        .and_then(|p| p.metadata().ok().map(|m| m.len()))
                })
                .await
                .ok()
                .flatten();
                if local_len == Some(remote_len) {
                    log::info!(
                        "[download] {} p{} already on disk at remote size ({} B) — skipped",
                        id,
                        page,
                        remote_len
                    );
                    self.index += 1;
                    let overall = self.index as f32 / total_pages;
                    return Some(Message::DownloadProgress(self.id, overall, 0.0));
                }
            }

            self.page_start = std::time::Instant::now();
            self.page_bytes = 0;
            self.phase = Some(DownloadPagePhase::Fetch {
                response: resp,
                received: 0,
                total,
                buffer: Vec::new(),
            });
        }

        // Consume chunks from the current page, reporting progress.
        loop {
            let outcome = {
                let DownloadPagePhase::Fetch {
                    response,
                    received,
                    total,
                    buffer,
                } = self.phase.as_mut()?;
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        *received += chunk.len() as u64;
                        buffer.extend_from_slice(&chunk);
                        self.page_bytes += chunk.len() as u64;
                        match total {
                            Some(total) if *total > 0 => {
                                let page_frac = (*received as f64 / *total as f64) as f32;
                                let overall =
                                    (self.index as f32 + page_frac.min(1.0)) / total_pages;
                                Outcome::Progress(overall)
                            }
                            _ => Outcome::Continue,
                        }
                    }
                    Ok(None) => {
                        let bytes = std::mem::take(buffer);
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
                    let elapsed = self.page_start.elapsed().as_secs_f64().max(0.001);
                    let speed = self.page_bytes as f64 / elapsed;
                    return Some(Message::DownloadProgress(self.id, frac, speed));
                }
                Outcome::Continue => {}
                Outcome::Finished(result) => {
                    self.phase = None;
                    match result {
                        Ok(bytes) => {
                            let ext = self
                                .urls
                                .get(self.index)
                                .map(|u| crate::core::file_extension(u))
                                .unwrap_or("jpg");
                            let filename = crate::core::image_filename(
                                self.id,
                                self.index,
                                self.urls.len(),
                                ext,
                            );
                            let path = self.gallery_dir.join(&filename);
                            let path_for_fs = path.clone();
                            match tokio::task::spawn_blocking(move || {
                                std::fs::write(&path_for_fs, &bytes)
                            })
                            .await
                            {
                                Ok(Ok(())) => {
                                    log::info!("[download] Saved {:?}", path);
                                    self.index += 1;
                                    let overall = self.index as f32 / total_pages;
                                    let elapsed =
                                        self.page_start.elapsed().as_secs_f64().max(0.001);
                                    let speed = self.page_bytes as f64 / elapsed;
                                    return Some(Message::DownloadProgress(
                                        self.id, overall, speed,
                                    ));
                                }
                                Ok(Err(e)) => {
                                    self.done = true;
                                    return Some(Message::DownloadCompleted(
                                        self.id,
                                        Err(PixivError::HttpStatus(
                                            0,
                                            format!("write {:?}: {}", path, e),
                                        )),
                                    ));
                                }
                                Err(e) => {
                                    self.done = true;
                                    return Some(Message::DownloadCompleted(
                                        self.id,
                                        Err(PixivError::HttpStatus(
                                            0,
                                            format!("write join: {}", e),
                                        )),
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            self.done = true;
                            return Some(Message::DownloadCompleted(self.id, Err(e)));
                        }
                    }
                }
            }
        }
    }
}

async fn bookmark_remove(client: PixivApiClient, id: u64) -> Result<(), PixivError> {
    client.remove_bookmark(id).await
}

async fn follow_user(client: PixivApiClient, user_id: u64) -> Result<(), PixivError> {
    client.follow_user(&FollowUserRequest::new(user_id)).await
}

async fn unfollow_user(client: PixivApiClient, user_id: u64) -> Result<(), PixivError> {
    client.unfollow_user(&FollowUserRequest::new(user_id)).await
}

// ── Main update function ───────────────────────────────────────────────

/// Enqueue a single download and start workers for any free slot. No-op when
/// the illust is already in flight.
pub(super) fn start_download(state: &mut AppState, id: u64) -> iced::Task<Message> {
    if !state.jobs.downloads.start(id) {
        log::info!("[update] DownloadStarted: already downloading {}", id);
        return iced::Task::none();
    }
    // Pixiv downloads need the API client; Twitter/Bilibili download from
    // stored URLs.
    if state.client.is_none() && crate::media::is_pixiv(id) {
        state.jobs.downloads.complete(id);
        log::error!("[update] Download: not logged in");
        return iced::Task::none();
    }
    // Add to the shared, persistent queue (in-memory; flushed by the
    // debounced writer task).
    let newly_queued = state.jobs.download_queue.enqueue(id);
    if newly_queued {
        state.jobs.queue_dirty = true;
        state.jobs.download_job.running = true;
        state.jobs.download_job.error = None;
    }
    iced::Task::batch([schedule_queue_flush(), spawn_download_workers(state)])
}

/// Sequential, rate-limit-friendly bookmark library sync.
///
/// Two phases, so both new AND old bookmarks are always collected:
/// * **Catch-up**: paginate from the newest bookmark, merging into the
///   library, and stop as soon as a whole page is already known.
/// * **Deepen**: continue from the saved `cursor` (the deepest point the last
///   run reached) so an interrupted previous sync keeps pulling older
///   bookmarks instead of stopping early.
///
/// After the Pixiv half, the same catch-up/deepen walk runs over the account's
/// Twitter likes (when a Twitter client is available), merged into the same
/// library. The Twitter cursor is persisted as `library.twitter_cursor`.
///
/// `full == true` rebuilds the whole library (drops removed bookmarks) after
/// both halves complete.
///
/// The library + cursor are written atomically after every page, so a crash
/// mid-sync never corrupts it and the next run resumes gracefully. The total
/// bookmark count (for the progress bar) comes from the user profile.
pub(super) fn bookmark_sync_task(
    client: PixivApiClient,
    twitter: Option<crate::twitter::TwitterApiClient>,
    library: BookmarkLibrary,
    library_path: std::path::PathBuf,
    full: bool,
    cancel: crate::app::state::CancelFlag,
) -> iced::Task<Message> {
    iced::Task::run(
        iced::futures::stream::unfold(
            BookmarkSyncStream {
                client,
                twitter,
                library,
                library_path,
                full,
                phase: SyncPhase::Profile,
                next_url: None,
                total: None,
                user_id: None,
                first_full_page: true,
                twitter_next: None,
                twitter_deepening: false,
                rate_limit_streak: 0,
                finished: false,
                cancel,
            },
            |mut this| async move { this.next().await.map(|msg| (msg, this)) },
        ),
        |m| m,
    )
}

/// Which part of the sync we're in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncPhase {
    /// Fetch the user profile to learn the total bookmark count.
    Profile,
    /// Pull new bookmarks from the top until a known page.
    CatchUp,
    /// Continue from the saved cursor to reach older bookmarks.
    Deepen,
    /// Full sync: pull everything, rebuild at the end.
    Full,
    /// Incremental Twitter likes (catch-up, then deepen from the saved cursor).
    Twitter,
    /// Full Twitter likes sweep (rebuild after this).
    TwitterFull,
}

/// Sequential sync driver. Pulls one page per step with a delay between
/// pages (stays under Pixiv's rate limits).
struct BookmarkSyncStream {
    client: PixivApiClient,
    /// Present when Twitter is connected; adds the likes half of the sync.
    twitter: Option<crate::twitter::TwitterApiClient>,
    /// In-memory mirror, loaded once by the caller (never re-parsed per
    /// page). Written atomically after every page so a crash can't corrupt it.
    library: BookmarkLibrary,
    library_path: std::path::PathBuf,
    full: bool,
    phase: SyncPhase,
    next_url: Option<String>,
    total: Option<usize>,
    user_id: Option<u64>,
    first_full_page: bool,
    /// Cursor for the active Twitter phase (`None` = start from the top).
    twitter_next: Option<String>,
    /// Whether the Twitter half is walking older likes from the saved cursor.
    twitter_deepening: bool,
    finished: bool,
    /// Consecutive rate-limit retries: if the API keeps 429ing (e.g. the
    /// download-all job is hammering it), the sync would spin forever — cap
    /// it so the progress bar always disappears.
    rate_limit_streak: u32,
    /// Set by the UI's Cancel button; checked between steps so a sync stops
    /// cleanly at the next page boundary.
    cancel: crate::app::state::CancelFlag,
}

impl BookmarkSyncStream {
    async fn next(&mut self) -> Option<Message> {
        if self.finished {
            return None;
        }
        if self.cancel.is_cancelled() {
            log::info!("[jobs] Bookmark sync cancelled by user");
            self.finished = true;
            return Some(Message::BookmarkSyncFinished {
                total: self.total.unwrap_or_default(),
                error: None,
                library: self.library.clone(),
            });
        }

        // First: ensure the token refresh (captures the user id) and learn
        // the total bookmark count from the user profile. The phase stays at
        // Profile until the user id is actually known — otherwise a
        // rate-limited refresh would advance us past it and the bookmarks
        // request would go out without a user_id (400 Invalid request).
        if self.phase == SyncPhase::Profile {
            match self.client.user_id_ensured().await {
                Ok(uid) => {
                    self.user_id = Some(uid);
                    self.phase = if self.full {
                        SyncPhase::Full
                    } else {
                        SyncPhase::CatchUp
                    };
                    if let Ok(detail) = self.client.get_user_details(uid).await {
                        self.total = detail
                            .profile
                            .total_illust_bookmarks_public
                            .map(|t| t as usize);
                    }
                }
                Err(e) if is_rate_limit(&e) => {
                    // The retried request (user_id_ensured → get_token →
                    // get_bookmarks) goes through the client's shared gate
                    // (rate_limit.rs), which waits out the window and lets
                    // only one request through at a time — no pre-sleep here.
                    let _ = &e;
                    return Some(Message::BookmarkSyncProgress {
                        loaded: self.library.illusts.len(),
                        total: self.total,
                        new: Vec::new(),
                    });
                }
                Err(e) => {
                    self.finished = true;
                    return Some(Message::BookmarkSyncFinished {
                        total: self.total.unwrap_or_default(),
                        error: Some(format!("{}", e)),
                        library: self.library.clone(),
                    });
                }
            }
        }

        // Twitter likes phase (after the Pixiv half).
        if matches!(self.phase, SyncPhase::Twitter | SyncPhase::TwitterFull) {
            return self.next_twitter().await;
        }

        // Determine the next URL for the current phase.
        let (url, is_first_of_phase) = match self.phase {
            SyncPhase::CatchUp => {
                if self.next_url.is_none() {
                    (None, true)
                } else {
                    (self.next_url.clone(), false)
                }
            }
            SyncPhase::Deepen => {
                match self.library.cursor.clone() {
                    Some(cursor) if self.next_url.is_none() => (Some(cursor), true),
                    Some(_) => (self.next_url.clone(), false),
                    None => {
                        // Cursor is None → everything is already synced.
                        if let Some(msg) = self.enter_twitter_phase() {
                            return Some(msg);
                        }
                        return self.next_twitter().await;
                    }
                }
            }
            SyncPhase::Full => {
                if self.first_full_page {
                    self.first_full_page = false;
                    (None, true)
                } else {
                    (self.next_url.clone(), false)
                }
            }
            SyncPhase::Profile | SyncPhase::Twitter | SyncPhase::TwitterFull => unreachable!(),
        };

        // Fetch the page.
        let result = if is_first_of_phase && url.is_none() {
            let filter = V2Filter {
                user_id: self.user_id,
                restrict: Some(Restrict::Public),
                ..Default::default()
            };
            self.client.get_bookmarks(&filter).await
        } else {
            let Some(url) = url else {
                if let Some(msg) = self.enter_twitter_phase() {
                    return Some(msg);
                }
                return self.next_twitter().await;
            };
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            self.client.fetch_by_next_url(&url).await
        };

        match result {
            Ok(data) => {
                self.rate_limit_streak = 0;
                self.next_url = data.next_url;
                let new_illusts = data.illusts;

                if self.full {
                    let added = self.library.merge(new_illusts.clone());
                    // Completion is detected by reaching the end of the
                    // pagination (next_url None) OR by returning to bookmarks
                    // we already have (a whole page known + nothing new added)
                    // — that means we've captured every NEW bookmark and
                    // caught up to the previously-synced region. Without the
                    // all-known stop, a pagination that never returns None
                    // left the sync running forever.
                    let caught_up = self.library.all_known(&new_illusts) && added.is_empty();
                    if self.next_url.is_none() || caught_up {
                        if let Some(msg) = self.enter_twitter_phase() {
                            return Some(msg);
                        }
                        return self.next_twitter().await;
                    }
                    let _ = self.library.save_atomic(&self.library_path);
                    return Some(Message::BookmarkSyncProgress {
                        loaded: self.library.illusts.len(),
                        total: self.total,
                        new: added,
                    });
                }

                // Catch-up: merge and stop once we hit a page we fully know.
                if self.phase == SyncPhase::CatchUp {
                    let all_known = self.library.all_known(&new_illusts);
                    let added = self.library.merge(new_illusts.clone());
                    if self.next_url.is_none() {
                        self.library.cursor = None;
                    }
                    let _ = self.library.save_atomic(&self.library_path);
                    if self.next_url.is_none() {
                        if let Some(msg) = self.enter_twitter_phase() {
                            return Some(msg);
                        }
                        return self.next_twitter().await;
                    }
                    if all_known && added.is_empty() {
                        // Everything from here on is known: switch to deepen.
                        self.phase = SyncPhase::Deepen;
                        self.next_url = None;
                        return Some(Message::BookmarkSyncProgress {
                            loaded: self.library.illusts.len(),
                            total: self.total,
                            new: Vec::new(),
                        });
                    }
                    return Some(Message::BookmarkSyncProgress {
                        loaded: self.library.illusts.len(),
                        total: self.total,
                        new: added,
                    });
                }

                // Deepen: merge older pages; update the cursor each step.
                let added = self.library.merge(new_illusts.clone());
                self.library.cursor = self.next_url.clone();
                if self.next_url.is_none() {
                    self.library.cursor = None;
                }
                let _ = self.library.save_atomic(&self.library_path);
                if self.next_url.is_none() {
                    if let Some(msg) = self.enter_twitter_phase() {
                        return Some(msg);
                    }
                    return self.next_twitter().await;
                }
                Some(Message::BookmarkSyncProgress {
                    loaded: self.library.illusts.len(),
                    total: self.total,
                    new: added,
                })
            }
            Err(e) => {
                if is_rate_limit(&e) {
                    // Retry the same page on the next stream step; the
                    // client's shared gate waits out the window and fires
                    // one request at a time (rate_limit.rs). But a persistent
                    // 429 (e.g. a concurrent download-all) must not spin
                    // forever — give up so the progress bar disappears.
                    self.rate_limit_streak += 1;
                    if self.rate_limit_streak >= MAX_SYNC_RATE_LIMITS {
                        self.finished = true;
                        return Some(Message::BookmarkSyncFinished {
                            total: self.total.unwrap_or_default(),
                            error: Some("Sync aborted: repeatedly rate-limited".into()),
                            library: self.library.clone(),
                        });
                    }
                    return Some(Message::BookmarkSyncProgress {
                        loaded: self.library.illusts.len(),
                        total: self.total,
                        new: Vec::new(),
                    });
                }
                self.finished = true;
                Some(Message::BookmarkSyncFinished {
                    total: self.total.unwrap_or_default(),
                    error: Some(format!("{}", e)),
                    library: self.library.clone(),
                })
            }
        }
    }

    /// The terminal message; marks the stream finished so the progress bar
    /// always clears (ending silently left `bookmark_sync.running` stuck on).
    fn finish(&mut self) -> Message {
        self.finished = true;
        Message::BookmarkSyncFinished {
            total: self.total.unwrap_or_default(),
            error: None,
            library: self.library.clone(),
        }
    }

    /// Leave the Pixiv half of the sync: switch to the Twitter likes half when
    /// connected, otherwise finish (rebuilding the library first for a full
    /// run). `Some` = the stream is done; `None` = the Twitter phase is next.
    fn enter_twitter_phase(&mut self) -> Option<Message> {
        if self.twitter.is_some() {
            self.phase = if self.full {
                SyncPhase::TwitterFull
            } else {
                SyncPhase::Twitter
            };
            self.twitter_next = None;
            self.twitter_deepening = false;
            log::info!("[jobs] Bookmark sync: Pixiv half complete, syncing Twitter likes");
            return None;
        }
        if self.full {
            self.library.full_rebuild(self.library.illusts.clone());
            let _ = self.library.save_atomic(&self.library_path);
        }
        Some(self.finish())
    }

    /// One Twitter likes page. Mirrors the Pixiv catch-up/deepen logic on the
    /// likes cursor: walk from the newest until a fully-known page, then (if a
    /// previous run left a deeper cursor) continue older from there.
    async fn next_twitter(&mut self) -> Option<Message> {
        if self.finished {
            return None;
        }
        if self.cancel.is_cancelled() {
            log::info!("[jobs] Bookmark sync cancelled during Twitter phase");
            return Some(self.finish());
        }
        let Some(client) = self.twitter.clone() else {
            return Some(self.finish());
        };
        let cursor = self.twitter_next.clone();
        if cursor.is_some() {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        match client.likes(cursor.as_deref()).await {
            Ok(page) => {
                self.rate_limit_streak = 0;
                let new_illusts = page.items;
                let all_known = self.library.all_known(&new_illusts);
                let added = self.library.merge(new_illusts.clone());
                self.twitter_next = page.next_cursor.clone();

                if self.phase == SyncPhase::TwitterFull {
                    let caught_up = all_known && added.is_empty();
                    if self.twitter_next.is_none() || caught_up {
                        self.library.full_rebuild(self.library.illusts.clone());
                        let _ = self.library.save_atomic(&self.library_path);
                        return Some(self.finish());
                    }
                    let _ = self.library.save_atomic(&self.library_path);
                    return Some(Message::BookmarkSyncProgress {
                        loaded: self.library.illusts.len(),
                        total: self.total,
                        new: added,
                    });
                }

                // Catch-up reached known data: jump to the deeper cursor a
                // previous interrupted run saved, or finish.
                if all_known && added.is_empty() {
                    let saved = self.library.twitter_cursor.clone();
                    match saved {
                        Some(saved) if !self.twitter_deepening => {
                            self.twitter_next = Some(saved);
                            self.twitter_deepening = true;
                            return Some(Message::BookmarkSyncProgress {
                                loaded: self.library.illusts.len(),
                                total: self.total,
                                new: Vec::new(),
                            });
                        }
                        _ => {
                            self.library.twitter_cursor = None;
                            let _ = self.library.save_atomic(&self.library_path);
                            return Some(self.finish());
                        }
                    }
                }

                // Catch-up pages do NOT touch the saved deep cursor (that
                // would lose the deeper point); only the deepen walk advances
                // and clears it.
                if self.twitter_deepening {
                    self.library.twitter_cursor = self.twitter_next.clone();
                }
                if self.twitter_next.is_none() {
                    self.library.twitter_cursor = None;
                }
                let _ = self.library.save_atomic(&self.library_path);
                if self.twitter_next.is_none() {
                    return Some(self.finish());
                }
                Some(Message::BookmarkSyncProgress {
                    loaded: self.library.illusts.len(),
                    total: self.total,
                    new: added,
                })
            }
            Err(e) => self.handle_twitter_error(e),
        }
    }

    /// Rate-limit retry / failure handling for the Twitter half. A dead
    /// Twitter session is reported without discarding the Pixiv progress.
    fn handle_twitter_error(&mut self, e: crate::twitter::TwitterError) -> Option<Message> {
        if crate::twitter::error::is_rate_limit(&e) {
            self.rate_limit_streak += 1;
            if self.rate_limit_streak >= MAX_SYNC_RATE_LIMITS {
                return Some({
                    self.finished = true;
                    Message::BookmarkSyncFinished {
                        total: self.total.unwrap_or_default(),
                        error: Some("Sync aborted: repeatedly rate-limited".into()),
                        library: self.library.clone(),
                    }
                });
            }
            return Some(Message::BookmarkSyncProgress {
                loaded: self.library.illusts.len(),
                total: self.total,
                new: Vec::new(),
            });
        }
        log::warn!("[jobs] Twitter likes sync failed: {e}");
        self.finished = true;
        Some(Message::BookmarkSyncFinished {
            total: self.total.unwrap_or_default(),
            error: Some(format!("{e}")),
            library: self.library.clone(),
        })
    }
}

/// Whether an error is a Pixiv rate-limit response.
fn is_rate_limit(e: &PixivError) -> bool {
    pixivj_rs::rate_limit::is_rate_limit(e)
}

/// Whether an error means the illust was deleted/removed from Pixiv
/// (permanent — it will never download, so skip it in future runs instead of
/// re-attempting it every "download all").
fn is_permanently_deleted(e: &PixivError) -> bool {
    matches!(
        e,
        PixivError::HttpStatus(404, _) | PixivError::HttpStatus(410, _)
    )
}

/// Consecutive rate-limit retries allowed during a bookmark sync before it
/// gives up (so a persistent 429 can't leave the progress bar stuck on).
const MAX_SYNC_RATE_LIMITS: u32 = 12;

/// Spawn download workers until the concurrency limit is reached. Each worker
/// runs the streaming illust downloader (which reports per-item progress and
/// completion through the normal messages). In-flight ids are tracked in
/// memory but kept in the persisted queue until done, so a crash can only
/// re-process (never lose) an item.
pub(super) fn spawn_download_workers(state: &mut AppState) -> iced::Task<Message> {
    let pixiv = state.client.clone();
    let has_direct = state
        .jobs
        .download_queue
        .remaining
        .iter()
        .any(|id| !crate::media::is_pixiv(*id));
    if pixiv.is_none() && !has_direct {
        state.jobs.download_job.running = false;
        return iced::Task::none();
    }
    let concurrency = state.settings.download_concurrency.max(1);
    let queue = state.jobs.download_queue.clone();
    let gallery_path = state
        .settings
        .gallery_path
        .clone()
        .unwrap_or_else(crate::core::default_gallery_dir);

    let mut tasks: Vec<iced::Task<Message>> = Vec::new();
    let candidates: Vec<u64> = queue
        .remaining
        .iter()
        .copied()
        .filter(|id| !state.jobs.download_job.in_flight.contains(id))
        .collect();
    // Already-running workers count against the budget: without this, every
    // call site that spawns while others are in flight (bookmark toggle,
    // priority change, a second "download all") could exceed the limit.
    let free_slots = concurrency.saturating_sub(state.jobs.download_job.in_flight.len());
    for id in candidates.into_iter().take(free_slots) {
        state.jobs.download_job.in_flight.push(id);
        state.jobs.download_job.current = format!("#{}", crate::media::native_id(id));
        state.jobs.downloads.start(id);
        tasks.push(download_task(
            DownloadOrigin::for_id(state, id),
            state.images.fetcher.clone(),
            id,
            gallery_path.clone(),
        ));
    }
    state.jobs.download_queue = queue;
    // Persist the (unchanged) queue; in-flight ids remain so a crash resumes.
    state.jobs.queue_dirty = true;

    if state.jobs.download_job.in_flight.is_empty() && state.jobs.download_queue.is_finished() {
        state.jobs.download_job.running = false;
        state.jobs.download_job.current.clear();
    }
    iced::Task::batch(tasks)
}

/// Whether the user is followed, according to the first copy of them found in
/// the tabs, the artist page or the viewer.
fn find_user_followed(state: &AppState, user_id: u64) -> bool {
    for illust in state.tabs.iter().flat_map(|tab| tab.illustrations.iter()) {
        if let Some(user) = illust.user.as_ref().filter(|u| u.id == user_id) {
            return user.followed();
        }
    }
    if let Some(ref user) = state.artist.user {
        if user.id == user_id {
            return user.followed();
        }
    }
    if let Some(ref illust) = state.viewer.illust {
        if let Some(ref author) = illust.user {
            if author.id == user_id {
                return author.followed();
            }
        }
    }
    false
}

/// Toggle follow state for a user in all tabs.
fn toggle_follow_in_state(state: &mut AppState, user_id: u64) {
    for illust in state
        .tabs
        .iter_mut()
        .flat_map(|tab| tab.illustrations.iter_mut())
    {
        if let Some(user) = illust.user.as_mut().filter(|u| u.id == user_id) {
            user.is_followed = Some(!user.followed());
        }
    }
    // Update artist page
    if let Some(ref mut user) = state.artist.user {
        if user.id == user_id {
            user.is_followed = Some(!user.followed());
        }
    }
    // Update viewer
    if let Some(ref mut illust) = state.viewer.illust {
        if let Some(ref mut author) = illust.user {
            if author.id == user_id {
                author.is_followed = Some(!author.followed());
            }
        }
    }
}

/// Find an illust by id anywhere in the app state (viewer first, then all
/// tabs), for download-skip decisions.
pub(super) fn find_illust_anywhere(
    state: &AppState,
    id: u64,
) -> Option<pixivj_rs::model::Illustration> {
    if let Some(illust) = &state.viewer.illust {
        if illust.id == id {
            return Some(illust.clone());
        }
    }
    state
        .tabs
        .iter()
        .flat_map(|tab| tab.illustrations.iter())
        .find(|i| i.id == id)
        .cloned()
}

/// Whether bookmarking `id` should trigger its auto-download: only when the
/// illust is NOT already fully downloaded on disk.
pub(super) fn should_download_on_bookmark(state: &AppState, id: u64) -> bool {
    let gallery = state
        .settings
        .gallery_path
        .clone()
        .unwrap_or_else(crate::core::default_gallery_dir);
    match find_illust_anywhere(state, id) {
        Some(illust) => !crate::core::is_illust_downloaded(&gallery, &illust),
        // Unknown illust: attempt the download as before.
        None => true,
    }
}

/// Toggle a Bilibili item's bookmark. Bilibili has no liked-posts feed, so
/// the local library is the source of truth: bookmarks are added/removed from
/// it directly (and persisted through the debounced flush), while the remote
/// like is a best-effort side effect.
pub(super) fn handle(state: &mut AppState, message: Message) -> iced::Task<Message> {
    match message {
        Message::BookmarkToggled(id) => {
            // Each direct-CDN provider owns its bookmark/like semantics
            // (Bilibili mirrors into the local library, Twitter likes).
            match crate::media::provider_of(id) {
                crate::media::Provider::Bilibili => {
                    return super::bilibili::toggle_bookmark(state, id);
                }
                crate::media::Provider::Twitter => {
                    return super::twitter::toggle_bookmark(state, id);
                }
                crate::media::Provider::Pixiv => {}
            }
            if let Some(client) = state.client.clone() {
                // Find the illust in the current tab to determine current bookmark state
                let bookmarked = find_illust_bookmarked(state, id);
                let task = if bookmarked {
                    iced::Task::perform(bookmark_remove(client, id), move |result| {
                        Message::BookmarkResult(id, result)
                    })
                } else {
                    let add = iced::Task::perform(bookmark_add(client, id), move |result| {
                        Message::BookmarkResult(id, result)
                    });
                    // Bookmarking auto-triggers a download of the illust —
                    // unless it is already fully downloaded on disk. The
                    // queue-level worker also verifies per-page sizes, but
                    // skipping here avoids even the detail fetch + queueing.
                    if !should_download_on_bookmark(state, id) {
                        log::info!("[jobs] Bookmark: {id} already downloaded, not re-downloading");
                        return add;
                    }
                    let download = start_download(state, id);
                    return iced::Task::batch([add, download]);
                };
                return task;
            }
            iced::Task::none()
        }

        Message::TwitterLikeResult(id, result) => {
            match result {
                Ok(()) => {
                    toggle_bookmark_in_state(state, id);
                    log::info!("[jobs] Tweet {id} like state toggled");
                }
                Err(e) => {
                    log::error!("[jobs] Tweet like failed: {}", e);
                    return iced::Task::done(Message::Error(format!("Twitter like failed: {e}")));
                }
            }
            iced::Task::none()
        }

        Message::BookmarkResult(id, result) => {
            match result {
                Ok(()) => {
                    // Toggle bookmark state in all tabs
                    toggle_bookmark_in_state(state, id);
                    // Refetch detail to get the updated bookmark count back
                    // from the API (the button shows the real number).
                    if let Some(client) = state.client.clone() {
                        return iced::Task::perform(
                            fetch_illust_detail(client, id),
                            move |r| match r {
                                Ok(d) => Message::IllustDetailRefreshed(id, d),
                                Err(e) => Message::Error(format!("refresh illust {}: {}", id, e)),
                            },
                        );
                    }
                }
                Err(e) => {
                    log::error!("[jobs] Bookmark failed: {}", e);
                }
            }
            iced::Task::none()
        }

        Message::IllustDetailRefreshed(id, detail) => {
            // Update bookmark count/state in all tabs and the viewer.
            let is_bookmarked = detail.illust.is_bookmarked;
            let total_bookmarks = detail.illust.total_bookmarks;
            for illust in state
                .tabs
                .iter_mut()
                .flat_map(|tab| tab.illustrations.iter_mut())
                .filter(|i| i.id == id)
            {
                illust.is_bookmarked = is_bookmarked;
                illust.total_bookmarks = total_bookmarks;
            }
            if let Some(illust) = state.viewer.illust.as_mut() {
                if illust.id == id {
                    illust.is_bookmarked = is_bookmarked;
                    illust.total_bookmarks = total_bookmarks;
                }
            }
            iced::Task::none()
        }

        Message::FollowToggled(user_id) => {
            if let Some(client) = state.client.clone() {
                let followed = find_user_followed(state, user_id);
                let task = if followed {
                    iced::Task::perform(unfollow_user(client, user_id), move |result| {
                        Message::FollowResult(user_id, result)
                    })
                } else {
                    iced::Task::perform(follow_user(client, user_id), move |result| {
                        Message::FollowResult(user_id, result)
                    })
                };
                return task;
            }
            iced::Task::none()
        }

        Message::FollowResult(user_id, result) => {
            match result {
                Ok(()) => {
                    toggle_follow_in_state(state, user_id);
                }
                Err(e) => {
                    log::error!("[jobs] Follow failed: {}", e);
                }
            }
            iced::Task::none()
        }

        Message::DownloadStarted(id) => start_download(state, id),

        Message::StartBookmarkSync => {
            if state.jobs.bookmark_sync.running {
                return iced::Task::none();
            }
            let Some(client) = state.client.clone() else {
                return iced::Task::none();
            };
            state.jobs.bookmark_sync = SyncJobState {
                running: true,
                done: 0,
                total: None,
                current: "Starting full sync...".into(),
                error: None,
            };
            state.jobs.sync_cancel = crate::app::state::CancelFlag::default();
            bookmark_sync_task(
                client,
                state.twitter.client.clone(),
                state.jobs.library.clone(),
                state.jobs.library_path.clone(),
                true,
                state.jobs.sync_cancel.clone(),
            )
        }

        Message::CancelBookmarkSync => {
            // The stream notices the flag between pages and emits a normal
            // BookmarkSyncFinished (already-saved pages are kept).
            state.jobs.sync_cancel.cancel();
            iced::Task::none()
        }

        Message::CancelDownloadAll => {
            // Drop everything not yet started. In-flight downloads finish
            // naturally (no torn files); their completion handlers see the
            // empty queue and stop the job.
            let removed = state.jobs.download_queue.remaining.len();
            state.jobs.download_queue.remaining.clear();
            if removed > 0 {
                state.jobs.queue_dirty = true;
                log::info!("[jobs] Download all cancelled — dropped {removed} queued items");
            }
            // Nothing left to wait for — clear the job indicator now.
            // (Otherwise the last in-flight completion does it.)
            if state.jobs.download_job.in_flight.is_empty() {
                state.jobs.download_job.running = false;
                state.jobs.download_job.current.clear();
            }
            schedule_queue_flush()
        }

        Message::BookmarkSyncProgress { loaded, total, new } => {
            state.jobs.bookmark_sync.total = total;
            state.jobs.bookmark_sync.done = loaded;
            // Live gallery expansion: append the newly merged bookmarks so
            // the grid grows as pages are pulled.
            if !new.is_empty() {
                state.tabs.bookmarks.illustrations.extend(new);
                super::twitter::sort_by_date_desc(&mut state.tabs.bookmarks.illustrations);
                state.tabs.bookmarks.reapply(&state.images.palettes);
                refresh_gallery_tags(state);
            }
            // Lazy thumbnails: as the library grows, keep the visible window
            // loaded (never the whole 6000-item list at once).
            iced::Task::batch(spawn_thumbnails_for_window(state))
        }

        Message::BookmarkSyncFinished {
            total,
            error,
            library,
        } => {
            state.jobs.bookmark_sync.running = false;
            state.jobs.bookmark_sync.total = Some(total);
            state.jobs.bookmark_sync.error = error.clone();
            if let Some(err) = error {
                log::error!("[update] Bookmark sync finished with error: {}", err);
            } else {
                log::info!("[update] Bookmark sync finished ({} pages)", total);
            }
            // Adopt the stream's in-memory library (already persisted by the
            // stream itself) — no disk round-trip on the UI thread.
            state.jobs.library = library;
            state.tabs.bookmarks.illustrations = state.jobs.library.illusts.clone();
            // The library stores the two providers in separate blocks; the
            // mixed tab shows them interleaved chronologically.
            super::twitter::sort_by_date_desc(&mut state.tabs.bookmarks.illustrations);
            state.tabs.bookmarks.reapply(&state.images.palettes);
            refresh_gallery_tags(state);
            iced::Task::batch(spawn_thumbnails_for_window(state))
        }

        Message::StartDownloadAll => {
            if state.jobs.download_job.running {
                return iced::Task::none();
            }
            // The queue must cover ALL synced bookmarks, not just the ones
            // currently shown in the UI. If the library isn't populated yet,
            // run a full sync first.
            if state.jobs.library.illusts.is_empty() {
                log::info!("[update] Download all: library empty, syncing first");
                return iced::Task::done(Message::StartBookmarkSync);
            }
            // Start a FRESH job: reset the lifetime enqueue counter so the
            // progress reflects THIS "download all" run, not every historical
            // run (the old lifetime `total` kept accumulating and made it look
            // like thousands of extra "download items"). The permanently-failed
            // (deleted) set is kept so they stay skipped.
            state.jobs.download_queue.remaining.clear();
            state.jobs.download_queue.total = 0;
            // Skip illusts already downloaded to disk so repeated "download
            // all" runs never re-download existing files (the queue only
            // dedups against its own `remaining`, not against disk state).
            let gallery_path = state
                .settings
                .gallery_path
                .clone()
                .unwrap_or_else(crate::core::default_gallery_dir);
            let mut added = 0;
            for illust in &state.jobs.library.illusts {
                let id = illust.id;
                if crate::core::is_illust_downloaded(&gallery_path, illust) {
                    continue;
                }
                if state.jobs.download_queue.enqueue(id) {
                    added += 1;
                }
            }
            if added > 0 {
                state.jobs.queue_dirty = true;
                state.jobs.download_job.running = true;
                state.jobs.download_job.error = None;
                log::info!("[update] Download all: enqueued {} illusts", added);
            }
            iced::Task::batch([schedule_queue_flush(), spawn_download_workers(state)])
        }

        Message::ToggleDownloadManager => {
            state.download_manager_open = !state.download_manager_open;
            iced::Task::none()
        }

        Message::DownloadPriorityUp(id) => {
            if state.jobs.download_queue.move_up(id) {
                state.jobs.queue_dirty = true;
                // A slot may have freed — pick the newly-prioritized item now.
                let flush = schedule_queue_flush();
                let workers = spawn_download_workers(state);
                iced::Task::batch([flush, workers])
            } else {
                iced::Task::none()
            }
        }

        Message::DownloadPriorityDown(id) => {
            if state.jobs.download_queue.move_down(id) {
                state.jobs.queue_dirty = true;
                let flush = schedule_queue_flush();
                let workers = spawn_download_workers(state);
                iced::Task::batch([flush, workers])
            } else {
                iced::Task::none()
            }
        }

        Message::DownloadProgress(id, frac, speed) => {
            state.jobs.downloads.set_fraction(id, frac);
            if speed > 0.0 {
                state.jobs.download_speeds.insert(id, speed);
            }
            if state.jobs.download_job.running {
                state.jobs.download_job.current = format!("#{} ({:.0}%)", id, frac * 100.0);
            }
            iced::Task::none()
        }

        Message::DownloadCompleted(id, result) => {
            let retry = result.as_ref().is_err_and(is_rate_limit);
            state.jobs.downloads.complete(id);
            let mut tasks = Vec::new();
            match result {
                Ok(()) => {
                    // Keep the button disabled + show "Downloaded" on success.
                    state.jobs.downloads.mark_completed(id);
                    // The illust is now on disk — refresh the downloaded set
                    // so the Missing filter and tile checkmarks update.
                    tasks.push(iced::Task::done(Message::RefreshDownloaded));
                }
                Err(e) => {
                    // A deleted illust (HTTP 404/410) will never download — mark
                    // it permanently-failed so "download all" stops re-attempting
                    // it on every run, and skip the error banner (it's not a
                    // user-actionable failure).
                    if is_permanently_deleted(&e) {
                        log::info!(
                            "[jobs] #{} is deleted on Pixiv ({}); skipping in future runs",
                            id,
                            e
                        );
                        state.jobs.download_queue.mark_failed(id);
                        state.jobs.download_job.current.clear();
                    } else {
                        log::error!("[jobs] Download failed: {}", e);
                        tasks.push(iced::Task::done(Message::Error(format!(
                            "Download of #{} failed: {}",
                            id, e
                        ))));
                    }
                }
            }
            // Update the shared persistent queue and keep the worker pool at
            // the concurrency limit. Rate-limited (429) items are NOT marked
            // done: they stay in the queue and get retried after the shared
            // backoff window ends.
            state.jobs.download_speeds.remove(&id);
            state.jobs.download_job.in_flight.retain(|&x| x != id);
            if retry {
                log::warn!(
                    "[jobs] Download of #{} rate-limited; will retry after backoff",
                    id
                );
            } else {
                state.jobs.download_queue.mark_done(id);
            }
            let finished = state.jobs.download_queue.is_finished()
                && state.jobs.download_queue.remaining.is_empty();
            state.jobs.queue_dirty = true;
            let flush = schedule_queue_flush();
            if finished {
                state.jobs.download_job.running = false;
                state.jobs.download_job.current.clear();
                iced::Task::batch(tasks.into_iter().chain(std::iter::once(flush)))
            } else {
                tasks.push(flush);
                tasks.push(spawn_download_workers(state));
                iced::Task::batch(tasks)
            }
        }

        // Any message routed here by the dispatcher must be one of the above.
        _ => unreachable!("message routed to the wrong update handler"),
    }
}
