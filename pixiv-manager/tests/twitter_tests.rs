//! Provider (Twitter) integration tests — no network, no credentials.
//!
//! These drive the public `update()` reducer and the core path helpers, so
//! the mixed-tab behavior and the on-disk namespacing are covered end to end
//! without touching X or Pixiv.

use pixiv_manager::app::message::Message;
use pixiv_manager::app::state::{AppPhase, AppState, Tab};
use pixiv_manager::app::update::update;
use pixiv_manager::media::{is_twitter, native_id, provider_of, twitter_key, Provider};
use pixiv_manager::twitter::{TwitterFeedMode, TwitterPage};
use pixivj_rs::model::*;

fn init_test_env() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = tempfile::tempdir().expect("temp dir");
        std::env::set_var("PIXIV_MANAGER_CONFIG_DIR", dir.path());
        std::mem::forget(dir);
    });
}

fn tweet_illust(native: u64, day: i64, title: &str) -> Illustration {
    Illustration {
        id: twitter_key(native),
        title: Some(title.into()),
        caption: Some(title.into()),
        user: Some(Author {
            id: twitter_key(7),
            name: "artist".into(),
            account: Some("artist_x".into()),
            profile_image_urls: None,
            comment: None,
            is_followed: None,
        }),
        create_date: Some(chrono::DateTime::from_timestamp(day * 86_400, 0).unwrap()),
        page_count: Some(1),
        is_bookmarked: Some(false),
        total_bookmarks: Some(5),
        ..Illustration::default_local()
    }
}

fn pixiv_illust(id: u64, day: i64) -> Illustration {
    Illustration {
        id,
        title: Some(format!("pixiv {id}")),
        create_date: Some(chrono::DateTime::from_timestamp(day * 86_400, 0).unwrap()),
        page_count: Some(1),
        ..Illustration::default_local()
    }
}

#[test]
fn likes_page_merges_bookmarks_by_date() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    // Pixiv bookmarks: day 1 and day 3.
    state.tabs.bookmarks.illustrations = vec![pixiv_illust(1, 1), pixiv_illust(2, 3)];
    state.tabs.bookmarks.filtered = state.tabs.bookmarks.illustrations.clone();

    // Twitter likes: day 2 and day 4 (newest overall).
    let page = TwitterPage {
        items: vec![
            tweet_illust(100, 2, "tweet two"),
            tweet_illust(200, 4, "tweet four"),
        ],
        next_cursor: Some("CURSOR".into()),
    };
    let _ = update(&mut state, Message::TwitterLikesFetched(Ok(page)));

    let titles: Vec<&str> = state
        .tabs
        .bookmarks
        .illustrations
        .iter()
        .map(|i| i.title.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(
        titles,
        vec!["tweet four", "pixiv 2", "tweet two", "pixiv 1"],
        "both providers interleave newest-first"
    );
    assert_eq!(
        state.tabs.bookmarks.next_twitter_cursor.as_deref(),
        Some("CURSOR")
    );
    // The next LoadMore must keep paginating Twitter even though the Pixiv
    // cursor is exhausted-but-nonempty.
    assert!(state
        .tab(Tab::Bookmarks)
        .illustrations
        .iter()
        .any(|i| is_twitter(i.id)));
}

#[test]
fn twitter_like_result_toggles_state_in_tabs_and_viewer() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    let tweet = tweet_illust(42, 1, "liked?");
    state.tabs.twitter.illustrations = vec![tweet.clone()];
    state.tabs.bookmarks.illustrations = vec![tweet.clone()];
    state.viewer.open = true;
    state.viewer.illust_id = tweet.id;
    state.viewer.illust = Some(tweet.clone());

    let _ = update(&mut state, Message::TwitterLikeResult(tweet.id, Ok(())));

    assert!(state.tabs.twitter.illustrations[0].bookmarked());
    assert!(state.tabs.bookmarks.illustrations[0].bookmarked());
    assert!(state.viewer.illust.as_ref().unwrap().bookmarked());
}

