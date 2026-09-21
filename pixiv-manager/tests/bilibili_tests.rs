//! Bilibili provider integration tests — no network by default.
//!
//! Drives the public `update()` reducer and core path helpers for the local
//! bookmark behavior and on-disk namespacing; the ignored `live_bilibili_smoke`
//! test exercises the real API with the repository cookies.

use pixiv_manager::app::message::Message;
use pixiv_manager::app::state::{AppPhase, AppState, Tab};
use pixiv_manager::app::update::update;
use pixiv_manager::bilibili::{BilibiliFeedMode, BilibiliPage};
use pixiv_manager::media::{bilibili_key, is_bilibili, native_id, provider_of, Provider};
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

fn dynamic_illust(native: u64, day: i64, title: &str) -> Illustration {
    Illustration {
        id: bilibili_key(native),
        title: Some(title.into()),
        caption: Some(title.into()),
        user: Some(Author {
            id: bilibili_key(2105955),
            name: "桌子君".into(),
            account: None,
            profile_image_urls: None,
            comment: None,
            is_followed: Some(true),
        }),
        create_date: Some(chrono::DateTime::from_timestamp(day * 86_400, 0).unwrap()),
        page_count: Some(1),
        is_bookmarked: Some(false),
        total_bookmarks: Some(828),
        provider_data: Some(ProviderData {
            comment_oid: Some("395446267".into()),
        }),
        ..Illustration::default_local()
    }
}

#[test]
fn bookmark_toggles_local_library_and_mixed_tab() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    let item = dynamic_illust(1205134739172753431, 1, "draw");
    state.tabs.bilibili.illustrations = vec![item.clone()];
    state.active_tab = Tab::Bilibili;

    // Bookmark: added to the library and the mixed Bookmarks tab.
    let _ = update(&mut state, Message::BookmarkToggled(item.id));
    assert!(state.jobs.library.contains(item.id));
    assert!(state.jobs.library_dirty);
    assert!(state.tabs.bilibili.illustrations[0].bookmarked());
    assert!(state
        .tabs
        .bookmarks
        .illustrations
        .iter()
        .any(|i| i.id == item.id && i.bookmarked()));

    // Unbookmark: removed from both (Bilibili is locally authoritative).
    let _ = update(&mut state, Message::BookmarkToggled(item.id));
    assert!(!state.jobs.library.contains(item.id));
    assert!(!state.tabs.bilibili.illustrations[0].bookmarked());
    assert!(!state
        .tabs
        .bookmarks
        .illustrations
        .iter()
        .any(|i| i.id == item.id));
}

#[test]
fn feed_page_seeds_and_imports_likes() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    // One item already liked on Bilibili (`is_bookmarked: true` from the API),
    // one not.
    let mut liked = dynamic_illust(1, 2, "liked");
    liked.is_bookmarked = Some(true);
    let plain = dynamic_illust(2, 1, "plain");
    let page = BilibiliPage {
        items: vec![liked.clone(), plain.clone()],
        next_cursor: Some("NEXT".into()),
    };
    let _ = update(
        &mut state,
        Message::BilibiliFeedFetched(BilibiliFeedMode::Following, Ok(page)),
    );

    assert_eq!(state.tabs.bilibili.illustrations.len(), 2);
    assert_eq!(state.tabs.bilibili.next_url.as_deref(), Some("NEXT"));
    assert!(state.tabs.bilibili.illustrations[0].bookmarked());
    assert!(!state.tabs.bilibili.illustrations[1].bookmarked());
    // The already-liked post was imported into the local library.
    assert!(state.jobs.library.contains(liked.id));
    assert!(state.jobs.library_dirty);
    assert!(!state.jobs.library.contains(plain.id));
}

#[test]
fn user_id_list_parsing_and_feed_mode_reset() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    let _ = update(
        &mut state,
        Message::BilibiliUserIdsChanged("2105955, 12345678; 2105955 0 nope".into()),
    );
    assert_eq!(state.settings.bilibili_user_ids, vec![2105955, 12345678]);
    assert_eq!(
        state.bilibili.user_ids_input,
        "2105955, 12345678; 2105955 0 nope"
    );

    state.tabs.bilibili.illustrations = vec![dynamic_illust(3, 1, "old")];
    state.tabs.bilibili.next_url = Some("5".into());
    state.bilibili.users_pending = vec![(2105955, None)];
    let _ = update(
        &mut state,
        Message::BilibiliFeedModeChanged(BilibiliFeedMode::Users),
    );
    assert_eq!(state.bilibili.feed_mode, BilibiliFeedMode::Users);
    assert!(state.tabs.bilibili.illustrations.is_empty());
    assert!(state.tabs.bilibili.next_url.is_none());
    assert!(state.bilibili.users_pending.is_empty());

    // Selecting the same mode is a no-op.
    state.tabs.bilibili.illustrations = vec![dynamic_illust(4, 1, "kept")];
    let _ = update(
        &mut state,
        Message::BilibiliFeedModeChanged(BilibiliFeedMode::Users),
    );
    assert_eq!(state.tabs.bilibili.illustrations.len(), 1);
}

