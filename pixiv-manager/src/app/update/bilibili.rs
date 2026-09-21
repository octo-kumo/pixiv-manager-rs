//! Bilibili feed reducers and async fetch helpers.
//!
//! The Bilibili tab is a plain [`crate::app::state::GalleryTab`] fed by the
//! followed-users feed (`t.bilibili.com`) or the configured fixed user list.
//! Bilibili has no liked-posts feed, so bookmarks are tracked locally in the
//! shared [`crate::io::library::BookmarkLibrary`] (see the `BookmarkToggled`
//! branch in [`super::jobs`]).

use super::gallery::{
    find_illust_bookmarked, maybe_auto_load, refresh_gallery_tags, set_bookmark_in_state,
};
use super::jobs::{find_illust_anywhere, should_download_on_bookmark, start_download};
use super::settings::{mark_config_dirty, schedule_config_flush, schedule_queue_flush};
use super::twitter::sort_by_date_desc;
use super::viewer::{apply_comments, author_pfp_url, full_image_or_cache, load_full_from_disk};
use super::{fetch_pfp, spawn_thumbnails_for_window};
use crate::app::message::Message;
use crate::app::state::{AppState, BilibiliStatus, Tab};
use crate::bilibili::{
    BilibiliApiClient, BilibiliError, BilibiliFeedMode, BilibiliPage, BilibiliProfile,
};
use crate::media::{native_id, Provider};
use pixivj_rs::model::{Comments, Illustration};
use std::path::PathBuf;

/// Payload of a successful client build.
#[derive(Debug, Clone)]
pub struct BilibiliBoot {
    pub client: BilibiliApiClient,
    pub profile: BilibiliProfile,
}