#[test]
fn feed_mode_change_resets_the_twitter_tab() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    state.twitter.feed_mode = TwitterFeedMode::ForYou;
    state.tabs.twitter.illustrations = vec![tweet_illust(1, 1, "old")];
    state.tabs.twitter.next_url = Some("OLD_CURSOR".into());
    state.tabs.twitter.auto_load_remaining = 3;

    let _ = update(
        &mut state,
        Message::TwitterFeedModeChanged(TwitterFeedMode::Following),
    );

    assert_eq!(state.twitter.feed_mode, TwitterFeedMode::Following);
    assert!(state.tabs.twitter.illustrations.is_empty());
    assert!(state.tabs.twitter.next_url.is_none());
    assert_eq!(state.tabs.twitter.auto_load_remaining, 0);

    // Selecting the same mode again is a no-op (no reset).
    state.tabs.twitter.illustrations = vec![tweet_illust(2, 2, "kept")];
    let _ = update(
        &mut state,
        Message::TwitterFeedModeChanged(TwitterFeedMode::Following),
    );
    assert_eq!(state.tabs.twitter.illustrations.len(), 1);
}

#[test]
fn download_dirs_and_scanner_namespace_twitter_items() {
    init_test_env();
    let dir = tempfile::tempdir().expect("temp dir");
    let key = twitter_key(12345);

    // The download dir for a tweet is tagged (`tw_{native}`), Pixiv keeps the
    // bare numeric id.
    let tw = pixiv_manager::core::download_dir(dir.path(), key, "hello");
    assert_eq!(tw.file_name().unwrap().to_string_lossy(), "tw_12345");
    let pix = pixiv_manager::core::download_dir(dir.path(), 456, "hello");
    assert_eq!(pix.file_name().unwrap().to_string_lossy(), "456");
    std::fs::create_dir_all(&tw).unwrap();
    std::fs::create_dir_all(&pix).unwrap();

    let ids = pixiv_manager::core::scan_downloaded_ids(dir.path());
    assert!(
        ids.contains(&key),
        "twitter dir maps back to the namespaced id"
    );
    assert!(ids.contains(&456), "pixiv dir unchanged");
    assert_eq!(native_id(key), 12345);
    assert_eq!(provider_of(key), Provider::Twitter);
    assert_eq!(provider_of(456), Provider::Pixiv);

    // The Extra tab's local scan recognizes tweets too.
    let (local, dirs) =
        pixiv_manager::core::scan_local_illusts(dir.path(), &std::collections::HashSet::new());
    assert!(local.iter().any(|i| i.id == key));
    assert_eq!(dirs.get(&key), Some(&tw));
}

/// Live smoke test against X using the repository cookies file. Ignored by
/// default (no network in CI); run explicitly with:
/// `cargo test -p pixiv-manager --test twitter_tests -- --ignored live`.
#[test]
#[ignore]
fn live_twitter_smoke() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../twitter_cookies.txt");
    assert!(path.is_file(), "expected {}", path.display());
    let creds = pixiv_manager::twitter::cookies::load_cookies_file(&path).expect("cookies parse");
    let client = pixiv_manager::twitter::TwitterApiClient::new(creds, None).expect("client");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let profile = client.viewer().await.expect("viewer()");
        eprintln!("connected as @{} ({})", profile.handle, profile.user_id);

        let likes = client.likes(None).await.expect("likes()");
        assert!(!likes.items.is_empty(), "likes page has photo tweets");
        assert!(likes.next_cursor.is_some(), "likes page has a next cursor");
        let first = &likes.items[0];
        assert!(first.page_count() >= 1);
        assert!(!first.meta_pages.is_empty());
        assert!(first.meta_pages[0]
            .image_urls
            .original
            .as_deref()
            .unwrap()
            .contains("name=orig"));

        let tweet = client
            .tweet(pixiv_manager::media::native_id(first.id))
            .await
            .expect("tweet()");
        assert_eq!(tweet.id, first.id);

        let (replies, _cursor) = client
            .replies(pixiv_manager::media::native_id(first.id), None)
            .await
            .expect("replies()");
        eprintln!("first like has {} replies", replies.len());

        let home = client
            .home_timeline(TwitterFeedMode::ForYou, None)
            .await
            .expect("home_timeline(ForYou)");
        assert!(!home.items.is_empty(), "home feed has photo tweets");
        let following = client
            .home_timeline(TwitterFeedMode::Following, None)
            .await
            .expect("home_timeline(Following)");
        eprintln!(
            "home: {} photo tweets, following: {} photo tweets",
            home.items.len(),
            following.items.len()
        );

        // The original media URL downloads with the same browser UA the app
        // sends (twimg rejects some library agents).
        let url = first.meta_pages[0]
            .image_urls
            .original
            .clone()
            .expect("orig url");
        let resp = reqwest::Client::new()
            .get(&url)
            .header("User-Agent", pixiv_manager::twitter::USER_AGENT)
            .send()
            .await
            .expect("media request");
        assert!(
            resp.status().is_success(),
            "twimg media status {}",
            resp.status()
        );

        // Like mutation round-trip: unlike then re-like one of our likes —
        // validates both write endpoints while leaving account state as it
        // was.
        let native = pixiv_manager::media::native_id(first.id);
        client.set_like(native, false).await.expect("unlike");
        client.set_like(native, true).await.expect("re-like");
        eprintln!("like/unlike round-trip ok");
    });
}

