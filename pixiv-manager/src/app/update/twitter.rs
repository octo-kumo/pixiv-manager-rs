//! Twitter feed/likes reducers and async fetch helpers.
//!
//! The Twitter tab is a plain [`crate::app::state::GalleryTab`] fed by
//! [`fetch_home`]; the mixed Bookmarks tab appends [`fetch_likes`] pages next
//! to the Pixiv bookmark pages (both are stored in the same tab, provider
//! tagged by id). Everything Twitter-specific in the app is either here, in
//! [`crate::twitter`] or in the provider branches of the gallery/viewer/jobs
//! reducers.

use super::gallery::{find_illust_bookmarked, maybe_auto_load, refresh_gallery_tags};
use super::jobs::{find_illust_anywhere, should_download_on_bookmark, start_download};
use super::settings::{mark_config_dirty, schedule_config_flush};
use super::viewer::{apply_comments, author_pfp_url, full_image_or_cache, load_full_from_disk};
use super::{fetch_pfp, spawn_thumbnails_for_window};
use crate::app::message::Message;
use crate::app::state::{AppState, Tab, TwitterStatus};
use crate::media::{native_id, Provider};
use crate::twitter::{
    TwitterApiClient, TwitterError, TwitterFeedMode, TwitterPage, TwitterProfile,
};
use pixivj_rs::model::{Comments, Illustration};

/// Payload of a successful client build: the client plus the account it
/// authenticated as.
#[derive(Debug, Clone)]
pub struct TwitterBoot {
    pub client: TwitterApiClient,
    pub profile: TwitterProfile,
}

