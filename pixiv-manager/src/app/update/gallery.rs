//! Gallery tabs: fetching, pagination, filters, sorting, search.

use super::{
    handle_api_error, spawn_thumbnails_for_window, update, viewport_estimate, AUTO_LOAD_MAX_PAGES,
    AUTO_LOAD_MIN_TILES,
};
use crate::app::filter::{FilterMode, OptionFilterKey, Tristate};
use crate::app::message::Message;
use crate::app::query::Query;
use crate::app::sort::SortDirection;
use crate::app::state::AppState;
use crate::app::state::Tab;
use crate::widgets::control_pane;
use pixivj_rs::api::PixivApiClient;
use pixivj_rs::error::PixivError;
use pixivj_rs::model::*;

// ── Async fetch functions ──────────────────────────────────────────────

async fn fetch_recommended(
    client: PixivApiClient,
    next_url: Option<String>,
) -> Result<RecommendedIllusts, PixivError> {
    if let Some(url) = next_url {
        client.fetch_by_next_url(&url).await
    } else {
        client
            .get_recommended_illusts(&RecommendedIllustsFilter::default())
            .await
    }
}

async fn fetch_feed(
    client: PixivApiClient,
    next_url: Option<String>,
) -> Result<SearchedIllusts, PixivError> {
    if let Some(url) = next_url {
        client.fetch_by_next_url(&url).await
    } else {
        client.get_follow_feed(&V2Filter::default()).await
    }
}

async fn fetch_bookmarks(
    client: PixivApiClient,
    next_url: Option<String>,
) -> Result<SearchedIllusts, PixivError> {
    if let Some(url) = next_url {
        client.fetch_by_next_url(&url).await
    } else {
        // One's own bookmarks require the logged-in user id + restrict.
        // The id is captured during the token refresh, so ensure the token
        // is fetched BEFORE building the request (otherwise user_id is None
        // and Pixiv rejects the request with a 400).
        let user_id = client.user_id_ensured().await?;
        let filter = V2Filter {
            user_id: Some(user_id),
            restrict: Some(Restrict::Public),
            ..Default::default()
        };
        client.get_bookmarks(&filter).await
    }
}

async fn search_illusts(
    client: PixivApiClient,
    query: String,
    prefs: crate::app::state::SearchApiPrefs,
    next_url: Option<String>,
) -> Result<SearchedIllusts, PixivError> {
    if let Some(url) = next_url {
        client.fetch_by_next_url(&url).await
    } else {
        let mut filter = SearchedIllustsFilter::new(query);
        filter.sort = prefs.sort;
        filter.search_target = prefs.target;
        // Only send the flag when enabled — absent means false server-side.
        if prefs.translated {
            filter.include_translated_tag_results = Some(true);
        }
        client.search_illusts(&filter).await
    }
}

async fn fetch_ranking(
    client: PixivApiClient,
    mode: Option<pixivj_rs::model::FilterMode>,
    date: Option<String>,
    next_url: Option<String>,
) -> Result<RankedIllusts, PixivError> {
    if let Some(url) = next_url {
        client.fetch_by_next_url(&url).await
    } else {
        client
            .get_ranked_illusts(&RankedIllustsFilter {
                mode,
                filter: None,
                date,
                offset: None,
            })
            .await
    }
}

async fn fetch_autocomplete(
    client: PixivApiClient,
    word: String,
) -> Result<Autocomplete, PixivError> {
    client.autocomplete(&SearchedIllustsFilter::new(word)).await
}

pub(super) async fn fetch_illust_detail(
    client: PixivApiClient,
    id: u64,
) -> Result<IllustDetail, PixivError> {
    client.get_illust_detail(id).await
}

/// Load a local image file from a download directory as a small thumbnail.
/// Returns resized PNG bytes (or the raw bytes when resizing fails).
async fn load_local_thumb(id: u64, dir: std::path::PathBuf) -> (u64, Result<Vec<u8>, String>) {
    let result = tokio::task::spawn_blocking(move || {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Err(format!("cannot read {}", dir.display()));
        };
        let file = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .map(|e| {
                        e.eq_ignore_ascii_case("jpg")
                            || e.eq_ignore_ascii_case("jpeg")
                            || e.eq_ignore_ascii_case("png")
                            || e.eq_ignore_ascii_case("gif")
                    })
                    .unwrap_or(false)
            })
            .min()
            .ok_or_else(|| format!("no image in {}", dir.display()))?;
        let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
        // Downscale to a thumbnail so loading a huge original doesn't spike
        // memory; fall back to the raw bytes if resizing fails.
        Ok(super::resize_to_thumbnail(&bytes, 300).unwrap_or(bytes))
    })
    .await;
    match result {
        Ok(Ok(bytes)) => (id, Ok(bytes)),
        Ok(Err(e)) => (id, Err(e)),
        Err(e) => (id, Err(e.to_string())),
    }
}

pub(super) async fn fetch_comments(
    client: PixivApiClient,
    id: u64,
    last_comment_id: Option<u64>,
) -> Result<Comments, PixivError> {
    client
        .get_illust_comments(&IllustCommentsFilter {
            illust_id: id,
            last_comment_id,
        })
        .await
}

pub(super) async fn fetch_user_detail(
    client: PixivApiClient,
    user_id: u64,
) -> Result<UserDetail, PixivError> {
    client.get_user_details(user_id).await
}

pub(super) async fn fetch_user_illusts(
    client: PixivApiClient,
    user_id: u64,
    next_url: Option<String>,
) -> Result<UserIllusts, PixivError> {
    if let Some(url) = next_url {
        client.fetch_by_next_url(&url).await
    } else {
        client
            .get_user_illusts(&UserIllustQuery {
                user_id,
                illust_type: None,
                filter: None,
                offset: None,
            })
            .await
    }
}

