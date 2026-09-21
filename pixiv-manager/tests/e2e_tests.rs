//! E2E tests for onboarding flow, state transitions, and gallery rendering.
//!
//! These tests use `iced_test::simulator` to verify the UI renders correctly
//! and `update()` directly for state machine transitions.

use pixiv_manager::app::message::Message;
use pixiv_manager::app::state::{AppPhase, OnboardingStep, Tab};
use pixiv_manager::app::update::update;
use pixiv_manager::app::view::view;
use pixiv_manager::widgets::onboarding::onboarding_view;

/// An API client with a fixed token (no network; requests fail fast).
fn test_client() -> pixivj_rs::api::PixivApiClient {
    let refresher = std::sync::Arc::new(pixivj_rs::auth::LazyTokenRefresher::new(
        pixivj_rs::auth::PixivOAuthClient::new().unwrap(),
        "token".into(),
    ));
    pixivj_rs::api::PixivApiClient::new(refresher).unwrap()
}

// ── Onboarding State Transition Tests ──────────────────────────────────

#[test]
fn test_onboarding_starts_in_starting_phase() {
    let state = pixiv_manager::app::state::AppState::default();
    assert_eq!(state.phase, AppPhase::Onboarding);
    assert_eq!(state.onboarding.step, OnboardingStep::Starting);
}

#[test]
fn test_onboarding_server_ready_transitions_to_waiting_for_token() {
    let mut state = pixiv_manager::app::state::AppState::default();
    let _ = update(
        &mut state,
        Message::OnboardingServerReady("http://127.0.0.1:9999".into()),
    );
    assert_eq!(state.onboarding.step, OnboardingStep::WaitingForToken);
    assert_eq!(
        state.onboarding.url.as_deref(),
        Some("http://127.0.0.1:9999")
    );
}

#[test]
fn test_onboarding_token_submit_transitions_to_validating() {
    let mut state = pixiv_manager::app::state::AppState {
        onboarding: pixiv_manager::app::state::OnboardingState {
            step: OnboardingStep::WaitingForToken,
            ..Default::default()
        },
        ..Default::default()
    };
    let _ = update(&mut state, Message::TokenSubmitted("test_token_123".into()));
    assert_eq!(state.onboarding.step, OnboardingStep::ValidatingToken);
    assert_eq!(state.onboarding.token, "test_token_123");
}

#[test]
fn test_onboarding_empty_token_ignored() {
    let mut state = pixiv_manager::app::state::AppState {
        onboarding: pixiv_manager::app::state::OnboardingState {
            step: OnboardingStep::WaitingForToken,
            ..Default::default()
        },
        ..Default::default()
    };
    let _ = update(&mut state, Message::TokenSubmitted(String::new()));
    // Should stay on WaitingForToken — empty token is ignored
    assert_eq!(state.onboarding.step, OnboardingStep::WaitingForToken);
}