/// Where the cookies file comes from: the explicit setting first, then (in
/// release/user builds) the conventional locations. Test builds skip the
/// implicit locations so `cargo test` never reads a developer's real cookies.
pub(super) fn resolve_cookie_path(state: &AppState) -> Option<std::path::PathBuf> {
    if let Some(path) = state
        .settings
        .twitter_cookie_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        return Some(std::path::PathBuf::from(path));
    }
    #[cfg(not(feature = "test-util"))]
    {
        if let Some(config) = crate::io::config::config_path() {
            if let Some(dir) = config.parent() {
                let candidate = dir.join("twitter_cookies.txt");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        let cwd = std::path::PathBuf::from("twitter_cookies.txt");
        if cwd.is_file() {
            return Some(cwd);
        }
    }
    None
}

/// Environment variable pointing at a GraphQL override file (highest
/// priority; useful for custom installs and tests).
pub(super) const GRAPHQL_CONFIG_ENV: &str = "PIXIV_MANAGER_TWITTER_GRAPHQL";

/// Resolve the optional GraphQL-override file: the explicit env var first,
/// then `twitter_graphql.json` next to the config and in the cwd (test builds
/// skip the implicit locations so `cargo test` never reads developer files).
fn resolve_graphql_path() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os(GRAPHQL_CONFIG_ENV) {
        let path = std::path::PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    #[cfg(not(feature = "test-util"))]
    {
        if let Some(config) = crate::io::config::config_path() {
            if let Some(dir) = config.parent() {
                let candidate = dir.join("twitter_graphql.json");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        let cwd = std::path::PathBuf::from("twitter_graphql.json");
        if cwd.is_file() {
            return Some(cwd);
        }
    }
    None
}

/// The runtime GraphQL config: compiled defaults plus the override file when
/// present. A broken override degrades to the defaults (Twitter keeps
/// working) and is logged.
fn load_graphql_config() -> crate::twitter::graphql::GraphQlConfig {
    use crate::twitter::graphql::GraphQlConfig;
    let Some(path) = resolve_graphql_path() else {
        return GraphQlConfig::compiled();
    };
    match GraphQlConfig::load(&path) {
        Ok(config) => {
            log::info!("[twitter] GraphQL config loaded from {}", path.display());
            config
        }
        Err(e) => {
            log::warn!("[twitter] GraphQL config ignored: {e}");
            GraphQlConfig::compiled()
        }
    }
}

/// Where to persist a refreshed query-id override: the file the client loads
/// (env var / existing file), else `twitter_graphql.json` next to the config.
fn graphql_write_path() -> Option<std::path::PathBuf> {
    resolve_graphql_path().or_else(|| {
        crate::io::config::config_path()
            .and_then(|config| config.parent().map(|dir| dir.join("twitter_graphql.json")))
    })
}

/// Start the in-app query-id refresh (the settings button): scrape X's web
/// bundles in the background; the result is persisted and the client reloaded
/// by [`Message::TwitterQueryIdsFetched`].
fn refresh_query_ids(state: &mut AppState) -> iced::Task<Message> {
    let Some(client) = state.twitter.client.clone() else {
        return iced::Task::none();
    };
    state.twitter.query_ids_loading = true;
    iced::Task::perform(
        async move { client.refresh_query_ids().await },
        Message::TwitterQueryIdsFetched,
    )
}

/// (Re)build the Twitter client from the resolved cookies file and verify the
/// session in the background. Safe to call at boot and on every settings
/// reload; a failure leaves Twitter disabled with a readable status. Also
/// (re)loads the GraphQL override file, so refreshing `queryId`s is a data
/// update (reload the client) instead of a rebuild.
pub(super) fn start_client(state: &mut AppState) -> iced::Task<Message> {
    let Some(path) = resolve_cookie_path(state) else {
        state.twitter.client = None;
        state.twitter.status = TwitterStatus::Disabled;
        return iced::Task::none();
    };
    state.twitter.status = TwitterStatus::Loading;
    let creds = match crate::twitter::cookies::load_cookies_file(&path) {
        Ok(creds) => creds,
        Err(e) => {
            log::warn!("[twitter] cookies load failed: {e}");
            state.twitter.client = None;
            state.twitter.status = TwitterStatus::Error(e.to_string());
            return iced::Task::none();
        }
    };
    let client = match TwitterApiClient::with_config(
        creds,
        state.settings.proxy.clone(),
        load_graphql_config(),
    ) {
        Ok(client) => client,
        Err(e) => {
            state.twitter.client = None;
            state.twitter.status = TwitterStatus::Error(e.to_string());
            return iced::Task::none();
        }
    };
    state.twitter.client = Some(client.clone());
    iced::Task::perform(
        async move {
            match client.viewer().await {
                Ok(profile) => Ok(TwitterBoot { client, profile }),
                Err(e) => Err(e),
            }
        },
        Message::TwitterClientBuilt,
    )
}

// ── Async fetches ──────────────────────────────────────────────────────

/// Paginate the Twitter home timeline tab (cursor is stored in `next_url`).
pub(super) fn load_more(state: &mut AppState) -> iced::Task<Message> {
    let Some(client) = state.twitter.client.clone() else {
        // The view shows the setup hint instead of a spinner.
        return iced::Task::none();
    };
    let (loading, cursor, has_items) = {
        let t = &state.tabs.twitter;
        (t.loading, t.next_url.clone(), !t.illustrations.is_empty())
    };
    if loading || (cursor.is_none() && has_items) {
        return iced::Task::none();
    }
    let mode = state.twitter.feed_mode;
    state.tabs.twitter.loading = true;
    iced::Task::perform(fetch_home(client, mode, cursor), move |result| {
        Message::TwitterFeedFetched(mode, result)
    })
}

/// Paginate the Twitter likes half of the mixed Bookmarks tab. Returns
/// `None` when the client is disconnected or the likes are exhausted.
pub(super) fn load_likes(state: &mut AppState) -> Option<iced::Task<Message>> {
    let (loading, cursor, has_items) = {
        let t = &state.tabs.bookmarks;
        (
            t.loading_twitter,
            t.next_twitter_cursor.clone(),
            t.illustrations
                .iter()
                .any(|i| crate::media::is_twitter(i.id)),
        )
    };
    // A `None` cursor with no Twitter items yet means "start from the top";
    // with items it means the likes are exhausted.
    if loading || (cursor.is_none() && has_items) {
        return None;
    }
    let client = state.twitter.client.clone()?;
    state.tabs.bookmarks.loading_twitter = true;
    Some(iced::Task::perform(
        fetch_likes(client, cursor),
        Message::TwitterLikesFetched,
    ))
}

pub(super) async fn fetch_home(
    client: TwitterApiClient,
    mode: TwitterFeedMode,
    cursor: Option<String>,
) -> Result<TwitterPage, TwitterError> {
    client.home_timeline(mode, cursor.as_deref()).await
}

pub(super) async fn fetch_likes(
    client: TwitterApiClient,
    cursor: Option<String>,
) -> Result<TwitterPage, TwitterError> {
    client.likes(cursor.as_deref()).await
}

/// One page of a user's photo tweets (the author page). `native_uid` is the
/// provider-native user id (the namespaced one lives on `ArtistState`).
pub(super) async fn fetch_user_tweets(
    client: TwitterApiClient,
    native_uid: u64,
    cursor: Option<String>,
) -> Result<TwitterPage, TwitterError> {
    client.user_tweets(native_uid, cursor.as_deref()).await
}

pub(super) async fn fetch_tweet(
    client: TwitterApiClient,
    id: u64,
) -> Result<Illustration, TwitterError> {
    client.tweet(crate::media::native_id(id)).await
}

pub(super) async fn fetch_replies(
    client: TwitterApiClient,
    id: u64,
    cursor: Option<String>,
) -> Result<Comments, TwitterError> {
    let native = crate::media::native_id(id);
    let (comments, next_cursor) = client.replies(native, cursor.as_deref()).await?;
    Ok(Comments {
        comments,
        next_url: next_cursor,
    })
}

pub(super) async fn like_tweet(
    client: TwitterApiClient,
    id: u64,
    liked: bool,
) -> Result<(), TwitterError> {
    client.set_like(native_id(id), liked).await
}

// ── Reducer helpers (provider-owned) ───────────────────────────────────

/// Toggle a tweet's "bookmark" (the like action) and auto-download its photos
/// when liking, mirroring the Pixiv bookmark behavior.
pub(super) fn toggle_bookmark(state: &mut AppState, id: u64) -> iced::Task<Message> {
    let Some(client) = state.twitter.client.clone() else {
        return iced::Task::none();
    };
    let liked = find_illust_bookmarked(state, id);
    let task = iced::Task::perform(like_tweet(client, id, !liked), move |result| {
        Message::TwitterLikeResult(id, result)
    });
    if liked || !should_download_on_bookmark(state, id) {
        return task;
    }
    iced::Task::batch([task, start_download(state, id)])
}

/// Open a tweet in the viewer: seed from live state (the gallery item is the
/// freshest copy), fall back to the disk cache then a by-id fetch, and load
/// the replies as comments.
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
                fetch_pfp(url, cache, fetcher, Provider::Twitter),
                Message::ArtistPfpLoaded,
            ));
        }
        if let Some(task) = load_full_from_disk(state, 0) {
            tasks.push(task);
        }
        tasks.push(full_image_or_cache(state));
    } else if let Some(client) = state.twitter.client.clone() {
        tasks.push(iced::Task::perform(fetch_tweet(client, id), move |r| {
            Message::TwitterTweetFetched(id, r)
        }));
    } else {
        state.viewer.loading = false;
    }

    // Replies (rendered by the same comment panel as Pixiv comments).
    if let Some(client) = state.twitter.client.clone() {
        tasks.push(iced::Task::perform(
            fetch_replies(client, id, None),
            Message::TwitterRepliesFetched,
        ));
    }
    iced::Task::batch(tasks)
}