#[test]
fn download_dirs_and_scanner_namespace_bilibili_items() {
    init_test_env();
    let dir = tempfile::tempdir().expect("temp dir");
    let key = bilibili_key(1205134739172753431);
    let path = pixiv_manager::core::download_dir(dir.path(), key, "title");
    assert_eq!(
        path.file_name().unwrap().to_string_lossy(),
        "bl_1205134739172753431"
    );
    std::fs::create_dir_all(&path).unwrap();

    let ids = pixiv_manager::core::scan_downloaded_ids(dir.path());
    assert!(ids.contains(&key));
    assert_eq!(native_id(key), 1205134739172753431);
    assert_eq!(provider_of(key), Provider::Bilibili);
    assert!(is_bilibili(key));

    let (local, dirs) =
        pixiv_manager::core::scan_local_illusts(dir.path(), &std::collections::HashSet::new());
    assert!(local.iter().any(|i| i.id == key));
    assert_eq!(dirs.get(&key), Some(&path));
}

#[test]
fn stale_feed_response_for_previous_mode_is_dropped() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    state.bilibili.feed_mode = BilibiliFeedMode::Users;
    let page = BilibiliPage {
        items: vec![dynamic_illust(1, 1, "stale")],
        next_cursor: None,
    };
    let _ = update(
        &mut state,
        Message::BilibiliFeedFetched(BilibiliFeedMode::Following, Ok(page)),
    );
    assert!(state.tabs.bilibili.illustrations.is_empty());

    let page = BilibiliPage {
        items: vec![dynamic_illust(2, 2, "fresh")],
        next_cursor: None,
    };
    let _ = update(
        &mut state,
        Message::BilibiliFeedFetched(BilibiliFeedMode::Users, Ok(page)),
    );
    assert_eq!(state.tabs.bilibili.illustrations.len(), 1);
}

/// Live smoke test against Bilibili using the repository cookies file.
/// Ignored by default (no network in CI); run explicitly with:
/// `cargo test -p pixiv-manager --test bilibili_tests -- --ignored live`.
#[test]
#[ignore]
fn live_bilibili_smoke() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../bilibili_cookies.txt");
    assert!(path.is_file(), "expected {}", path.display());
    let creds = pixiv_manager::bilibili::cookies::load_cookies_file(&path).expect("cookies parse");
    let client = pixiv_manager::bilibili::BilibiliApiClient::new(creds, None).expect("client");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let profile = client.nav().await.expect("nav()");
        eprintln!("connected as {} ({})", profile.name, profile.user_id);

        // Fixed-user feed (the uid from the request).
        let space = client
            .space_feed(2105955, None)
            .await
            .expect("space_feed()");
        assert!(!space.items.is_empty(), "space feed has image dynamics");
        let first = &space.items[0];
        eprintln!("first create_date: {:?}", first.create_date);
        assert!(first.create_date.is_some(), "dynamic has a timestamp");
        assert!(first.meta_pages[0]
            .image_urls
            .original
            .as_deref()
            .unwrap()
            .starts_with("https://"));
        assert!(first
            .provider_data
            .as_ref()
            .and_then(|d| d.comment_oid.as_ref())
            .is_some());
        eprintln!("space feed: {} image dynamics", space.items.len());

        // Following feed pagination.
        let following = client.following_feed(1).await.expect("following_feed(1)");
        let following2 = client.following_feed(2).await.expect("following_feed(2)");
        assert!(!following.items.is_empty());
        assert!(!following2.items.is_empty());
        eprintln!(
            "following: page1 {} items, page2 {} items",
            following.items.len(),
            following2.items.len()
        );

        // Detail by id round-trips to the same namespaced id.
        let detail = client
            .dynamic(pixiv_manager::media::native_id(first.id))
            .await
            .expect("dynamic()");
        assert_eq!(detail.id, first.id);

        // Comments (page 1).
        let oid = first
            .provider_data
            .as_ref()
            .and_then(|d| d.comment_oid.clone())
            .unwrap();
        let comments = client.comments(&oid, 1).await.expect("comments()");
        eprintln!("first dynamic has {} comments", comments.comments.len());
        assert!(!comments.comments.is_empty());

        // The original image downloads over https.
        let url = first.meta_pages[0]
            .image_urls
            .original
            .clone()
            .expect("orig");
        let resp = reqwest::Client::new()
            .get(&url)
            .header("User-Agent", pixiv_manager::bilibili::USER_AGENT)
            .send()
            .await
            .expect("media request");
        assert!(resp.status().is_success(), "media status {}", resp.status());
        eprintln!(
            "media download ok ({} bytes)",
            resp.content_length().unwrap_or(0)
        );

        // Like round-trip: like then unlike, leaving account state unchanged.
        if client.can_write() {
            let native = pixiv_manager::media::native_id(first.id);
            client.set_like(native, true).await.expect("like");
            client.set_like(native, false).await.expect("unlike");
            eprintln!("like/unlike round-trip ok");
        }
    });
}