#[test]
fn stale_feed_response_for_previous_mode_is_dropped() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    state.twitter.feed_mode = TwitterFeedMode::Following;
    state.tabs.twitter.loading = true;

    // A slow For You page lands after the user switched to Following.
    let page = TwitterPage {
        items: vec![tweet_illust(1, 1, "stale")],
        next_cursor: None,
    };
    let _ = update(
        &mut state,
        Message::TwitterFeedFetched(TwitterFeedMode::ForYou, Ok(page)),
    );
    assert!(
        state.tabs.twitter.illustrations.is_empty(),
        "the stale timeline must not leak in"
    );

    // The matching mode still lands.
    let page = TwitterPage {
        items: vec![tweet_illust(2, 2, "fresh")],
        next_cursor: None,
    };
    let _ = update(
        &mut state,
        Message::TwitterFeedFetched(TwitterFeedMode::Following, Ok(page)),
    );
    assert_eq!(state.tabs.twitter.illustrations.len(), 1);
    assert!(!state.tabs.twitter.loading);
}

#[test]
fn later_pixiv_page_keeps_the_mixed_order() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    // Twitter likes already merged (days 4 and 2).
    let page = TwitterPage {
        items: vec![
            tweet_illust(200, 4, "tweet four"),
            tweet_illust(100, 2, "tweet two"),
        ],
        next_cursor: None,
    };
    let _ = update(&mut state, Message::TwitterLikesFetched(Ok(page)));

    // An older Pixiv bookmarks page arrives next (days 3 and 1).
    let pixiv = SearchedIllusts {
        illusts: vec![pixiv_illust(3, 3), pixiv_illust(4, 1)],
        next_url: None,
        search_span_limit: None,
    };
    let _ = update(&mut state, Message::BookmarksFetched(Ok(pixiv)));

    let titles: Vec<&str> = state
        .tabs
        .bookmarks
        .illustrations
        .iter()
        .map(|i| i.title.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(
        titles,
        vec!["tweet four", "pixiv 3", "tweet two", "pixiv 4"]
    );
}

#[test]
fn duplicate_pages_are_not_appended_twice() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    let page = TwitterPage {
        items: vec![tweet_illust(1, 1, "one"), tweet_illust(2, 2, "two")],
        next_cursor: None,
    };
    let _ = update(&mut state, Message::TwitterLikesFetched(Ok(page.clone())));
    let _ = update(&mut state, Message::TwitterLikesFetched(Ok(page)));
    assert_eq!(state.tabs.bookmarks.illustrations.len(), 2);
}

#[test]
fn refresh_without_a_client_is_a_no_op() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    let _ = update(&mut state, Message::RefreshTwitterQueryIds);
    assert!(!state.twitter.query_ids_loading);
}

#[test]
fn fetched_query_ids_are_written_to_the_override_file() {
    use pixiv_manager::twitter::graphql::GraphQlConfig;

    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    state.twitter.query_ids_loading = true;
    let mut config = GraphQlConfig::compiled();
    config
        .queries
        .insert("HomeTimeline".into(), "REFRESHED".into());

    let _ = update(&mut state, Message::TwitterQueryIdsFetched(Ok(config)));

    assert!(!state.twitter.query_ids_loading);
    let config_dir = std::env::var("PIXIV_MANAGER_CONFIG_DIR").expect("test config dir");
    let path = std::path::Path::new(&config_dir).join("twitter_graphql.json");
    let raw = std::fs::read_to_string(&path).expect("override file written");
    assert!(raw.contains("REFRESHED"), "got {raw}");
    // Unchanged operations are not written, so new app defaults can apply.
    assert!(!raw.contains("HomeLatestTimeline"), "got {raw}");
}

#[test]
fn failed_query_id_fetch_clears_the_loading_flag() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    state.twitter.query_ids_loading = true;
    // The handler turns the error into a `Message::Error` task (the banner)
    // and must stop the settings spinner.
    let _ = update(
        &mut state,
        Message::TwitterQueryIdsFetched(Err(pixiv_manager::twitter::TwitterError::Internal(
            "no bundles".into(),
        ))),
    );
    assert!(!state.twitter.query_ids_loading);
}