/// Fetch the next page of replies for the open tweet.
pub(super) fn comments_load_more(state: &mut AppState) -> iced::Task<Message> {
    let Some(cursor) = state.viewer.comments_next_url.clone() else {
        return iced::Task::none();
    };
    let Some(client) = state.twitter.client.clone() else {
        return iced::Task::none();
    };
    state.viewer.comments_loading = true;
    let id = state.viewer.illust_id;
    iced::Task::perform(
        fetch_replies(client, id, Some(cursor)),
        Message::TwitterRepliesPageFetched,
    )
}

fn on_tweet_fetched(
    state: &mut AppState,
    id: u64,
    result: Result<Illustration, TwitterError>,
) -> iced::Task<Message> {
    match result {
        Ok(illust) => {
            // Stale-response guard (mirrors `IllustDetailFetched`).
            if !state.viewer.open || state.viewer.illust_id != id {
                return iced::Task::none();
            }
            crate::io::meta_cache::save_detail(id, &illust);
            state.viewer.loading = false;
            state.viewer.total_pages = illust.page_count();
            state.viewer.illust = Some(illust);
            let mut tasks: Vec<iced::Task<Message>> = Vec::new();
            if let Some(url) = state.viewer.illust.as_ref().and_then(author_pfp_url) {
                let cache = state.images.image_cache.clone();
                let fetcher = state.images.fetcher.clone();
                tasks.push(iced::Task::perform(
                    fetch_pfp(url, cache, fetcher, Provider::Twitter),
                    Message::ArtistPfpLoaded,
                ));
            }
            if let Some(task) = load_full_from_disk(state, 0) {
                tasks.push(task);
            }
            tasks.push(full_image_or_cache(state));
            iced::Task::batch(tasks)
        }
        Err(e) => {
            log::error!("[viewer] Failed to load tweet: {}", e);
            iced::Task::done(Message::Error(format!("Failed to load tweet: {e}")))
        }
    }
}