/// Where the cookies file comes from (explicit setting, then the conventional
/// locations). Test builds skip implicit locations so `cargo test` never
/// reads a developer's real cookies.
pub(super) fn resolve_cookie_path(state: &AppState) -> Option<PathBuf> {
    if let Some(path) = state
        .settings
        .bilibili_cookie_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        return Some(PathBuf::from(path));
    }
    #[cfg(not(feature = "test-util"))]
    {
        if let Some(config) = crate::io::config::config_path() {
            if let Some(dir) = config.parent() {
                let candidate = dir.join("bilibili_cookies.txt");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        let cwd = PathBuf::from("bilibili_cookies.txt");
        if cwd.is_file() {
            return Some(cwd);
        }
    }
    None
}

/// (Re)build the Bilibili client from the resolved cookies file and verify
/// the session in the background.
pub(super) fn start_client(state: &mut AppState) -> iced::Task<Message> {
    let Some(path) = resolve_cookie_path(state) else {
        state.bilibili.client = None;
        state.bilibili.status = BilibiliStatus::Disabled;
        return iced::Task::none();
    };
    state.bilibili.status = BilibiliStatus::Loading;
    let creds = match crate::bilibili::cookies::load_cookies_file(&path) {
        Ok(creds) => creds,
        Err(e) => {
            log::warn!("[bilibili] cookies load failed: {e}");
            state.bilibili.client = None;
            state.bilibili.status = BilibiliStatus::Error(e.to_string());
            return iced::Task::none();
        }
    };
    let client = match BilibiliApiClient::new(creds, state.settings.proxy.clone()) {
        Ok(client) => client,
        Err(e) => {
            state.bilibili.client = None;
            state.bilibili.status = BilibiliStatus::Error(e.to_string());
            return iced::Task::none();
        }
    };
    state.bilibili.client = Some(client.clone());
    iced::Task::perform(
        async move {
            match client.nav().await {
                Ok(profile) => Ok(BilibiliBoot { client, profile }),
                Err(e) => Err(e),
            }
        },
        Message::BilibiliClientBuilt,
    )
}

// ── Async fetches ──────────────────────────────────────────────────────

/// Paginate the Bilibili tab. Following mode follows the feed cursor
/// (`next_url` holds the page number); Users mode walks the configured fixed
/// list, a few users per LoadMore, re-arming each user until their feed ends.
pub(super) fn load_more(state: &mut AppState) -> iced::Task<Message> {
    let Some(client) = state.bilibili.client.clone() else {
        // The view shows the setup hint instead of a spinner.
        return iced::Task::none();
    };
    match state.bilibili.feed_mode {
        BilibiliFeedMode::Following => {
            let (loading, cursor, has_items) = {
                let t = &state.tabs.bilibili;
                (t.loading, t.next_url.clone(), !t.illustrations.is_empty())
            };
            if loading || (cursor.is_none() && has_items) {
                return iced::Task::none();
            }
            let page = cursor
                .as_deref()
                .and_then(|c| c.parse::<u32>().ok())
                .unwrap_or(1);
            state.tabs.bilibili.loading = true;
            iced::Task::perform(fetch_following(client, page), move |result| {
                Message::BilibiliFeedFetched(BilibiliFeedMode::Following, result)
            })
        }
        BilibiliFeedMode::Users => {
            // First load (or after a reset): arm every configured user.
            if state.bilibili.users_pending.is_empty()
                && state.bilibili.users_loading == 0
                && state.tabs.bilibili.illustrations.is_empty()
            {
                state.bilibili.users_pending = state
                    .settings
                    .bilibili_user_ids
                    .iter()
                    .map(|uid| (*uid, None))
                    .collect();
            }
            if state.bilibili.users_pending.is_empty() {
                return iced::Task::none();
            }
            const USERS_PER_LOAD: usize = 5;
            let take = USERS_PER_LOAD.min(state.bilibili.users_pending.len());
            let batch: Vec<(u64, Option<String>)> =
                state.bilibili.users_pending.drain(..take).collect();
            state.bilibili.users_loading += batch.len();
            state.tabs.bilibili.loading = true;
            let tasks: Vec<iced::Task<Message>> = batch
                .into_iter()
                .map(|(uid, offset)| {
                    iced::Task::perform(fetch_user(client.clone(), uid, offset), move |result| {
                        Message::BilibiliUserFetched(uid, result)
                    })
                })
                .collect();
            iced::Task::batch(tasks)
        }
    }
}

pub(super) async fn fetch_following(
    client: BilibiliApiClient,
    page: u32,
) -> Result<BilibiliPage, BilibiliError> {
    client.following_feed(page).await
}

pub(super) async fn fetch_user(
    client: BilibiliApiClient,
    uid: u64,
    offset: Option<String>,
) -> Result<BilibiliPage, BilibiliError> {
    client.space_feed(uid, offset.as_deref()).await
}

pub(super) async fn fetch_dynamic(
    client: BilibiliApiClient,
    id: u64,
) -> Result<Illustration, BilibiliError> {
    client.dynamic(crate::media::native_id(id)).await
}

/// Fetch a comment page. The comment thread id (`oid`) is carried on the item
/// (`provider_data.comment_oid`); when it is missing (rare: opened from an
/// old library entry) the dynamic is fetched once to recover it.
pub(super) async fn fetch_comments(
    client: BilibiliApiClient,
    id: u64,
    oid: Option<String>,
    next: u32,
) -> Result<Comments, BilibiliError> {
    let oid = match oid {
        Some(oid) if !oid.is_empty() => oid,
        _ => {
            let illust = client.dynamic(crate::media::native_id(id)).await?;
            illust
                .provider_data
                .and_then(|d| d.comment_oid)
                .ok_or_else(|| BilibiliError::Internal("dynamic has no comment id".into()))?
        }
    };
    client.comments(&oid, next).await
}

pub(super) async fn set_like(
    client: BilibiliApiClient,
    id: u64,
    liked: bool,
) -> Result<(), BilibiliError> {
    client.set_like(native_id(id), liked).await
}

// ── Reducer helpers (provider-owned) ───────────────────────────────────

/// Toggle a Bilibili item's bookmark. Bilibili has no liked-posts feed, so
/// the local library is the source of truth: bookmarks are added/removed from
/// it directly (and persisted through the debounced flush), while the remote
/// like is a best-effort side effect.
pub(super) fn toggle_bookmark(state: &mut AppState, id: u64) -> iced::Task<Message> {
    let was_bookmarked = find_illust_bookmarked(state, id);
    if was_bookmarked {
        state.jobs.library.remove(id);
        set_bookmark_in_state(state, id, false);
        // The mixed Bookmarks tab lists local bookmarks directly, so drop it
        // there too (the sync library keeps Pixiv/Twitter items until the
        // next full rebuild, but local ones are authoritative immediately).
        state.tabs.bookmarks.illustrations.retain(|i| i.id != id);
        state.tabs.bookmarks.reapply(&state.images.palettes);
        state.jobs.library_dirty = true;
        log::info!("[jobs] Bilibili bookmark removed: {id}");
    } else {
        let Some(illust) = find_illust_anywhere(state, id) else {
            log::warn!("[jobs] Bilibili bookmark: {id} not found in state");
            return iced::Task::none();
        };
        let mut item = illust;
        item.is_bookmarked = Some(true);
        state.jobs.library.merge(vec![item.clone()]);
        set_bookmark_in_state(state, id, true);
        state.tabs.bookmarks.illustrations.push(item);
        sort_by_date_desc(&mut state.tabs.bookmarks.illustrations);
        state.tabs.bookmarks.reapply(&state.images.palettes);
        state.jobs.library_dirty = true;
        log::info!("[jobs] Bilibili bookmark added: {id}");
    }
    // Best-effort remote like so the action shows on Bilibili as well; the
    // local bookmark stands even if this fails.
    let like_task = match state.bilibili.client.clone() {
        Some(client) => iced::Task::perform(set_like(client, id, !was_bookmarked), move |result| {
            Message::BilibiliLikeResult(id, result)
        }),
        None => iced::Task::none(),
    };
    let mut tasks = vec![schedule_queue_flush(), like_task];
    // Bookmarking auto-downloads the images, matching Pixiv/Twitter.
    if !was_bookmarked && should_download_on_bookmark(state, id) {
        tasks.push(start_download(state, id));
    }
    iced::Task::batch(tasks)
}

/// Open a Bilibili dynamic in the viewer: seed from live state (the gallery
/// item is the freshest copy), fall back to the disk cache then a by-id
/// fetch, and load comments (the reply thread id rides on the item).
pub(super) fn open_viewer(state: &mut AppState, id: u64) -> iced::Task<Message> {
    if let Some(comments) = crate::io::meta_cache::load_comments(id) {
        if !comments.is_empty() {
            state.viewer.comments = comments;
        }
    }

    let mut tasks: Vec<iced::Task<Message>> = Vec::new();
    let seeded = find_illust_anywhere(state, id).or_else(|| crate::io::meta_cache::load_detail(id));
    if let Some(illust) = seeded {
        state.viewer.loading = false;
        state.viewer.total_pages = illust.page_count();
        state.viewer.illust = Some(illust);
        if let Some(url) = state.viewer.illust.as_ref().and_then(author_pfp_url) {
            let cache = state.images.image_cache.clone();
            let fetcher = state.images.fetcher.clone();
            tasks.push(iced::Task::perform(
                fetch_pfp(url, cache, fetcher, Provider::Bilibili),
                Message::ArtistPfpLoaded,
            ));
        }
        if let Some(task) = load_full_from_disk(state, 0) {
            tasks.push(task);
        }
        tasks.push(full_image_or_cache(state));
    } else if let Some(client) = state.bilibili.client.clone() {
        tasks.push(iced::Task::perform(fetch_dynamic(client, id), move |r| {
            Message::BilibiliDynamicFetched(id, r)
        }));
    } else {
        state.viewer.loading = false;
    }

    // Comments (the first page; the cursor is the page number).
    if let Some(client) = state.bilibili.client.clone() {
        let oid = state
            .viewer
            .illust
            .as_ref()
            .and_then(|i| i.provider_data.as_ref())
            .and_then(|d| d.comment_oid.clone());
        tasks.push(iced::Task::perform(
            fetch_comments(client, id, oid, 1),
            Message::BilibiliCommentsFetched,
        ));
    }
    iced::Task::batch(tasks)
}

/// Fetch the next page of comments for the open dynamic.
pub(super) fn comments_load_more(state: &mut AppState) -> iced::Task<Message> {
    let Some(cursor) = state.viewer.comments_next_url.clone() else {
        return iced::Task::none();
    };
    let Some(client) = state.bilibili.client.clone() else {
        return iced::Task::none();
    };
    let next = cursor.parse::<u32>().unwrap_or(1);
    state.viewer.comments_loading = true;
    let id = state.viewer.illust_id;
    let oid = state
        .viewer
        .illust
        .as_ref()
        .and_then(|i| i.provider_data.as_ref())
        .and_then(|d| d.comment_oid.clone());
    iced::Task::perform(
        fetch_comments(client, id, oid, next),
        Message::BilibiliCommentsPageFetched,
    )
}

/// Apply a by-id dynamic fetch to the open viewer.
fn on_dynamic_fetched(
    state: &mut AppState,
    id: u64,
    result: Result<Illustration, BilibiliError>,
) -> iced::Task<Message> {
    match result {
        Ok(illust) => {
            if !state.viewer.open || state.viewer.illust_id != id {
                return iced::Task::none();
            }
            crate::io::meta_cache::save_detail(id, &illust);
            state.viewer.loading = false;
            state.viewer.total_pages = illust.page_count();
            state.viewer.illust = Some(illust);
            // The seed was missing, so the comment fetch that ran in
            // parallel had no oid; fetch now that we know it.
            let oid = state
                .viewer
                .illust
                .as_ref()
                .and_then(|i| i.provider_data.as_ref())
                .and_then(|d| d.comment_oid.clone());
            let mut tasks: Vec<iced::Task<Message>> = Vec::new();
            if let Some(url) = state.viewer.illust.as_ref().and_then(author_pfp_url) {
                let cache = state.images.image_cache.clone();
                let fetcher = state.images.fetcher.clone();
                tasks.push(iced::Task::perform(
                    fetch_pfp(url, cache, fetcher, Provider::Bilibili),
                    Message::ArtistPfpLoaded,
                ));
            }
            if let Some(client) = state.bilibili.client.clone() {
                tasks.push(iced::Task::perform(
                    fetch_comments(client, id, oid, 1),
                    Message::BilibiliCommentsFetched,
                ));
            }
            if let Some(task) = load_full_from_disk(state, 0) {
                tasks.push(task);
            }
            tasks.push(full_image_or_cache(state));
            iced::Task::batch(tasks)
        }
        Err(e) => {
            log::error!("[viewer] Failed to load dynamic: {}", e);
            iced::Task::done(Message::Error(format!("Failed to load dynamic: {e}")))
        }
    }
}

/// Append a comments page to the open viewer.
fn on_comments_page(state: &mut AppState, page: Comments) {
    state.viewer.comments_next_url = page.next_url.clone();
    state.viewer.comments.extend(page.comments);
}

// ── Reducer ────────────────────────────────────────────────────────────

/// Apply a fetched author-page feed (one user's image dynamics): append,
/// save the offset cursor, resolve the author (name/avatar) from the first
/// item when the page has none yet, and fetch that page's thumbnails.
fn on_artist_page(
    state: &mut AppState,
    uid: u64,
    result: Result<BilibiliPage, BilibiliError>,
) -> iced::Task<Message> {
    state.artist.loading = false;
    match result {
        Ok(page) => {
            // Stale-response guard: only apply to the still-open page.
            if !state.artist.open
                || state.artist.id != uid
                || state.artist.provider != Provider::Bilibili
            {
                return iced::Task::none();
            }
            state.artist.next_url = page.next_cursor;
            let mut tasks: Vec<iced::Task<Message>> = Vec::new();
            let new_illusts = page.items;
            if let Some(author) = new_illusts
                .iter()
                .find(|i| i.user.is_some())
                .and_then(|i| i.user.clone())
            {
                if state.artist.user.is_none() {
                    if let Some(url) = author
                        .profile_image_urls
                        .as_ref()
                        .and_then(|p| p.medium.clone())
                    {
                        let cache = state.images.image_cache.clone();
                        let fetcher = state.images.fetcher.clone();
                        tasks.push(iced::Task::perform(
                            fetch_pfp(url, cache, fetcher, Provider::Bilibili),
                            Message::ArtistPfpLoaded,
                        ));
                    }
                }
                state.artist.user = Some(author);
            }
            // Load thumbnails; register the generation target FIRST so the
            // staleness guard keeps the results (see `UserIllustsFetched`).
            let target = super::thumbnail_target(state);
            for illust in &new_illusts {
                state.images.thumbnail_targets.insert(illust.id, target);
            }
            let fetcher = state.images.fetcher.clone();
            tasks.extend(super::spawn_thumbnail_fetches(
                &new_illusts,
                target,
                state.settings.high_res_thumbnails,
                state.images.image_cache.clone(),
                state.images.thumbnail_cache.clone(),
                fetcher,
            ));
            state.artist.illusts.extend(new_illusts);
            iced::Task::batch(tasks)
        }
        Err(e) => {
            log::warn!("[bilibili] author page fetch failed: {e}");
            iced::Task::none()
        }
    }
}

pub(super) fn handle(state: &mut AppState, message: Message) -> iced::Task<Message> {
    match message {
        Message::BilibiliClientBuilt(result) => match result {
            Ok(boot) => {
                log::info!(
                    "[bilibili] connected as {} ({})",
                    boot.profile.name,
                    boot.profile.user_id
                );
                state.bilibili.client = Some(boot.client);
                state.bilibili.status = BilibiliStatus::Ready {
                    name: boot.profile.name,
                };
                if state.active_tab == Tab::Bilibili && state.tabs.bilibili.illustrations.is_empty()
                {
                    return iced::Task::done(Message::LoadMore);
                }
                iced::Task::none()
            }
            Err(e) => {
                log::warn!("[bilibili] session invalid: {e}");
                state.bilibili.client = None;
                state.bilibili.status = BilibiliStatus::Error(e.to_string());
                iced::Task::none()
            }
        },

        Message::BilibiliCookiePathChanged(path) => {
            state.settings.bilibili_cookie_path = if path.trim().is_empty() {
                None
            } else {
                Some(path)
            };
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::BilibiliUserIdsChanged(text) => {
            state.bilibili.user_ids_input = text.clone();
            state.settings.bilibili_user_ids = parse_user_ids(&text);
            mark_config_dirty(state);
            // Re-arm the Users-mode walk with the new list.
            if state.bilibili.feed_mode == BilibiliFeedMode::Users {
                reset_bilibili_tab(state);
                if state.active_tab == Tab::Bilibili {
                    return iced::Task::batch([
                        schedule_config_flush(),
                        iced::Task::done(Message::LoadMore),
                    ]);
                }
            }
            schedule_config_flush()
        }

        Message::ReloadBilibiliCookies => start_client(state),

        Message::BilibiliFeedModeChanged(mode) => {
            if state.bilibili.feed_mode == mode {
                return iced::Task::none();
            }
            state.bilibili.feed_mode = mode;
            reset_bilibili_tab(state);
            if state.active_tab == Tab::Bilibili {
                return iced::Task::done(Message::LoadMore);
            }
            iced::Task::none()
        }

        Message::BilibiliFeedReload => {
            reset_bilibili_tab(state);
            if state.bilibili.client.is_some() {
                if state.active_tab == Tab::Bilibili {
                    return iced::Task::done(Message::LoadMore);
                }
                iced::Task::none()
            } else {
                start_client(state)
            }
        }

        Message::BilibiliFeedFetched(mode, result) => {
            if mode != state.bilibili.feed_mode {
                log::debug!("[bilibili] dropping stale {mode:?} feed response");
                return iced::Task::none();
            }
            on_feed_fetched(state, result)
        }

        Message::BilibiliUserFetched(uid, result) => on_user_fetched(state, uid, result),

        Message::BilibiliArtistLoaded(uid, result) => on_artist_page(state, uid, result),

        Message::BilibiliLikeResult(id, result) => {
            match result {
                Ok(()) => log::debug!("[bilibili] like state pushed for {id}"),
                Err(e) => log::warn!("[bilibili] like request failed for {id}: {e}"),
            }
            iced::Task::none()
        }

        Message::BilibiliDynamicFetched(id, result) => on_dynamic_fetched(state, id, result),

        Message::BilibiliCommentsFetched(result) => match result {
            Ok(comments) => apply_comments(state, comments),
            Err(e) => {
                log::warn!("[viewer] Failed to load Bilibili comments: {}", e);
                iced::Task::none()
            }
        },

        Message::BilibiliCommentsPageFetched(result) => {
            state.viewer.comments_loading = false;
            match result {
                Ok(page) => on_comments_page(state, page),
                Err(e) => log::warn!("[viewer] Failed to load more comments: {}", e),
            }
            iced::Task::none()
        }

        _ => iced::Task::none(),
    }
}

/// Clear the Bilibili tab and reset the Users-mode walk.
fn reset_bilibili_tab(state: &mut AppState) {
    let t = &mut state.tabs.bilibili;
    t.illustrations.clear();
    t.filtered.clear();
    t.next_url = None;
    t.error = None;
    t.auto_load_remaining = 0;
    t.loading = false;
    state.bilibili.users_pending.clear();
    state.bilibili.users_loading = 0;
}

fn on_feed_fetched(
    state: &mut AppState,
    result: Result<BilibiliPage, BilibiliError>,
) -> iced::Task<Message> {
    state.tabs.bilibili.loading = false;
    match result {
        Ok(page) => {
            let new_count = page.items.len();
            let has_more = page.next_cursor.is_some();
            state.tabs.bilibili.error = None;
            let mut items = page.items;
            let imported = apply_local_bookmarks(state, &mut items);
            // Keep the tab newest-first as pages arrive (lazy merge: no full
            // re-sort per page).
            super::merge_page_by_date(&mut state.tabs.bilibili.illustrations, items);
            state.tabs.bilibili.next_url = page.next_cursor;
            state.tabs.bilibili.reapply(&state.images.palettes);
            refresh_gallery_tags(state);
            if new_count == 0 || !has_more {
                state.tabs.bilibili.auto_load_remaining = 0;
            }
            let mut tasks = spawn_thumbnails_for_window(state);
            if imported {
                tasks.push(schedule_config_flush());
            }
            if state.tabs.bilibili.auto_load_remaining > 0 {
                tasks.push(maybe_auto_load(state));
            }
            return iced::Task::batch(tasks);
        }
        Err(e) => {
            log::warn!("[bilibili] feed fetch failed: {e}");
            note_error(state, &e);
            state.tabs.bilibili.error = Some(e.to_string());
        }
    }
    iced::Task::none()
}

fn on_user_fetched(
    state: &mut AppState,
    uid: u64,
    result: Result<BilibiliPage, BilibiliError>,
) -> iced::Task<Message> {
    state.bilibili.users_loading = state.bilibili.users_loading.saturating_sub(1);
    match result {
        Ok(page) => {
            let has_more = page.next_cursor.is_some();
            state.tabs.bilibili.error = None;
            let mut items = page.items;
            let imported = apply_local_bookmarks(state, &mut items);
            super::merge_page_by_date(&mut state.tabs.bilibili.illustrations, items);
            if has_more {
                // Re-arm this user for the next round.
                state
                    .bilibili
                    .users_pending
                    .push((uid, page.next_cursor.clone()));
            }
            state.tabs.bilibili.reapply(&state.images.palettes);
            refresh_gallery_tags(state);
            let mut tasks = spawn_thumbnails_for_window(state);
            if imported {
                tasks.push(schedule_config_flush());
            }
            if state.bilibili.users_loading == 0 && state.bilibili.users_pending.is_empty() {
                state.tabs.bilibili.loading = false;
                state.tabs.bilibili.auto_load_remaining = 0;
            }
            return iced::Task::batch(tasks);
        }
        Err(e) => {
            log::warn!("[bilibili] user {uid} feed failed: {e}");
            note_error(state, &e);
            if state.bilibili.users_loading == 0 && state.bilibili.users_pending.is_empty() {
                state.tabs.bilibili.loading = false;
                state.tabs.bilibili.error = Some(e.to_string());
            }
        }
    }
    iced::Task::none()
}

/// Seed local bookmark state on freshly fetched items and auto-import posts
/// the account already liked on Bilibili (Bilibili exposes a per-post
/// `like.status`, but no liked-posts feed — so browsing is how we learn about
/// them). Returns whether anything was added to the library.
fn apply_local_bookmarks(state: &mut AppState, items: &mut [Illustration]) -> bool {
    let mut added: Vec<Illustration> = Vec::new();
    for item in items.iter_mut() {
        if state.jobs.library.contains(item.id) {
            item.is_bookmarked = Some(true);
        } else if item.is_bookmarked.unwrap_or(false) {
            // Already liked on Bilibili but unknown locally: import it.
            item.is_bookmarked = Some(true);
            added.push(item.clone());
        } else {
            item.is_bookmarked = Some(false);
        }
    }
    if added.is_empty() {
        return false;
    }
    state.jobs.library.merge(added);
    state.jobs.library_dirty = true;
    true
}

/// Parse a free-form user list: ids separated by commas, whitespace or `;`.
pub(super) fn parse_user_ids(text: &str) -> Vec<u64> {
    let mut ids: Vec<u64> = text
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<u64>().ok())
        .filter(|id| *id != 0)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Degrade to the setup/error panel when the session is rejected.
fn note_error(state: &mut AppState, e: &BilibiliError) {
    if matches!(e, BilibiliError::Auth(_)) {
        state.bilibili.client = None;
        state.bilibili.status = BilibiliStatus::Error(e.to_string());
    }
}