#[test]
fn test_onboarding_token_validated_transitions_to_directory() {
    let mut state = pixiv_manager::app::state::AppState {
        onboarding: pixiv_manager::app::state::OnboardingState {
            step: OnboardingStep::ValidatingToken,
            token: "valid_token".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let _ = update(
        &mut state,
        Message::TokenValidated(Ok("valid_token".into())),
    );
    assert_eq!(state.onboarding.step, OnboardingStep::SelectDirectory);
    assert_eq!(state.settings.refresh_token.as_deref(), Some("valid_token"));
}

#[test]
fn test_onboarding_token_rejected_returns_to_waiting() {
    let mut state = pixiv_manager::app::state::AppState {
        onboarding: pixiv_manager::app::state::OnboardingState {
            step: OnboardingStep::ValidatingToken,
            token: "bad_token".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let _ = update(
        &mut state,
        Message::TokenValidated(Err(pixivj_rs::error::PixivError::HttpStatus(
            401,
            "Invalid refresh token".into(),
        ))),
    );
    assert_eq!(state.onboarding.step, OnboardingStep::WaitingForToken);
    assert!(state.onboarding.token.is_empty());
}

#[test]
fn test_onboarding_directory_selected_saves_path() {
    let mut state = pixiv_manager::app::state::AppState {
        onboarding: pixiv_manager::app::state::OnboardingState {
            step: OnboardingStep::SelectDirectory,
            ..Default::default()
        },
        ..Default::default()
    };
    let _ = update(
        &mut state,
        Message::DirectorySelected(Some("/tmp/gallery".into())),
    );
    assert_eq!(
        state.settings.gallery_path.as_deref(),
        Some(std::path::Path::new("/tmp/gallery"))
    );
}

#[test]
fn test_onboarding_complete_enters_main_phase() {
    let mut state = pixiv_manager::app::state::AppState {
        onboarding: pixiv_manager::app::state::OnboardingState {
            step: OnboardingStep::SelectDirectory,
            ..Default::default()
        },
        settings: pixiv_manager::app::state::Settings {
            refresh_token: Some("token".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let _ = update(&mut state, Message::OnboardingComplete);
    assert_eq!(state.onboarding.step, OnboardingStep::Complete);
    assert_eq!(state.phase, AppPhase::Main);
}

#[test]
fn test_onboarding_full_flow_starting_to_complete() {
    let mut state = pixiv_manager::app::state::AppState::default();

    // Step 1: Server ready
    let _ = update(
        &mut state,
        Message::OnboardingServerReady("http://127.0.0.1:8080".into()),
    );
    assert_eq!(state.onboarding.step, OnboardingStep::WaitingForToken);

    // Step 2: Token submitted
    let _ = update(
        &mut state,
        Message::TokenSubmitted("my_refresh_token".into()),
    );
    assert_eq!(state.onboarding.step, OnboardingStep::ValidatingToken);

    // Step 3: Token validated
    let _ = update(
        &mut state,
        Message::TokenValidated(Ok("valid_token".into())),
    );
    assert_eq!(state.onboarding.step, OnboardingStep::SelectDirectory);

    // Step 4: Directory selected
    let _ = update(
        &mut state,
        Message::DirectorySelected(Some("/home/user/gallery".into())),
    );
    assert!(state.settings.gallery_path.is_some());

    // Step 5: Complete
    let _ = update(&mut state, Message::OnboardingComplete);
    assert_eq!(state.phase, AppPhase::Main);
    assert_eq!(state.onboarding.step, OnboardingStep::Complete);
}

// ── Tab Management Tests ───────────────────────────────────────────────

#[test]
fn test_tab_selected_changes_active_tab() {
    let mut state = pixiv_manager::app::state::AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };

    let _ = update(&mut state, Message::TabSelected(Tab::Feed));
    assert_eq!(state.active_tab, Tab::Feed);

    let _ = update(&mut state, Message::TabSelected(Tab::Bookmarks));
    assert_eq!(state.active_tab, Tab::Bookmarks);

    let _ = update(&mut state, Message::TabSelected(Tab::Search));
    assert_eq!(state.active_tab, Tab::Search);

    let _ = update(&mut state, Message::TabSelected(Tab::Extra));
    assert_eq!(state.active_tab, Tab::Extra);

    let _ = update(&mut state, Message::TabSelected(Tab::Recommend));
    assert_eq!(state.active_tab, Tab::Recommend);
}

#[test]
fn test_grid_size_changed_updates_settings() {
    let mut state = pixiv_manager::app::state::AppState::default();
    let _ = update(&mut state, Message::GridSizeChanged(300));
    assert_eq!(state.settings.grid_size, 300);

    let _ = update(&mut state, Message::GridSizeChanged(150));
    assert_eq!(state.settings.grid_size, 150);
}

// ── Viewer State Tests ─────────────────────────────────────────────────

#[test]
fn test_viewer_opened_sets_state() {
    let mut state = pixiv_manager::app::state::AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };

    let _ = update(&mut state, Message::ViewerOpened(12345));
    assert!(state.viewer.open);
    assert_eq!(state.viewer.illust_id, 12345);
    assert_eq!(state.viewer.page, 0);
    assert!(state.viewer.loading);
}

#[test]
fn test_viewer_closed_resets_state() {
    let mut state = pixiv_manager::app::state::AppState {
        viewer: pixiv_manager::app::state::ViewerState {
            open: true,
            illust_id: 12345,
            page: 3,
            loading: true,
            ..Default::default()
        },
        ..Default::default()
    };

    let _ = update(&mut state, Message::ViewerClosed);
    assert!(!state.viewer.open);
    assert_eq!(state.viewer.illust_id, 0);
    assert_eq!(state.viewer.page, 0);
    assert!(!state.viewer.loading);
}

#[test]
fn test_viewer_page_changed() {
    let mut state = pixiv_manager::app::state::AppState::default();
    let _ = update(&mut state, Message::ViewerPageChanged(2));
    assert_eq!(state.viewer.page, 2);
}

// ── Artist Page Tests ──────────────────────────────────────────────────

#[test]
fn test_artist_opened_sets_state() {
    let mut state = pixiv_manager::app::state::AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };

    let _ = update(&mut state, Message::ArtistOpened(67890));
    assert!(state.artist.open);
    assert_eq!(state.artist.id, 67890);
    assert!(state.artist.loading);
    assert!(state.artist.illusts.is_empty());
}

#[test]
fn test_artist_closed_resets_state() {
    let mut state = pixiv_manager::app::state::AppState {
        artist: pixiv_manager::app::state::ArtistState {
            open: true,
            id: 67890,
            loading: true,
            ..Default::default()
        },
        ..Default::default()
    };

    let _ = update(&mut state, Message::ArtistClosed);
    assert!(!state.artist.open);
    assert_eq!(state.artist.id, 0);
    assert!(!state.artist.loading);
}

// ── Filter Tests ───────────────────────────────────────────────────────

#[test]
fn test_search_text_changed_is_per_tab() {
    use pixiv_manager::app::filter::OptionFilterKey;

    let mut state = pixiv_manager::app::state::AppState::default();
    // The search box is per-tab: typing on Recommend must not leak into (or
    // be cleared from) the other tabs.
    let _ = update(&mut state, Message::SearchTextChanged("cat".into()));
    assert_eq!(state.tabs.recommend.filters.search_text, "cat");
    assert_eq!(state.tabs.feed.filters.search_text, "");
    assert_eq!(state.tabs.bookmarks.filters.search_text, "");
    assert_eq!(state.tabs.search.filters.search_text, "");
    assert_eq!(state.tabs.extra.filters.search_text, "");

    let _ = update(&mut state, Message::TabSelected(Tab::Search));
    let _ = update(&mut state, Message::SearchTextChanged("dog".into()));
    assert_eq!(state.tabs.search.filters.search_text, "dog");
    assert_eq!(state.tabs.recommend.filters.search_text, "cat");

    // Verify option filter toggle doesn't crash
    let _ = update(
        &mut state,
        Message::OptionFilterToggled(OptionFilterKey::R18),
    );
}

#[test]
fn test_tag_added_and_removed() {
    let mut state = pixiv_manager::app::state::AppState {
        active_tab: Tab::Recommend,
        ..Default::default()
    };

    let _ = update(&mut state, Message::TagAdded("landscape".into()));
    assert!(state
        .tabs
        .recommend
        .filters
        .selected_tags
        .contains(&"landscape".to_string()));

    let _ = update(&mut state, Message::TagAdded("landscape".into())); // dedup
    assert_eq!(state.tabs.recommend.filters.selected_tags.len(), 1);

    let _ = update(&mut state, Message::TagRemoved("landscape".into()));
    assert!(!state
        .tabs
        .recommend
        .filters
        .selected_tags
        .contains(&"landscape".to_string()));
}

#[test]
fn test_reset_filters() {
    let mut state = pixiv_manager::app::state::AppState {
        active_tab: Tab::Recommend,
        tabs: pixiv_manager::app::state::TabsState {
            recommend: pixiv_manager::app::state::GalleryTab {
                filters: pixiv_manager::app::filter::FilterState {
                    search_text: "test".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    state
        .tabs
        .recommend
        .filters
        .selected_tags
        .push("tag1".into());

    let _ = update(&mut state, Message::ResetFilters);
    assert!(state.tabs.recommend.filters.search_text.is_empty());
    assert!(state.tabs.recommend.filters.selected_tags.is_empty());
}

// ── Sort Tests ─────────────────────────────────────────────────────────

#[test]
fn test_sort_key_clicked_adds_key() {
    use pixiv_manager::app::sort::SortKey;

    let mut state = pixiv_manager::app::state::AppState {
        active_tab: Tab::Recommend,
        ..Default::default()
    };

    let _ = update(&mut state, Message::SortKeyClicked(SortKey::Id));
    assert_eq!(state.tabs.recommend.sort.active.len(), 1);
    assert_eq!(state.tabs.recommend.sort.active[0].0, SortKey::Id);
}

#[test]
fn test_multi_sort_toggle() {
    let mut state = pixiv_manager::app::state::AppState {
        active_tab: Tab::Recommend,
        ..Default::default()
    };
    assert!(!state.tabs.recommend.sort.multi_sort);

    let _ = update(&mut state, Message::MultiSortToggled);
    assert!(state.tabs.recommend.sort.multi_sort);

    let _ = update(&mut state, Message::MultiSortToggled);
    assert!(!state.tabs.recommend.sort.multi_sort);
}

#[test]
fn test_multi_sort_cycles_to_neutral_and_reorders() {
    use pixiv_manager::app::sort::{SortDirection, SortKey};

    let mut state = pixiv_manager::app::state::AppState {
        active_tab: Tab::Recommend,
        ..Default::default()
    };
    let _ = update(&mut state, Message::MultiSortToggled);
    assert!(state.tabs.recommend.sort.multi_sort);

    // Add Views (Down), cycle to Up, then back to Undefined → removed.
    let _ = update(&mut state, Message::SortKeyClicked(SortKey::Views));
    assert_eq!(
        state.tabs.recommend.sort.active,
        vec![(SortKey::Views, SortDirection::Down)]
    );
    let _ = update(&mut state, Message::SortKeyClicked(SortKey::Views));
    assert_eq!(
        state.tabs.recommend.sort.active,
        vec![(SortKey::Views, SortDirection::Up)]
    );
    // Neutral removes the rule so the user can re-select it later (changing
    // the ordering) — this is what "unselecting" means in multi-sort mode.
    let _ = update(&mut state, Message::SortKeyClicked(SortKey::Views));
    assert!(
        state.tabs.recommend.sort.active.is_empty(),
        "neutral rule is removed from the active list"
    );

    // Re-selecting appends at the end → selection order is the sort order.
    let _ = update(&mut state, Message::SortKeyClicked(SortKey::Views));
    let _ = update(&mut state, Message::SortKeyClicked(SortKey::Bookmarks));
    assert_eq!(
        state.tabs.recommend.sort.active,
        vec![
            (SortKey::Views, SortDirection::Down),
            (SortKey::Bookmarks, SortDirection::Down),
        ],
        "rules sort in the order they were selected"
    );
}

#[test]
fn test_single_sort_cycles_to_neutral() {
    use pixiv_manager::app::sort::{SortDirection, SortKey};

    let mut state = pixiv_manager::app::state::AppState {
        active_tab: Tab::Recommend,
        ..Default::default()
    };
    assert!(!state.tabs.recommend.sort.multi_sort);

    let _ = update(&mut state, Message::SortKeyClicked(SortKey::Id));
    assert_eq!(
        state.tabs.recommend.sort.active,
        vec![(SortKey::Id, SortDirection::Down)]
    );
    let _ = update(&mut state, Message::SortKeyClicked(SortKey::Id));
    assert_eq!(
        state.tabs.recommend.sort.active,
        vec![(SortKey::Id, SortDirection::Up)]
    );
    // Every sort button is a tristate: a third click un-sorts (neutral).
    let _ = update(&mut state, Message::SortKeyClicked(SortKey::Id));
    assert!(
        state.tabs.recommend.sort.active.is_empty(),
        "third click un-sorts the rule"
    );
}

#[test]
fn tag_dropdown_survives_focus_loss_for_candidate_click() {
    // Clicking a candidate blurs the text input (TagFocusChanged(false))
    // BEFORE the button's release fires TagAdded. The dropdown must not close
    // instantly or the release would be lost and no tag added.
    let mut state = pixiv_manager::app::state::AppState {
        active_tab: Tab::Recommend,
        ..Default::default()
    };
    state.tag_input_focused = true;
    state.tag_dropdown_open = true;

    // Candidate press → input blurs.
    let _ = update(&mut state, Message::TagFocusChanged(false));
    assert!(
        state.tag_dropdown_open,
        "dropdown stays open through the candidate click (grace)"
    );

    // Candidate release → TagAdded fires and adds the tag, closing the list.
    let _ = update(&mut state, Message::TagAdded("cat".into()));
    assert!(
        state
            .tabs
            .recommend
            .filters
            .selected_tags
            .contains(&"cat".to_string()),
        "tag was added to the filter list"
    );
    assert!(!state.tag_dropdown_open, "dropdown closes after adding");

    // The grace timer fires later and leaves the closed state alone.
    let _ = update(&mut state, Message::TagFocusGrace);
    assert!(!state.tag_dropdown_open);
}

#[test]
fn high_res_thumbnails_increase_target() {
    use pixiv_manager::app::state::AppState;
    let mut state = AppState::default();
    state.settings.high_res_thumbnails = false;
    let normal = pixiv_manager::app::update::thumbnail_target(&state);
    state.settings.high_res_thumbnails = true;
    let high_res = pixiv_manager::app::update::thumbnail_target(&state);
    assert!(
        high_res > normal,
        "high-res target {high_res} > normal {normal}"
    );
}

#[test]
fn grid_change_keeps_raw_url_cache() {
    use pixiv_manager::app::state::AppState;
    let mut state = AppState::default();
    // Simulate a previously-fetched raw URL entry.
    state
        .images
        .image_cache
        .insert("https://cdn/thumb.jpg", vec![1, 2, 3, 4]);
    state
        .images
        .thumbnail_cache
        .insert("https://cdn/thumb.jpg#240", vec![9, 9]);

    // Grid / quality change: reset the processed cache only.
    let _tasks = pixiv_manager::app::update::reset_thumbnail_cache(&mut state);

    assert!(
        state
            .images
            .image_cache
            .get("https://cdn/thumb.jpg")
            .is_some(),
        "raw URL cache survives grid/quality changes"
    );
    assert!(
        state
            .images
            .thumbnail_cache
            .get("https://cdn/thumb.jpg#240")
            .is_none(),
        "processed thumbnail cache is invalidated"
    );
}

#[test]
fn bookmark_downloaded_illust_skips_download() {
    // Regression: bookmarking an already-downloaded illust queued it for a
    // full re-download. It must not.
    let gallery = tempfile::tempdir().unwrap();
    let mut state = pixiv_manager::app::state::AppState {
        client: Some(test_client()),
        ..Default::default()
    };
    state.settings.gallery_path = Some(gallery.path().to_path_buf());

    // A single-page illust whose file is already on disk.
    let mut illust = pixiv_manager::test_util::mock_illust(4242);
    illust.page_count = Some(1);
    state.tabs.recommend.illustrations.push(illust.clone());
    let dir = pixiv_manager::core::download_dir(gallery.path(), 4242, illust.title());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("0.jpg"), b"already here").unwrap();
    assert!(pixiv_manager::core::is_illust_downloaded(
        gallery.path(),
        &illust
    ));

    let _tasks = update(&mut state, Message::BookmarkToggled(4242));
    assert!(
        !state.jobs.download_queue.remaining.contains(&4242),
        "downloaded illust must not be re-queued by bookmarking"
    );
}

#[test]
fn bookmark_new_illust_still_queues_download() {
    let gallery = tempfile::tempdir().unwrap();
    let mut state = pixiv_manager::app::state::AppState {
        client: Some(test_client()),
        ..Default::default()
    };
    state.settings.gallery_path = Some(gallery.path().to_path_buf());

    let illust = pixiv_manager::test_util::mock_illust(777);
    state.tabs.recommend.illustrations.push(illust);
    let _tasks = update(&mut state, Message::BookmarkToggled(777));
    assert!(
        state.jobs.download_queue.remaining.contains(&777),
        "a fresh illust is still auto-downloaded on bookmark"
    );
}

#[test]
fn artist_page_thumbnails_are_not_dropped_as_stale() {
    // Regression: the artist page spawns thumbnail fetches directly, but
    // never registered their generation target — so ThumbnailLoaded's
    // staleness guard dropped EVERY result and no artist thumbnail loaded.
    let mut state = pixiv_manager::app::state::AppState::default();
    state.artist.open = true;

    let illusts = vec![pixiv_manager::test_util::mock_illust(31337)];
    let data = pixivj_rs::model::UserIllusts {
        illusts,
        next_url: None,
        user: None,
    };
    let _tasks = update(&mut state, Message::UserIllustsFetched(Ok(data)));
    assert!(
        state.images.thumbnail_targets.contains_key(&31337),
        "fetching artist illusts registers their thumbnail target"
    );

    let target = pixiv_manager::app::update::thumbnail_target(&state);
    let _tasks = update(
        &mut state,
        Message::ThumbnailLoaded(31337, "https://cdn/x.jpg".into(), target, Ok(vec![1, 2, 3])),
    );
    assert!(
        state.images.thumbnail_handles.contains_key(&31337),
        "artist-page thumbnail results must not be dropped as stale"
    );
}

#[test]
fn download_permanently_deleted_is_skipped() {
    let mut state = pixiv_manager::app::state::AppState {
        client: Some(test_client()),
        ..Default::default()
    };
    // A deleted illust (404) → marked permanently-failed so future
    // "download all" runs skip it (and no error banner is queued).
    let _ = update(
        &mut state,
        Message::DownloadCompleted(
            123,
            Err(pixivj_rs::error::PixivError::HttpStatus(
                404,
                "Page not found".into(),
            )),
        ),
    );
    assert!(
        state.jobs.download_queue.is_failed(123),
        "deleted illust marked permanently-failed"
    );
    assert!(
        !state.jobs.download_queue.remaining.contains(&123),
        "deleted illust leaves the queue"
    );

    // A transient error is NOT permanently-failed.
    let mut state2 = pixiv_manager::app::state::AppState {
        client: Some(test_client()),
        ..Default::default()
    };
    let _ = update(
        &mut state2,
        Message::DownloadCompleted(
            456,
            Err(pixivj_rs::error::PixivError::HttpStatus(
                500,
                "server error".into(),
            )),
        ),
    );
    assert!(!state2.jobs.download_queue.is_failed(456));
}

#[test]
fn download_all_starts_a_fresh_job() {
    // A stale lifetime `total` from previous buggy runs must not leak into a
    // new "download all" — the queue resets so progress reflects this run.
    let gallery = tempfile::tempdir().unwrap();
    let mut state = pixiv_manager::app::state::AppState {
        client: Some(test_client()),
        settings: pixiv_manager::app::state::Settings {
            gallery_path: Some(gallery.path().to_path_buf()),
            ..Default::default()
        },
        jobs: pixiv_manager::app::state::JobsState {
            library: {
                let mut lib = pixiv_manager::io::library::BookmarkLibrary::default();
                lib.illusts = (1..=10)
                    .map(pixiv_manager::test_util::mock_illust)
                    .collect();
                lib
            },
            download_queue: pixiv_manager::io::library::DownloadQueue {
                total: 4466, // stale from historical runs
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };

    let _ = update(&mut state, Message::StartDownloadAll);
    // Nothing is on disk → all 10 enqueued; the stale 4466 was reset.
    assert_eq!(
        state.jobs.download_queue.total, 10,
        "fresh job counts only this run's items"
    );
    assert_eq!(state.jobs.download_queue.remaining.len(), 10);
}

#[test]
fn download_all_skips_permanently_failed_ids() {
    let gallery = tempfile::tempdir().unwrap();
    let mut state = pixiv_manager::app::state::AppState {
        client: Some(test_client()),
        settings: pixiv_manager::app::state::Settings {
            gallery_path: Some(gallery.path().to_path_buf()),
            ..Default::default()
        },
        jobs: pixiv_manager::app::state::JobsState {
            library: {
                let mut lib = pixiv_manager::io::library::BookmarkLibrary::default();
                lib.illusts = (1..=10)
                    .map(pixiv_manager::test_util::mock_illust)
                    .collect();
                lib
            },
            download_queue: pixiv_manager::io::library::DownloadQueue {
                failed: vec![3, 7],
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };

    let _ = update(&mut state, Message::StartDownloadAll);
    assert_eq!(
        state.jobs.download_queue.total, 8,
        "failed ids not re-enqueued"
    );
    assert!(
        !state.jobs.download_queue.remaining.contains(&3)
            && !state.jobs.download_queue.remaining.contains(&7),
        "permanently-failed ids stay skipped"
    );
}

#[test]
fn viewer_page_step_navigates_within_bounds() {
    let mut state = pixiv_manager::app::state::AppState {
        client: Some(test_client()),
        ..Default::default()
    };
    state.viewer.open = true;
    state.viewer.total_pages = 3;
    state.viewer.page = 0;

    // Forward.
    let _ = update(&mut state, Message::ViewerPageStep(1));
    assert_eq!(state.viewer.page, 1, "next page");
    // Back.
    let _ = update(&mut state, Message::ViewerPageStep(-1));
    assert_eq!(state.viewer.page, 0, "prev page");
    // Bounds: can't go below 0.
    let _ = update(&mut state, Message::ViewerPageStep(-1));
    assert_eq!(state.viewer.page, 0, "clamped at first page");
    // Bounds: can't exceed the last page.
    let _ = update(&mut state, Message::ViewerPageStep(10));
    assert_eq!(state.viewer.page, 2, "clamped at last page");
}

#[test]
fn download_manager_priority_reorders_and_respawns() {
    let mut state = pixiv_manager::app::state::AppState {
        client: Some(test_client()),
        ..Default::default()
    };
    state.jobs.download_queue.remaining = vec![1, 2, 3];
    state.jobs.download_queue.total = 3;

    let _ = update(&mut state, Message::DownloadPriorityUp(3));
    assert_eq!(
        state.jobs.download_queue.remaining,
        vec![1, 3, 2],
        "id 3 swapped one slot earlier"
    );

    let _ = update(&mut state, Message::DownloadPriorityDown(3));
    assert_eq!(state.jobs.download_queue.remaining, vec![1, 2, 3]);

    // The manager panel opens/closes.
    let _ = update(&mut state, Message::ToggleDownloadManager);
    assert!(state.download_manager_open);
    let _ = update(&mut state, Message::ToggleDownloadManager);
    assert!(!state.download_manager_open);
}

// ── Download Tracker Tests ─────────────────────────────────────────────

#[test]
fn test_download_started_and_completed() {
    let mut state = pixiv_manager::app::state::AppState {
        client: Some(test_client()),
        ..Default::default()
    };

    let _ = update(&mut state, Message::DownloadStarted(111));
    assert!(state.jobs.downloads.is_downloading(111));

    let _ = update(&mut state, Message::DownloadCompleted(111, Ok(())));
    assert!(!state.jobs.downloads.is_downloading(111));
}

#[test]
fn test_download_dedup() {
    let mut state = pixiv_manager::app::state::AppState {
        client: Some(test_client()),
        ..Default::default()
    };

    let _ = update(&mut state, Message::DownloadStarted(222));
    let _ = update(&mut state, Message::DownloadStarted(222)); // dedup
    assert!(state.jobs.downloads.is_downloading(222));
}

// ── Window Resize Test ─────────────────────────────────────────────────

#[test]
fn test_window_resized_does_not_crash() {
    let mut state = pixiv_manager::app::state::AppState::default();
    let _ = update(
        &mut state,
        Message::WindowResized {
            width: 1024.0,
            height: 768.0,
        },
    );
    // Should not crash, no state change to verify
}

// ── Health Check Tests ─────────────────────────────────────────────────

// ── Onboarding View Rendering Tests (using iced_test simulator) ────────

#[test]
fn test_onboarding_view_renders_starting() {
    use iced_test::simulator;

    let state = pixiv_manager::app::state::AppState::default();
    // state.onboarding.step is Starting by default
    let mut ui = simulator(onboarding_view(&state));
    // Should render without panic
    assert!(
        ui.find("Pixiv Manager — Setup").is_ok() || ui.find("Starting setup server...").is_ok()
    );
}

#[test]
fn test_onboarding_view_renders_waiting_for_token() {
    use iced_test::simulator;

    let state = pixiv_manager::app::state::AppState {
        onboarding: pixiv_manager::app::state::OnboardingState {
            step: OnboardingStep::WaitingForToken,
            url: Some("http://127.0.0.1:9999".into()),
            ..Default::default()
        },
        ..Default::default()
    };

    let mut ui = simulator(onboarding_view(&state));
    // Should show the URL or instructions
    assert!(ui.find("Log in with Pixiv").is_ok() || ui.find("Continue").is_ok());
}

#[test]
fn test_onboarding_view_renders_validating() {
    use iced_test::simulator;

    let state = pixiv_manager::app::state::AppState {
        onboarding: pixiv_manager::app::state::OnboardingState {
            step: OnboardingStep::ValidatingToken,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut ui = simulator(onboarding_view(&state));
    // Should show validating/progress indicator
    assert!(
        ui.find("Verifying token with Pixiv...").is_ok()
            || ui.find("Pixiv Manager — Setup").is_ok()
    );
}

#[test]
fn test_onboarding_view_renders_select_directory() {
    use iced_test::simulator;

    let state = pixiv_manager::app::state::AppState {
        onboarding: pixiv_manager::app::state::OnboardingState {
            step: OnboardingStep::SelectDirectory,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut ui = simulator(onboarding_view(&state));
    // Should show directory selection
    assert!(
        ui.find("Select where to store illustrations:").is_ok()
            || ui.find("Choose Directory...").is_ok()
    );
}

#[test]
fn test_onboarding_view_renders_complete() {
    use iced_test::simulator;

    let state = pixiv_manager::app::state::AppState {
        onboarding: pixiv_manager::app::state::OnboardingState {
            step: OnboardingStep::Complete,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut ui = simulator(onboarding_view(&state));
    // Should show completion
    assert!(
        ui.find("Setup complete! Loading...").is_ok() || ui.find("Pixiv Manager — Setup").is_ok()
    );
}

// ── Main View Rendering Test ────────────────────────────────────────────

#[test]
fn test_main_view_renders_without_crash() {
    use iced_test::simulator;

    let state = pixiv_manager::app::state::AppState {
        phase: AppPhase::Main,
        ..Default::default()
    };
    // No client — should still render (empty gallery)
    let mut ui = simulator(view(&state));
    // The tab bar must render, proving the view built without crashing.
    assert!(ui.find("Recommend").is_ok() || ui.find("Bookmarks").is_ok());
}

// ── Config Persistence Tests ───────────────────────────────────────────

#[test]
fn test_config_roundtrip() {
    use pixiv_manager::app::state::Settings;

    let settings = Settings {
        refresh_token: Some("test_token_abc".into()),
        gallery_path: Some(std::path::PathBuf::from("/tmp/test_gallery")),
        grid_size: 250,
        proxy: None,
        ..Default::default()
    };

    let toml_str = toml::to_string_pretty(&settings).expect("serialize");
    let restored: Settings = toml::from_str(&toml_str).expect("deserialize");

    assert_eq!(restored.refresh_token, settings.refresh_token);
    assert_eq!(restored.gallery_path, settings.gallery_path);
    assert_eq!(restored.grid_size, settings.grid_size);
}

// ── Onboarding Server Tests ────────────────────────────────────────────

#[test]
fn test_onboarding_server_starts_and_returns_url() {
    let (url, _rx) = pixiv_manager::onboarding_server::start_server("https://example.com/login");
    assert!(
        url.starts_with("http://127.0.0.1:"),
        "URL should be http://127.0.0.1:PORT, got: {}",
        url
    );
}

#[test]
fn test_onboarding_server_receives_token() {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let (url, rx) = pixiv_manager::onboarding_server::start_server("https://example.com/login");
    let addr = url.strip_prefix("http://").unwrap();

    // The token endpoint carries a per-session nonce path segment. Fetch the
    // served page and discover it, exactly as a browser would.
    let mut stream = TcpStream::connect(addr).expect("connect to server");
    let get = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        addr
    );
    stream.write_all(get.as_bytes()).expect("write GET");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let html = String::from_utf8_lossy(&response);
    let token_path = html
        .lines()
        .find_map(|line| {
            let start = line.find("/token/")?;
            let end = line[start..].find('\'')?;
            Some(line[start..start + end].to_string())
        })
        .unwrap_or_else(|| panic!("nonce path not found in served HTML"));

    // POST the token to the discovered nonce path.
    let mut stream = TcpStream::connect(addr).expect("connect to server");
    let body = "test_token_from_test";
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
        token_path,
        addr,
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).expect("write");

    // Read response (don't care about content, just that server responds)
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);

    // Server should have sent the token through the channel
    let received = rx.recv().expect("should receive token");
    assert_eq!(received, "test_token_from_test");
}

// ── QoL: error banner ──────────────────────────────────────────────────

#[test]
fn test_error_dismiss_clears_banner() {
    let mut state = pixiv_manager::app::state::AppState::default();
    let _ = update(&mut state, Message::Error("boom".into()));
    assert_eq!(state.last_error.as_deref(), Some("boom"));
    let _ = update(&mut state, Message::DismissError);
    assert!(state.last_error.is_none());
}

// ── QoL: viewer page navigation (keyboard + jump box) ─────────────────

#[test]
fn test_viewer_page_navigation_clamps() {
    use pixiv_manager::app::state::ViewerPageNav;
    let mut state = pixiv_manager::app::state::AppState::default();
    state.viewer.open = true;
    state.viewer.total_pages = 5;

    // Prev at page 0 stays clamped at 0.
    let _ = update(&mut state, Message::ViewerPageNavigate(ViewerPageNav::Prev));
    assert_eq!(state.viewer.page, 0);

    // Next advances and mirrors into the jump box display.
    let _ = update(&mut state, Message::ViewerPageNavigate(ViewerPageNav::Next));
    assert_eq!(state.viewer.page, 1);
    assert_eq!(state.viewer.page_display, "2");

    let _ = update(&mut state, Message::ViewerPageNavigate(ViewerPageNav::Last));
    assert_eq!(state.viewer.page, 4);
    assert_eq!(state.viewer.page_display, "5");

    let _ = update(
        &mut state,
        Message::ViewerPageNavigate(ViewerPageNav::First),
    );
    assert_eq!(state.viewer.page, 0);
}

#[test]
fn test_viewer_page_submitted_jumps_and_clears_input() {
    let mut state = pixiv_manager::app::state::AppState::default();
    state.viewer.open = true;
    state.viewer.total_pages = 6;

    state.viewer.page_input = "4".into();
    let _ = update(&mut state, Message::ViewerPageSubmitted);
    assert_eq!(state.viewer.page, 3, "input is 1-based");
    assert!(state.viewer.page_input.is_empty());

    // Out-of-range input is ignored (page unchanged, input kept for fixing).
    state.viewer.page_input = "99".into();
    let _ = update(&mut state, Message::ViewerPageSubmitted);
    assert_eq!(state.viewer.page, 3);
    assert_eq!(state.viewer.page_input, "99");
}

#[test]
fn test_viewer_page_input_keeps_digits_only() {
    let mut state = pixiv_manager::app::state::AppState::default();
    let _ = update(&mut state, Message::ViewerPageInputChanged("12a3".into()));
    assert_eq!(state.viewer.page_input, "123");
}

// ── QoL: download-all cancellation ─────────────────────────────────────

#[test]
fn test_cancel_download_all_clears_remaining_queue() {
    let mut state = pixiv_manager::app::state::AppState::default();
    state.jobs.download_job.running = true;
    state.jobs.download_queue.total = 5;
    for id in [1u64, 2, 3, 4, 5] {
        state.jobs.download_queue.enqueue(id);
    }
    assert_eq!(state.jobs.download_queue.remaining.len(), 5);

    let _ = update(&mut state, Message::CancelDownloadAll);
    assert!(
        state.jobs.download_queue.remaining.is_empty(),
        "queued-but-unstarted items are dropped"
    );
    assert!(
        !state.jobs.download_job.running,
        "job indicator clears when nothing is in flight"
    );
    assert!(state.jobs.queue_dirty, "the cleared queue is persisted");
}

// ── QoL: server-side search preferences ────────────────────────────────

#[test]
fn test_search_api_prefs_restart_the_search() {
    use pixivj_rs::model::{SearchTarget, SortOption};

    let mut state = pixiv_manager::app::state::AppState::default();
    state.tabs.search.query = Some("ganyu".into());
    state.tabs.search.next_url = Some("https://next".into());
    state
        .tabs
        .search
        .illustrations
        .push(pixiv_manager::test_util::mock_illust(1));

    let _ = update(
        &mut state,
        Message::ApiSortChanged(Some(SortOption::DateAsc)),
    );
    assert_eq!(state.tabs.search.search_api.sort, Some(SortOption::DateAsc));

    let _ = update(
        &mut state,
        Message::ApiTargetChanged(Some(SearchTarget::Text)),
    );
    assert_eq!(
        state.tabs.search.search_api.target,
        Some(SearchTarget::Text)
    );

    let _ = update(&mut state, Message::ApiTranslatedChanged(true));
    assert!(state.tabs.search.search_api.translated);

    // The pending search was reset so LoadMore refetches from page 1.
    assert!(state.tabs.search.illustrations.is_empty());
    assert!(state.tabs.search.next_url.is_none());
}

// ── QoL: session persistence roundtrip ─────────────────────────────────

#[test]
fn settings_no_longer_persist_filters_or_sort() {
    // Filters/sort are intentionally NOT persisted (a prefilled filter box on
    // launch was a bug). The serialized config must not contain a session
    // table.
    let settings = pixiv_manager::app::state::Settings::default();
    let toml = toml::to_string(&settings).unwrap();
    assert!(!toml.contains("[session]"), "no persisted session table");
    assert!(!toml.contains("search_text"), "no persisted search text");
    assert!(!toml.contains("selected_tags"), "no persisted tags");
}