#[test]
fn bookmark_sync_keeps_locally_tracked_dynamics() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    let mut item = dynamic_illust(1205134739172753431, 1, "local");
    item.is_bookmarked = Some(true);
    let mut library = pixiv_manager::io::library::BookmarkLibrary::default();
    library.merge(vec![item.clone()]);
    state.jobs.library = library.clone();

    // A finished Pixiv/Twitter sync adopts the library; the locally tracked
    // dynamic must survive (it is part of the same library).
    let _ = update(
        &mut state,
        Message::BookmarkSyncFinished {
            total: 3,
            error: None,
            library: library.clone(),
        },
    );
    assert!(state
        .tabs
        .bookmarks
        .illustrations
        .iter()
        .any(|i| i.id == item.id && i.bookmarked()));
    assert!(state.jobs.library.contains(item.id));
}

#[test]
fn shift_click_toggles_bookmark_and_queues_download() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    let item = dynamic_illust(1205134739172753431, 1, "shift");
    state.tabs.bilibili.illustrations = vec![item.clone()];
    state.shift_pressed = true;

    let _ = update(&mut state, Message::GalleryItemClicked(item.id));
    assert!(!state.viewer.open, "shift-click must not open the viewer");
    assert!(state.jobs.library.contains(item.id));
    assert!(
        state.jobs.download_queue.remaining.contains(&item.id),
        "bookmarking a dynamic queues its images for download"
    );

    // A plain click opens the viewer as usual.
    state.shift_pressed = false;
    let _ = update(&mut state, Message::GalleryItemClicked(item.id));
    assert!(state.viewer.open);
    assert_eq!(state.viewer.illust_id, item.id);
}

#[test]
fn feed_pages_merge_into_date_order() {
    init_test_env();
    let mut state = AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    // Page 1 arrives with a repost whose original is older than the rest.
    let mut repost = dynamic_illust(10, 2, "repost (older original)");
    repost.create_date = Some(chrono::DateTime::from_timestamp(2 * 86_400, 0).unwrap());
    let page1 = BilibiliPage {
        items: vec![
            dynamic_illust(1, 5, "five"),
            repost,
            dynamic_illust(2, 4, "four"),
        ],
        next_cursor: Some("2".into()),
    };
    let _ = update(
        &mut state,
        Message::BilibiliFeedFetched(BilibiliFeedMode::Following, Ok(page1)),
    );
    // Page 2 is older.
    let page2 = BilibiliPage {
        items: vec![dynamic_illust(3, 3, "three"), dynamic_illust(4, 1, "one")],
        next_cursor: None,
    };
    let _ = update(
        &mut state,
        Message::BilibiliFeedFetched(BilibiliFeedMode::Following, Ok(page2)),
    );

    let days: Vec<i64> = state
        .tabs
        .bilibili
        .illustrations
        .iter()
        .map(|i| i.create_date.unwrap().timestamp() / 86_400)
        .collect();
    assert_eq!(days, vec![5, 4, 3, 2, 1], "newest-first across pages");
    // No duplicates from the lazy merge.
    let ids: std::collections::HashSet<u64> = state
        .tabs
        .bilibili
        .illustrations
        .iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(ids.len(), state.tabs.bilibili.illustrations.len());
}