// ── Reducer ────────────────────────────────────────────────────────────

/// Apply a fetched author-page timeline: append photo tweets, save the
/// cursor, resolve the author (name/avatar) from the first item when the
/// page has none yet, and fetch that page's thumbnails.
fn on_artist_page(
    state: &mut AppState,
    uid: u64,
    result: Result<TwitterPage, TwitterError>,
) -> iced::Task<Message> {
    state.artist.loading = false;
    match result {
        Ok(page) => {
            // Stale-response guard: only apply to the still-open page.
            if !state.artist.open
                || state.artist.id != uid
                || state.artist.provider != Provider::Twitter
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
                    // Fetch the profile picture (cached) for the header.
                    if let Some(url) = author
                        .profile_image_urls
                        .as_ref()
                        .and_then(|p| p.medium.clone())
                    {
                        let cache = state.images.image_cache.clone();
                        let fetcher = state.images.fetcher.clone();
                        tasks.push(iced::Task::perform(
                            fetch_pfp(url, cache, fetcher, Provider::Twitter),
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
            log::warn!("[twitter] author page fetch failed: {e}");
            iced::Task::none()
        }
    }
}

pub(super) fn handle(state: &mut AppState, message: Message) -> iced::Task<Message> {
    match message {
        Message::TwitterClientBuilt(result) => match result {
            Ok(boot) => {
                log::info!("[twitter] connected as @{}", boot.profile.handle);
                state.twitter.client = Some(boot.client);
                state.twitter.status = TwitterStatus::Ready {
                    handle: boot.profile.handle,
                };
                // A late client build must fill an already-open empty tab —
                // and add the likes half to an open Bookmarks tab.
                if state.active_tab == Tab::Twitter && state.tabs.twitter.illustrations.is_empty() {
                    return iced::Task::done(Message::LoadMore);
                }
                if state.active_tab == Tab::Bookmarks
                    && !state
                        .tabs
                        .bookmarks
                        .illustrations
                        .iter()
                        .any(|i| crate::media::is_twitter(i.id))
                {
                    return iced::Task::done(Message::LoadMore);
                }
                iced::Task::none()
            }
            Err(e) => {
                log::warn!("[twitter] session invalid: {e}");
                state.twitter.client = None;
                state.twitter.status = TwitterStatus::Error(e.to_string());
                iced::Task::none()
            }
        },

        Message::TwitterCookiePathChanged(path) => {
            state.settings.twitter_cookie_path = if path.trim().is_empty() {
                None
            } else {
                Some(path)
            };
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::ReloadTwitterCookies => start_client(state),

        Message::RefreshTwitterQueryIds => {
            if state.twitter.query_ids_loading {
                return iced::Task::none();
            }
            refresh_query_ids(state)
        }

        Message::TwitterQueryIdsFetched(result) => {
            state.twitter.query_ids_loading = false;
            match result {
                Ok(config) => {
                    let Some(path) = graphql_write_path() else {
                        return iced::Task::done(Message::Error(
                            "Could not resolve a twitter_graphql.json path to write".into(),
                        ));
                    };
                    match config.save_queries(&path) {
                        Ok(()) => {
                            log::info!("[twitter] query ids updated → {}", path.display());
                            // Rebuild so the new ids take effect immediately.
                            start_client(state)
                        }
                        Err(e) => iced::Task::done(Message::Error(format!(
                            "Could not write {}: {e}",
                            path.display()
                        ))),
                    }
                }
                Err(e) => {
                    log::warn!("[twitter] query id refresh failed: {e}");
                    iced::Task::done(Message::Error(format!(
                        "Twitter query id refresh failed: {e}"
                    )))
                }
            }
        }

        Message::TwitterFeedModeChanged(mode) => {
            if state.twitter.feed_mode == mode {
                return iced::Task::none();
            }
            state.twitter.feed_mode = mode;
            reset_twitter_tab(state);
            if state.active_tab == Tab::Twitter {
                return iced::Task::done(Message::LoadMore);
            }
            iced::Task::none()
        }

        Message::TwitterFeedReload => {
            reset_twitter_tab(state);
            if state.twitter.client.is_some() {
                iced::Task::done(Message::LoadMore)
            } else {
                start_client(state)
            }
        }

        Message::TwitterFeedFetched(mode, result) => {
            // Drop a response for a timeline the user already switched away
            // from (a slow For You page landing after selecting Following).
            if mode != state.twitter.feed_mode {
                log::debug!("[twitter] dropping stale {mode:?} feed response");
                return iced::Task::none();
            }
            on_feed_fetched(state, result)
        }

        Message::TwitterLikesFetched(result) => on_likes_fetched(state, result),

        Message::TwitterTweetFetched(id, result) => on_tweet_fetched(state, id, result),

        Message::TwitterArtistLoaded(uid, result) => on_artist_page(state, uid, result),

        Message::TwitterRepliesFetched(result) => match result {
            Ok(comments) => apply_comments(state, comments),
            Err(e) => {
                log::warn!("[viewer] Failed to load replies: {}", e);
                iced::Task::none()
            }
        },

        Message::TwitterRepliesPageFetched(result) => {
            state.viewer.comments_loading = false;
            match result {
                Ok(page) => {
                    state.viewer.comments_next_url = page.next_url.clone();
                    state.viewer.comments.extend(page.comments);
                }
                Err(e) => log::warn!("[viewer] Failed to load more replies: {}", e),
            }
            iced::Task::none()
        }

        _ => iced::Task::none(),
    }
}

/// Clear the Twitter tab (feed mode change / manual reload).
fn reset_twitter_tab(state: &mut AppState) {
    let t = &mut state.tabs.twitter;
    t.illustrations.clear();
    t.filtered.clear();
    t.next_url = None;
    t.error = None;
    t.auto_load_remaining = 0;
    t.loading = false;
}

fn on_feed_fetched(
    state: &mut AppState,
    result: Result<TwitterPage, TwitterError>,
) -> iced::Task<Message> {
    state.tabs.twitter.loading = false;
    match result {
        Ok(page) => {
            let new_count = page.items.len();
            let has_more = page.next_cursor.is_some();
            state.tabs.twitter.error = None;
            extend_unique(&mut state.tabs.twitter.illustrations, page.items);
            state.tabs.twitter.next_url = page.next_cursor;
            state.tabs.twitter.reapply(&state.images.palettes);
            refresh_gallery_tags(state);
            if new_count == 0 || !has_more {
                state.tabs.twitter.auto_load_remaining = 0;
            }
            let mut tasks = spawn_thumbnails_for_window(state);
            if state.tabs.twitter.auto_load_remaining > 0 {
                tasks.push(maybe_auto_load(state));
            }
            return iced::Task::batch(tasks);
        }
        Err(e) => {
            log::warn!("[twitter] feed fetch failed: {e}");
            note_error(state, &e);
            state.tabs.twitter.error = Some(e.to_string());
        }
    }
    iced::Task::none()
}

fn on_likes_fetched(
    state: &mut AppState,
    result: Result<TwitterPage, TwitterError>,
) -> iced::Task<Message> {
    state.tabs.bookmarks.loading_twitter = false;
    match result {
        Ok(page) => {
            let new_count = page.items.len();
            let has_more = page.next_cursor.is_some();
            state.tabs.bookmarks.error = None;
            extend_unique(&mut state.tabs.bookmarks.illustrations, page.items);
            state.tabs.bookmarks.next_twitter_cursor = page.next_cursor;
            // Interleave the two providers chronologically (stable: items on
            // the same date keep their fetch order).
            sort_by_date_desc(&mut state.tabs.bookmarks.illustrations);
            state.tabs.bookmarks.reapply(&state.images.palettes);
            refresh_gallery_tags(state);
            if new_count == 0 || !has_more {
                state.tabs.bookmarks.auto_load_remaining = 0;
            }
            let mut tasks = spawn_thumbnails_for_window(state);
            if state.tabs.bookmarks.auto_load_remaining > 0 {
                tasks.push(maybe_auto_load(state));
            }
            return iced::Task::batch(tasks);
        }
        Err(e) => {
            log::warn!("[twitter] likes fetch failed: {e}");
            note_error(state, &e);
            state.tabs.bookmarks.error = Some(e.to_string());
        }
    }
    iced::Task::none()
}

/// Sort a mixed provider list newest-first. `Reverse(None)` keeps items with
/// no date at the end.
pub(super) fn sort_by_date_desc(illusts: &mut [Illustration]) {
    illusts.sort_by_key(|i| std::cmp::Reverse(i.create_date));
}

/// Append items that aren't already present (a reload racing the previous
/// fetch could otherwise duplicate a page).
fn extend_unique(dst: &mut Vec<Illustration>, items: Vec<Illustration>) {
    let mut seen: std::collections::HashSet<u64> = dst.iter().map(|i| i.id).collect();
    for item in items {
        if seen.insert(item.id) {
            dst.push(item);
        }
    }
}

/// Degrade to the setup/error panel when the session is rejected (expired
/// cookies). Transient errors keep the client so the next fetch can retry.
fn note_error(state: &mut AppState, e: &TwitterError) {
    if matches!(e, TwitterError::Auth(_)) {
        state.twitter.client = None;
        state.twitter.status = TwitterStatus::Error(e.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Environment variables are process-global; serialize the tests that
    /// set the GraphQL-path override.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn graphql_override_is_loaded_from_the_env_var_path() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("twitter_graphql.json");
        std::fs::write(&path, r#"{"queries":{"Viewer":"FROM_ENV"}}"#).unwrap();

        std::env::set_var(GRAPHQL_CONFIG_ENV, &path);
        let resolved = resolve_graphql_path();
        let config = load_graphql_config();
        std::env::remove_var(GRAPHQL_CONFIG_ENV);

        assert_eq!(resolved.as_deref(), Some(path.as_path()));
        assert_eq!(config.query_id("Viewer"), Some("FROM_ENV"));
        // Operations not listed keep their compiled defaults.
        assert!(config.query_id("Likes").is_some());
    }

    #[test]
    fn malformed_graphql_override_degrades_to_the_defaults() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("twitter_graphql.json");
        std::fs::write(&path, "{not json").unwrap();

        std::env::set_var(GRAPHQL_CONFIG_ENV, &path);
        let config = load_graphql_config();
        std::env::remove_var(GRAPHQL_CONFIG_ENV);

        let compiled = crate::twitter::graphql::GraphQlConfig::compiled();
        assert_eq!(config.query_id("Viewer"), compiled.query_id("Viewer"));
        assert_eq!(config.bearer_token, compiled.bearer_token);
    }
}