pub(super) async fn bookmark_add(client: PixivApiClient, id: u64) -> Result<(), PixivError> {
    client.add_bookmark(&AddBookmark::new(id)).await.map(|_| ())
}

/// Paginate the mixed Bookmarks tab: one Pixiv bookmarks page (server
/// pagination via `next_url`) and one Twitter likes page (cursor via
/// `next_twitter_cursor`) when each source still has data.
fn load_more_bookmarks(state: &mut AppState) -> iced::Task<Message> {
    let mut tasks: Vec<iced::Task<Message>> = Vec::new();

    // Pixiv bookmarks page.
    let (pixiv_loading, pixiv_cursor, has_pixiv_items) = {
        let t = &state.tabs.bookmarks;
        (
            t.loading,
            t.next_url.clone(),
            t.illustrations
                .iter()
                .any(|i| !crate::media::is_twitter(i.id)),
        )
    };
    if !pixiv_loading && !(pixiv_cursor.is_none() && has_pixiv_items) {
        if let Some(client) = state.client.clone() {
            state.tabs.bookmarks.loading = true;
            tasks.push(iced::Task::perform(
                fetch_bookmarks(client, pixiv_cursor),
                Message::BookmarksFetched,
            ));
        }
    }

    // Twitter likes page (only when the client is connected and not
    // exhausted) — the Twitter module owns that half's pagination.
    if let Some(task) = super::twitter::load_likes(state) {
        tasks.push(task);
    }

    iced::Task::batch(tasks)
}

pub(super) fn refresh_gallery_tags(state: &mut AppState) {
    let illusts: &[Illustration] = &state.active_tab_data().illustrations;
    let mut seen_tags = std::collections::HashSet::new();
    let mut seen_tools = std::collections::HashSet::new();
    let mut out_tags = Vec::new();
    let mut out_tools = Vec::new();
    for illust in illusts {
        for tag in &illust.tags {
            if !tag.name.is_empty() && seen_tags.insert(tag.name.clone()) {
                out_tags.push(tag.name.clone());
            }
        }
        for tool in &illust.tools {
            if seen_tools.insert(tool.clone()) {
                out_tools.push(tool.clone());
            }
        }
    }
    out_tags.sort();
    out_tools.sort();
    state.gallery_tags = out_tags;
    state.gallery_tools = out_tools;
}

fn on_filters_changed(state: &mut AppState) -> iced::Task<Message> {
    // Re-apply filters+sort to the visible list (e.g. the search box changed
    // and doesn't reapply itself), then fetch thumbnails for the new window —
    // otherwise a filter/sort shift leaves new tiles as placeholders until
    // the user scrolls.
    state.reapply_active_tab();
    // Filter mutations are part of the persisted UI session.
    state.active_tab_data_mut().auto_load_remaining = AUTO_LOAD_MAX_PAGES;
    let mut tasks = vec![maybe_auto_load(state)];
    tasks.extend(spawn_thumbnails_for_window(state));
    iced::Task::batch(tasks)
}

/// Seed every tab's "Bookmarked" tristate from the `BookmarkedDisplay`
/// setting: `Hide` turns the tristate to "exclude" on every tab except the
/// Bookmarks tab; any other mode clears the exclusion (never touching a
/// tristate the user chose by hand).
pub(super) fn apply_bookmark_filter_defaults(state: &mut AppState) {
    let display = state.settings.bookmarked_display;
    let palettes = &state.images.palettes;
    for (tab, gallery) in state.tabs.enumerate_mut() {
        if tab == Tab::Bookmarks {
            continue;
        }
        let target = crate::app::state::bookmarked_filter_default(tab, display);
        if gallery.filters.options.bookmarked != Tristate::Indeterminate
            && gallery.filters.options.bookmarked != target
        {
            continue; // the user picked this themselves; leave it
        }
        if gallery.filters.options.bookmarked != target {
            gallery.filters.options.bookmarked = target;
            gallery.reapply(palettes);
        }
    }
}

/// Keep a tab's continuous loading going after a page lands: fetch the next
/// page (with a small pause so the API/CDN aren't hammered) while the toggle
/// is on and the tab still has pages. Returns whether a continuation was
/// scheduled (the caller batches it).
fn auto_load_continuation(state: &mut AppState) -> Option<iced::Task<Message>> {
    let tab = state.active_tab;
    if !state.tab(tab).auto_load || state.tab(tab).next_url.is_none() {
        return None;
    }
    Some(iced::Task::perform(
        async {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        },
        |_| Message::LoadMore,
    ))
}

/// Apply a mutation to the Search tab's server-side preferences and restart
/// the current search from page 1 (no-op when nothing is being searched).
fn refetch_search_with_prefs(
    state: &mut AppState,
    mutate: impl FnOnce(&mut crate::app::state::SearchApiPrefs),
) -> iced::Task<Message> {
    mutate(&mut state.tabs.search.search_api);
    if state.tabs.search.query.is_none() {
        return iced::Task::none();
    }
    state.tabs.search.illustrations.clear();
    state.tabs.search.filtered.clear();
    state.tabs.search.next_url = None;
    iced::Task::done(Message::LoadMore)
}

/// If the active tab's filtered grid is small and more pages exist, fire one
/// more `LoadMore` (consuming one of the `auto_load_remaining` slots).
/// New items go through the active filter automatically via `reapply()`.
pub(super) fn maybe_auto_load(state: &mut AppState) -> iced::Task<Message> {
    let t = state.active_tab_data();
    let (loading, has_data, next_url, twitter_cursor) = (
        t.loading || t.loading_twitter,
        !t.illustrations.is_empty(),
        t.next_url.clone(),
        t.next_twitter_cursor.clone(),
    );
    // Bilibili's fixed-user mode paginates per user, without a tab cursor.
    let bilibili_users_more =
        state.active_tab == Tab::Bilibili && !state.bilibili.users_pending.is_empty();
    if loading
        || !has_data
        || (next_url.is_none() && twitter_cursor.is_none() && !bilibili_users_more)
    {
        return iced::Task::none();
    }
    if t.filtered.len() >= AUTO_LOAD_MIN_TILES {
        return iced::Task::none();
    }
    if t.auto_load_remaining == 0 {
        return iced::Task::none();
    }
    state.active_tab_data_mut().auto_load_remaining -= 1;
    iced::Task::done(Message::LoadMore)
}

/// Whether a string looks like a ranking `date` value (`YYYY-MM-DD`), which
/// is the only form Pixiv's ranking endpoint accepts.
fn looks_like_ranking_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(|b| b.is_ascii_digit())
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[8..10].iter().all(|b| b.is_ascii_digit())
}

/// Rescan the gallery directory for illust ids present on disk and propagate
/// the set to every tab (drives the "Missing" filter's local-download
/// semantics and per-tile checkmarks). Re-applies the active tab.
pub(super) fn refresh_downloaded_ids(state: &mut AppState) {
    let gallery_path = state
        .settings
        .gallery_path
        .clone()
        .unwrap_or_else(crate::core::default_gallery_dir);
    let downloaded = crate::core::scan_downloaded_ids(&gallery_path);
    let palettes = &state.images.palettes;
    for tab in state.tabs.iter_mut() {
        tab.downloaded = downloaded.clone();
        tab.reapply(palettes);
    }
}

/// Find whether an illust is bookmarked. Checks the viewer illust first (the
/// freshest data), then the active tab.
pub(super) fn find_illust_bookmarked(state: &AppState, id: u64) -> bool {
    if let Some(illust) = &state.viewer.illust {
        if illust.id == id {
            return illust.bookmarked();
        }
    }
    let illusts = &state.active_tab_data().illustrations;
    illusts.iter().any(|i| i.id == id && i.bookmarked())
}

/// Toggle the bookmark state for an illust in all tabs.
pub(super) fn toggle_bookmark_in_state(state: &mut AppState, id: u64) {
    let current = find_illust_bookmarked(state, id);
    set_bookmark_in_state(state, id, !current);
}

/// Set the bookmark state for an illust in every tab and the viewer.
pub(super) fn set_bookmark_in_state(state: &mut AppState, id: u64, value: bool) {
    for illusts in state.tabs.iter_mut().map(|tab| &mut tab.illustrations) {
        if let Some(illust) = illusts.iter_mut().find(|i| i.id == id) {
            illust.is_bookmarked = Some(value);
        }
    }
    if let Some(ref mut illust) = state.viewer.illust {
        if illust.id == id {
            illust.is_bookmarked = Some(value);
        }
    }
}

pub(super) fn handle(state: &mut AppState, message: Message) -> iced::Task<Message> {
    match message {
        Message::TabSelected(tab) => {
            log::info!("[update] TabSelected: {:?}", tab);
            // Persist the outgoing tab's scroll position so switching back
            // restores it (state must survive tab switches).
            let previous = state.active_tab;
            state.tab_mut(previous).scroll = state.gallery_scroll;
            state.active_tab = tab;
            // Restore the incoming tab's saved offset: the row-build window
            // (`gallery_scroll` + `gallery_window`) and the widget's own
            // scroll (the view passes the saved offset as the restore value)
            // must agree, or the gallery renders rows off-screen.
            let restored = state.tab(tab).scroll;
            state.gallery_scroll = restored;
            let viewport = viewport_estimate(state);
            state.gallery_window = Some((restored, restored + viewport));
            state.scroll_at_bottom = false;
            // If the selected tab has no data yet, trigger a load. Extra is
            // fed by a local directory scan (no Pixiv source); Ranking by the
            // leaderboard fetch; both start on first selection.
            let needs_load = state.tab(tab).illustrations.is_empty();
            if needs_load {
                let mut tasks = vec![iced::Task::done(Message::LoadMore)];
                tasks.extend(spawn_thumbnails_for_window(state));
                return iced::Task::batch(tasks);
            }
            iced::Task::batch(spawn_thumbnails_for_window(state))
        }

        Message::LoadMore => {
            log::info!("[update] LoadMore (tab={:?})", state.active_tab);
            let tab = state.active_tab;
            // Twitter and Bilibili have their own clients (and pagination);
            // the Bookmarks tab paginates both providers side by side.
            if tab == Tab::Twitter {
                return super::twitter::load_more(state);
            }
            if tab == Tab::Bilibili {
                return super::bilibili::load_more(state);
            }
            if tab == Tab::Bookmarks {
                return load_more_bookmarks(state);
            }
            let Some(client) = state.client.clone() else {
                log::warn!("[update] LoadMore: no client, skipping");
                return iced::Task::none();
            };

            let (loading, next_url) = {
                let t = state.tab_mut(tab);
                (t.loading, t.next_url.clone())
            };
            if loading {
                return iced::Task::none();
            }

            // If next_url is None and we already have data, we've loaded all pages.
            if next_url.is_none() && !state.tab(tab).illustrations.is_empty() {
                return iced::Task::none();
            }

            state.tab_mut(tab).loading = true;

            match tab {
                Tab::Recommend => iced::Task::perform(
                    fetch_recommended(client, next_url),
                    Message::RecommendedFetched,
                ),
                Tab::Feed => {
                    iced::Task::perform(fetch_feed(client, next_url), Message::FollowFeedFetched)
                }
                Tab::Search => {
                    // For search, we need the current query
                    let query = state.tab(tab).query.clone().unwrap_or_default();
                    if query.is_empty() {
                        state.tab_mut(tab).loading = false;
                        return iced::Task::none();
                    }
                    let prefs = state.tabs.search.search_api;
                    iced::Task::perform(
                        search_illusts(client, query, prefs, next_url),
                        Message::SearchFetched,
                    )
                }
                Tab::Ranking => {
                    let (mode, date) = {
                        let t = state.tab(tab);
                        let date = if t.ranking_date.trim().is_empty() {
                            None
                        } else {
                            Some(t.ranking_date.trim().to_string())
                        };
                        (t.ranking_mode, date)
                    };
                    iced::Task::perform(
                        fetch_ranking(client, mode, date, next_url),
                        Message::RankedFetched,
                    )
                }
                Tab::Extra => {
                    // Extra is fed by a local gallery scan, not a Pixiv fetch.
                    let _ = &client;
                    state.tab_mut(tab).loading = false;
                    iced::Task::done(Message::ScanExtra)
                }
                // Handled above (own clients / mixed pagination).
                Tab::Twitter | Tab::Bilibili | Tab::Bookmarks => unreachable!(),
            }
        }

        Message::ResetTab => {
            let t = state.active_tab_data_mut();
            t.illustrations.clear();
            t.filtered.clear();
            t.next_url = None;
            t.next_twitter_cursor = None;
            t.loading = false;
            t.loading_twitter = false;
            let reload = iced::Task::done(Message::LoadMore);
            // A Twitter/Bilibili reload re-verifies the session when the
            // client is missing (the tab's empty state offers the same).
            if state.active_tab == Tab::Twitter && state.twitter.client.is_none() {
                return super::twitter::start_client(state);
            }
            if state.active_tab == Tab::Bilibili {
                state.bilibili.users_pending.clear();
                state.bilibili.users_loading = 0;
                if state.bilibili.client.is_none() {
                    return super::bilibili::start_client(state);
                }
            }
            reload
        }

        // ── API results ────────────────────────────────────────────
        Message::RecommendedFetched(result) => {
            state.tabs.recommend.loading = false;
            log::info!(
                "[update] RecommendedFetched: {:?}",
                result
                    .as_ref()
                    .map(|d| format!("{} illusts", d.illusts.len()))
                    .map_err(|e| format!("{}", e))
            );
            match result {
                Ok(data) => {
                    let new_illusts = data.illusts.clone();
                    let has_more = data.next_url.is_some();
                    state.tabs.recommend.error = None;
                    state.tabs.recommend.illustrations.extend(data.illusts);
                    state.tabs.recommend.next_url = data.next_url;
                    state.tabs.recommend.reapply(&state.images.palettes);
                    refresh_gallery_tags(state);
                    // Stop auto-loading when no more images arrive.
                    if new_illusts.is_empty() || !has_more {
                        state.tabs.recommend.auto_load_remaining = 0;
                        state.tabs.recommend.auto_load = false;
                    }
                    // Lazy thumbnails: only the visible window is fetched.
                    let mut tasks = spawn_thumbnails_for_window(state);
                    if state.tabs.recommend.auto_load_remaining > 0 {
                        tasks.push(maybe_auto_load(state));
                    }
                    if let Some(cont) = auto_load_continuation(state) {
                        tasks.push(cont);
                    }
                    return iced::Task::batch(tasks);
                }
                Err(e) => {
                    if let Some(task) = handle_api_error(state, &e) {
                        return task;
                    }
                    state.tabs.recommend.error = Some(format!("{}", e));
                }
            }
            iced::Task::none()
        }

        Message::FollowFeedFetched(result) => {
            state.tabs.feed.loading = false;
            match result {
                Ok(data) => {
                    let new_illusts = data.illusts.clone();
                    let has_more = data.next_url.is_some();
                    state.tabs.feed.error = None;
                    state.tabs.feed.illustrations.extend(data.illusts);
                    state.tabs.feed.next_url = data.next_url;
                    state.tabs.feed.reapply(&state.images.palettes);
                    refresh_gallery_tags(state);
                    if new_illusts.is_empty() || !has_more {
                        state.tabs.feed.auto_load_remaining = 0;
                        state.tabs.feed.auto_load = false;
                    }
                    let mut tasks = spawn_thumbnails_for_window(state);
                    if state.tabs.feed.auto_load_remaining > 0 {
                        tasks.push(maybe_auto_load(state));
                    }
                    if let Some(cont) = auto_load_continuation(state) {
                        tasks.push(cont);
                    }
                    return iced::Task::batch(tasks);
                }
                Err(e) => {
                    if let Some(task) = handle_api_error(state, &e) {
                        return task;
                    }
                    state.tabs.feed.error = Some(format!("{}", e));
                }
            }
            iced::Task::none()
        }

        Message::BookmarksFetched(result) => {
            state.tabs.bookmarks.loading = false;
            match result {
                Ok(data) => {
                    let new_illusts = data.illusts.clone();
                    let has_more = data.next_url.is_some();
                    state.tabs.bookmarks.error = None;
                    state.tabs.bookmarks.illustrations.extend(data.illusts);
                    // Keep the two providers interleaved newest-first.
                    super::twitter::sort_by_date_desc(&mut state.tabs.bookmarks.illustrations);
                    state.tabs.bookmarks.next_url = data.next_url;
                    state.tabs.bookmarks.reapply(&state.images.palettes);
                    refresh_gallery_tags(state);
                    if new_illusts.is_empty() || !has_more {
                        state.tabs.bookmarks.auto_load_remaining = 0;
                    }
                    let mut tasks = spawn_thumbnails_for_window(state);
                    if state.tabs.bookmarks.auto_load_remaining > 0 {
                        tasks.push(maybe_auto_load(state));
                    }
                    return iced::Task::batch(tasks);
                }
                Err(e) => {
                    if let Some(task) = handle_api_error(state, &e) {
                        return task;
                    }
                    state.tabs.bookmarks.error = Some(format!("{}", e));
                }
            }
            iced::Task::none()
        }

        Message::SearchFetched(result) => {
            state.tabs.search.loading = false;
            match result {
                Ok(data) => {
                    let new_illusts = data.illusts.clone();
                    let has_more = data.next_url.is_some();
                    state.tabs.search.error = None;
                    state.tabs.search.illustrations.extend(data.illusts);
                    state.tabs.search.next_url = data.next_url;
                    state.tabs.search.reapply(&state.images.palettes);
                    refresh_gallery_tags(state);
                    // Reached the true end: the API returned no next_url OR a
                    // page with nothing new — both mean "loaded everything".
                    // (A pagination that never returns None must not spin the
                    // continuous loader forever.)
                    let exhausted = !has_more || new_illusts.is_empty();
                    if exhausted {
                        state.tabs.search.auto_load_remaining = 0;
                        state.tabs.search.auto_load = false;
                    }
                    let mut tasks = spawn_thumbnails_for_window(state);
                    if state.tabs.search.auto_load_remaining > 0 {
                        tasks.push(maybe_auto_load(state));
                    }
                    // Continuous search loading: keep pulling pages until the
                    // query is exhausted, pausing briefly between pages so the
                    // CDN/API aren't hammered.
                    if let Some(cont) = auto_load_continuation(state) {
                        tasks.push(cont);
                    }
                    return iced::Task::batch(tasks);
                }
                Err(e) => {
                    if let Some(task) = handle_api_error(state, &e) {
                        return task;
                    }
                    state.tabs.search.error = Some(format!("{}", e));
                    // Stop the continuous loader on failure.
                    state.tabs.search.auto_load = false;
                }
            }
            iced::Task::none()
        }

        Message::RankedFetched(result) => {
            state.tabs.ranking.loading = false;
            match result {
                Ok(data) => {
                    let new_illusts = data.illusts.clone();
                    let has_more = data.next_url.is_some();
                    state.tabs.ranking.error = None;
                    state.tabs.ranking.illustrations.extend(data.illusts);
                    state.tabs.ranking.next_url = data.next_url;
                    state.tabs.ranking.reapply(&state.images.palettes);
                    refresh_gallery_tags(state);
                    if new_illusts.is_empty() || !has_more {
                        state.tabs.ranking.auto_load_remaining = 0;
                        state.tabs.ranking.auto_load = false;
                    }
                    let mut tasks = spawn_thumbnails_for_window(state);
                    if state.tabs.ranking.auto_load_remaining > 0 {
                        tasks.push(maybe_auto_load(state));
                    }
                    if let Some(cont) = auto_load_continuation(state) {
                        tasks.push(cont);
                    }
                    return iced::Task::batch(tasks);
                }
                Err(e) => {
                    if let Some(task) = handle_api_error(state, &e) {
                        return task;
                    }
                    state.tabs.ranking.error = Some(format!("{}", e));
                }
            }
            iced::Task::none()
        }

        // ── Filter changes ─────────────────────────────────────────
        Message::SearchTextChanged(text) => {
            // The search box is per-tab: its text lives in each tab's own
            // filter state and must not leak into (or be cleared from) the
            // other tabs.
            let filters = &mut state.active_tab_data_mut().filters;
            if filters.mode == FilterMode::Query {
                // The text query has its own box content; the Pixiv
                // autocomplete only serves the classic search.
                filters.query_text = text;
                return on_filters_changed(state);
            }
            filters.search_text = text;
            // On the Search tab, fetch autocomplete suggestions for the query.
            if state.active_tab == Tab::Search
                && !state.tabs.search.filters.search_text.trim().is_empty()
            {
                let filter_task = on_filters_changed(state);
                if let Some(client) = state.client.clone() {
                    let query = state.tabs.search.filters.search_text.clone();
                    let ac = iced::Task::perform(
                        fetch_autocomplete(client, query.clone()),
                        move |result| Message::AutocompleteLoaded(query, result),
                    );
                    return iced::Task::batch([filter_task, ac]);
                }
                return filter_task;
            }
            on_filters_changed(state)
        }

        Message::FilterModeChanged(mode) => {
            // One mode for the whole app; each tab keeps its own text.
            let palettes = &state.images.palettes;
            for tab in state.tabs.iter_mut() {
                tab.filters.mode = mode;
                tab.reapply(palettes);
            }
            state.autocomplete_tags.clear();
            state.autocomplete_selected = None;
            on_filters_changed(state)
        }

        Message::QueryTokenRemoved(index) => {
            let filters = &mut state.active_tab_data_mut().filters;
            filters.query_text = Query::parse(&filters.query_text).without_token(index);
            on_filters_changed(state)
        }

        Message::QuerySuggestionClicked(suggestion) => {
            let filters = &mut state.active_tab_data_mut().filters;
            filters.query_text =
                crate::app::query::apply_suggestion(&filters.query_text, &suggestion);
            let filter_task = on_filters_changed(state);
            iced::Task::batch([
                filter_task,
                iced::widget::operation::focus(control_pane::SEARCH_INPUT_ID),
                iced::widget::operation::move_cursor_to_end(control_pane::SEARCH_INPUT_ID),
            ])
        }

        Message::SearchTargetChanged(key, on) => {
            use crate::app::filter::SearchTargetKey::*;
            for tab in state.tabs.iter_mut() {
                let filters = &mut tab.filters;
                match key {
                    Tags => filters.search_tags = on,
                    Author => filters.search_author = on,
                    Title => filters.search_title = on,
                    Caption => filters.search_caption = on,
                    Id => filters.search_id = on,
                }
            }
            state.reapply_active_tab();
            iced::Task::none()
        }

        Message::SearchSubmitted => {
            // Enter in the search bar runs a Pixiv search on the Search tab.
            if state.active_tab == Tab::Search {
                let filters = &state.tabs.search.filters;
                // Query mode searches Pixiv for the query's plain tags and
                // filters the results locally with the rest.
                let query = if filters.mode == FilterMode::Query {
                    Query::parse(&filters.query_text).server_terms()
                } else {
                    filters.search_text.clone()
                };
                if !query.trim().is_empty() {
                    state.tabs.search.query = Some(query);
                    state.tabs.search.illustrations.clear();
                    state.tabs.search.filtered.clear();
                    state.tabs.search.next_url = None;
                    state.tabs.search.auto_load_remaining = 0;
                    return iced::Task::done(Message::LoadMore);
                }
            }
            iced::Task::none()
        }

        Message::TagSearchRequested(tag) => {
            // Clicking a tag chip jumps to the Search tab and runs that tag
            // as a fresh Pixiv search.
            log::info!("[update] TagSearchRequested: {}", tag);
            state.active_tab = Tab::Search;

            state.tabs.search.query = Some(tag.clone());
            if state.tabs.search.filters.mode == FilterMode::Query {
                state.tabs.search.filters.query_text = tag.trim().replace(' ', "_");
            } else {
                state.tabs.search.filters.search_text = tag;
            }
            state.tabs.search.illustrations.clear();
            state.tabs.search.filtered.clear();
            state.tabs.search.next_url = None;
            state.tabs.search.auto_load_remaining = 0;
            state.autocomplete_tags.clear();
            let mut tasks = vec![iced::Task::done(Message::LoadMore)];
            tasks.extend(spawn_thumbnails_for_window(state));
            iced::Task::batch(tasks)
        }

        // ── Server-side search parameters (sort/match/translated) ──
        Message::ApiSortChanged(sort) => {
            refetch_search_with_prefs(state, |prefs| prefs.sort = sort)
        }

        Message::ApiTargetChanged(target) => {
            refetch_search_with_prefs(state, |prefs| prefs.target = target)
        }

        Message::ApiTranslatedChanged(on) => {
            refetch_search_with_prefs(state, |prefs| prefs.translated = on)
        }

        Message::AutocompleteLoaded(query, result) => {
            match result {
                Ok(ac) => {
                    // Ignore stale results for an older query.
                    if query == state.tabs.search.filters.search_text {
                        state.autocomplete_tags = ac.tags.iter().map(|t| t.name.clone()).collect();
                        state.autocomplete_selected = None;
                    }
                }
                Err(e) => log::warn!("[update] Autocomplete failed: {}", e),
            }
            iced::Task::none()
        }

        Message::AutocompleteTagClicked(tag) => {
            state.autocomplete_selected = None;
            if state.active_tab == Tab::Search {
                state.tabs.search.query = Some(tag.clone());
                state.tabs.search.filters.search_text = tag;
                state.tabs.search.illustrations.clear();
                state.tabs.search.filtered.clear();
                state.tabs.search.next_url = None;
                state.autocomplete_tags.clear();
                return iced::Task::done(Message::LoadMore);
            }
            iced::Task::none()
        }

        Message::AutocompleteMove(delta) => {
            let count = state.autocomplete_tags.len().min(10);
            if count == 0 {
                return iced::Task::none();
            }
            // Wrap around at both ends; starting from -1 (nothing selected)
            // ↓ lands on the first entry.
            let next = match state.autocomplete_selected {
                Some(i) => (i as i32 + delta).rem_euclid(count as i32) as usize,
                None if delta > 0 => 0,
                None => count - 1,
            };
            state.autocomplete_selected = Some(next);
            iced::Task::none()
        }

        Message::AutocompleteCommit => {
            // Enter picks the highlighted suggestion; without one it runs
            // the query as typed.
            match state.autocomplete_selected {
                Some(i) => {
                    let tag = state.autocomplete_tags.get(i).cloned().unwrap_or_default();
                    if tag.is_empty() {
                        iced::Task::none()
                    } else {
                        update(state, Message::AutocompleteTagClicked(tag))
                    }
                }
                None => {
                    // Hide the dropdown, then run the query as typed.
                    state.autocomplete_selected = None;
                    state.autocomplete_tags.clear();
                    update(state, Message::SearchSubmitted)
                }
            }
        }

        Message::FocusSearch => iced::widget::operation::focus(control_pane::SEARCH_INPUT_ID),

        Message::ToggleShortcuts => {
            state.show_shortcuts = !state.show_shortcuts;
            iced::Task::none()
        }

        Message::ScrollPositionChanged(at_bottom) => {
            // Fire LoadMore only on the rising edge (scrolling into the
            // bottom), never repeatedly while parked at the bottom.
            let was_at_bottom = state.scroll_at_bottom;
            state.scroll_at_bottom = at_bottom;
            if at_bottom && !was_at_bottom {
                let mut tasks = vec![iced::Task::done(Message::LoadMore)];
                tasks.extend(spawn_thumbnails_for_window(state));
                return iced::Task::batch(tasks);
            }
            iced::Task::batch(spawn_thumbnails_for_window(state))
        }

        Message::GalleryWindowChanged { start, end } => {
            // The gallery told us which content region is visible: mirror it
            // (the view builds only these rows) and fetch their thumbnails.
            state.gallery_scroll = start;
            state.gallery_window = Some((start, end));
            iced::Task::batch(spawn_thumbnails_for_window(state))
        }

        Message::ThumbnailWindowPulse => {
            // Continue a capped lazy-load pass until the window is requested.
            iced::Task::batch(spawn_thumbnails_for_window(state))
        }

        Message::RefreshDownloaded => {
            refresh_downloaded_ids(state);
            iced::Task::none()
        }

        Message::ScanExtra => {
            let gallery_path = state
                .settings
                .gallery_path
                .clone()
                .unwrap_or_else(crate::core::default_gallery_dir);
            // Local scan on a blocking thread (walks the whole gallery dir).
            let exclude: std::collections::HashSet<u64> =
                state.jobs.library.illusts.iter().map(|i| i.id).collect();
            iced::Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        crate::core::scan_local_illusts(&gallery_path, &exclude)
                    })
                    .await
                    .map_err(|e| e.to_string())
                },
                Message::ExtraScanned,
            )
        }

        Message::ExtraScanned(result) => {
            match result {
                Ok((illusts, dirs)) => {
                    let palettes = &state.images.palettes;
                    let extra = &mut state.tabs.extra;
                    extra.illustrations = illusts;
                    extra.filtered = extra.illustrations.clone();
                    extra.sort.sort(&mut extra.filtered, palettes);
                    // The Extra tab is entirely local — the downloaded set is
                    // everything it sees.
                    extra.downloaded = extra.illustrations.iter().map(|i| i.id).collect();
                    // Lazily load the first ~40 local thumbnails from disk.
                    let mut tasks: Vec<iced::Task<Message>> = Vec::new();
                    for (id, dir) in dirs.into_iter().take(40) {
                        if state.images.thumbnail_handles.contains_key(&id) {
                            continue;
                        }
                        tasks.push(iced::Task::perform(
                            load_local_thumb(id, dir),
                            |(id, result)| Message::LocalThumbnailLoaded(id, result),
                        ));
                    }
                    iced::Task::batch(tasks)
                }
                Err(e) => {
                    log::error!("[update] Extra scan failed: {}", e);
                    iced::Task::none()
                }
            }
        }

        Message::LocalThumbnailLoaded(id, result) => {
            match result {
                Ok(bytes) => {
                    let handle = iced::widget::image::Handle::from_bytes(bytes);
                    state.images.thumbnail_handles.insert(id, handle);
                }
                Err(e) => log::warn!("[update] Local thumbnail {} failed: {}", id, e),
            }
            iced::Task::none()
        }

        Message::OptionFilterToggled(key) => {
            let filters = &mut state.active_tab_data_mut().filters;
            let opt = match key {
                OptionFilterKey::R18 => &mut filters.options.r18,
                OptionFilterKey::Ai => &mut filters.options.ai,
                OptionFilterKey::Bookmarked => &mut filters.options.bookmarked,
                OptionFilterKey::Landscape => &mut filters.options.landscape,
                OptionFilterKey::Visible => &mut filters.options.visible,
                OptionFilterKey::Restricted => &mut filters.options.restricted,
                OptionFilterKey::Missing => &mut filters.options.missing,
            };
            *opt = opt.cycle();
            state.reapply_active_tab();
            on_filters_changed(state)
        }

        Message::TagAdded(tag) => {
            let filters = &mut state.active_tab_data_mut().filters;
            if !filters.selected_tags.contains(&tag) {
                filters.selected_tags.push(tag);
            }
            filters.tag_input.clear();
            state.tag_dropdown_open = false;
            state.reapply_active_tab();
            on_filters_changed(state)
        }

        Message::TagInputChanged(text) => {
            state.active_tab_data_mut().filters.tag_input = text;
            state.tag_dropdown_open = true;
            iced::Task::none()
        }

        Message::TagRemoved(tag) => {
            state
                .active_tab_data_mut()
                .filters
                .selected_tags
                .retain(|t| t != &tag);
            state.reapply_active_tab();
            on_filters_changed(state)
        }

        Message::ToolFilterToggled(tool) => {
            let filters = &mut state.active_tab_data_mut().filters;
            let tristate = filters
                .tool_states
                .entry(tool)
                .or_insert(crate::app::filter::Tristate::Indeterminate);
            *tristate = tristate.cycle();
            state.reapply_active_tab();
            on_filters_changed(state)
        }

        Message::SourceFilterToggled(key) => {
            use crate::app::filter::SourceKey::*;
            let sources = &mut state.active_tab_data_mut().filters.sources;
            match key {
                Pixiv => sources.pixiv = !sources.pixiv,
                Twitter => sources.twitter = !sources.twitter,
                Bilibili => sources.bilibili = !sources.bilibili,
            }
            state.reapply_active_tab();
            on_filters_changed(state)
        }

        Message::ToggleAdvancedFilters => {
            state.show_advanced = !state.show_advanced;
            iced::Task::none()
        }

        Message::ResetFilters => {
            // "Clear" resets both the filters AND the sort (the user expects
            // a clean list back).
            let tab = state.active_tab;
            let t = state.tab_mut(tab);
            // The mode is a view preference, not a filter: keep it.
            let mode = t.filters.mode;
            t.filters = Default::default();
            t.filters.mode = mode;
            t.sort = Default::default();
            // Re-seed the Bookmarked exclusion when the "hide bookmarked"
            // option is on (a plain reset would otherwise show them again).
            apply_bookmark_filter_defaults(state);
            state.reapply_active_tab();
            on_filters_changed(state)
        }

        // ── Continuous loading (the "▶ Load all" toggle) ────────────
        Message::ToggleAutoLoad => {
            let tab = state.active_tab;
            let on = !state.tab(tab).auto_load;
            state.tab_mut(tab).auto_load = on;
            if on && state.tab(tab).next_url.is_some() {
                return iced::Task::done(Message::LoadMore);
            }
            iced::Task::none()
        }

        // ── Sort changes ───────────────────────────────────────────
        Message::SortKeyClicked(key) => {
            let sort = &mut state.active_tab_data_mut().sort;
            if let Some(pos) = sort.active.iter().position(|(k, _)| *k == key) {
                // Cycle direction; ALWAYS nullable so the third click un-sorts
                // (returns to neutral) — every sort button is a tristate.
                let (_, dir) = sort.active[pos];
                let next = dir.cycle(true);
                if next == SortDirection::Undefined {
                    sort.active.remove(pos);
                } else {
                    sort.active[pos].1 = next;
                }
            } else {
                // Add a new sort key.
                if !sort.multi_sort {
                    sort.active.clear();
                }
                sort.active.push((key, SortDirection::Down));
            }
            state.reapply_active_tab();
            on_filters_changed(state)
        }

        Message::MultiSortToggled => {
            let sort = &mut state.active_tab_data_mut().sort;
            sort.multi_sort = !sort.multi_sort;
            if !sort.multi_sort {
                sort.active.truncate(1);
            }
            state.reapply_active_tab();
            iced::Task::none()
        }

        // ── Leaderboard (ranking) ──────────────────────────────────
        Message::RankingModeChanged(mode) => {
            state.tabs.ranking.ranking_mode = Some(mode);
            state.tabs.ranking.illustrations.clear();
            state.tabs.ranking.filtered.clear();
            state.tabs.ranking.next_url = None;
            state.tabs.ranking.auto_load_remaining = 0;
            iced::Task::done(Message::LoadMore)
        }

        Message::RankingDateChanged(date) => {
            let trimmed = date.trim().to_string();
            let changed = state.tabs.ranking.ranking_date != trimmed;
            state.tabs.ranking.ranking_date = trimmed;
            // Refetch only when the date becomes a valid YYYY-MM-DD or is
            // cleared — never on every keystroke.
            let valid = state.tabs.ranking.ranking_date.is_empty()
                || looks_like_ranking_date(&state.tabs.ranking.ranking_date);
            if changed && valid {
                state.tabs.ranking.illustrations.clear();
                state.tabs.ranking.filtered.clear();
                state.tabs.ranking.next_url = None;
                state.tabs.ranking.auto_load_remaining = 0;
                return iced::Task::done(Message::LoadMore);
            }
            iced::Task::none()
        }

        // ── Color pipeline (sRGB/Hue sorts) ───────────────────────────
        Message::PaletteExtracted(id, palette) => {
            // Viewer-driven extraction (a single illust): store + persist.
            // Empty palettes (decode/fetch failures) are NOT stored — they'd
            // count as "already present" and never get reprocessed.
            if !palette.is_empty() {
                state.images.palettes.insert(id, palette);
                state.images.colors_dirty = true;
            }
            iced::Task::done(Message::FlushDisk)
        }

        Message::ColorPassPalette(id, palette) => {
            // A "process all" worker finished: store the palette, count it,
            // and refill the pool. The pass persists only when it completes —
            // the old per-palette FlushDisk re-serialized the whole library
            // and queue (and cloned them on the UI thread) thousands of
            // times, which is what froze the app during "process all".
            if !palette.is_empty() {
                state.images.palettes.insert(id, palette);
                state.images.colors_dirty = true;
            }
            state.images.color_in_flight = state.images.color_in_flight.saturating_sub(1);
            state.images.color_done = (state.images.color_done + 1).min(state.images.color_total);
            let mut tasks = super::spawn_color_workers(state);
            if !state.images.color_processing {
                tasks.push(iced::Task::done(Message::FlushDisk));
            }
            iced::Task::batch(tasks)
        }

        Message::ProcessAllColors => {
            if state.images.color_processing {
                return iced::Task::none();
            }
            // Fill in missing metadata (color palettes) for every bookmark
            // that doesn't have a non-empty palette yet. Stored-but-empty
            // entries (from a failed extraction) count as missing too.
            // Built without cloning each palette map entry (a 6000-item
            // library made the old `.cloned()` per illust a visible hitch).
            let target = super::thumbnail_target(state);
            let high_res = state.settings.high_res_thumbnails;
            let palettes = &state.images.palettes;
            let queue: Vec<(u64, String, u32)> = state
                .jobs
                .library
                .illusts
                .iter()
                .filter(|i| !palettes.get(&i.id).is_some_and(|p| !p.is_empty()))
                .filter_map(|i| {
                    super::thumbnail_source_url(i, target, high_res).map(|url| (i.id, url, target))
                })
                .collect();
            if queue.is_empty() {
                log::info!("[colors] All bookmark palettes already present");
                return iced::Task::none();
            }
            state.images.color_queue = queue;
            state.images.color_done = 0;
            state.images.color_total = state.images.color_queue.len();
            state.images.color_in_flight = 0;
            state.images.color_processing = true;
            log::info!(
                "[colors] Processing colors for {} illusts",
                state.images.color_total
            );
            iced::Task::batch(super::spawn_color_workers(state))
        }

        Message::ColorSortChanged(color) => {
            let sort = &mut state.active_tab_data_mut().sort;
            sort.color = Some(color);
            sort.custom_color_text = None;
            state.reapply_active_tab();
            iced::Task::none()
        }

        Message::CustomColorSelected => {
            let sort = &mut state.active_tab_data_mut().sort;
            if sort.custom_color_text.is_none() {
                let [r, g, b] = sort.color.unwrap_or([128, 128, 128]);
                sort.custom_color_text = Some(format!("#{r:02x}{g:02x}{b:02x}"));
                sort.color = Some([r, g, b]);
            }
            state.reapply_active_tab();
            iced::Task::none()
        }

        Message::CustomColorTextChanged(text) => {
            let sort = &mut state.active_tab_data_mut().sort;
            // An unparsable text keeps the last valid color.
            if let Some(rgb) = crate::app::sort::parse_color(&text) {
                sort.color = Some(rgb);
            }
            sort.custom_color_text = Some(text);
            state.reapply_active_tab();
            iced::Task::none()
        }

        Message::ColorLevelChanged(level) => {
            state.active_tab_data_mut().sort.color_level = level.clamp(1, 5);
            state.reapply_active_tab();
            iced::Task::none()
        }

        // ── Tag filter focus (drives the suggestion dropdown) ──────
        Message::TagFocusChanged(focused) => {
            state.tag_input_focused = focused;
            if focused {
                state.tag_dropdown_open = true;
            } else {
                // Close after a short grace window so a click on a candidate
                // (which blurs the input first) still registers before the
                // dropdown disappears.
                return iced::Task::perform(
                    async {
                        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    },
                    |_| Message::TagFocusGrace,
                );
            }
            iced::Task::none()
        }

        Message::TagFocusGrace => {
            if !state.tag_input_focused {
                state.tag_dropdown_open = false;
            }
            iced::Task::none()
        }

        // ── Image loading ───────────────────────────────────────────

        // Any message routed here by the dispatcher must be one of the above.
        _ => unreachable!("message routed to the wrong update handler"),
    }
}
