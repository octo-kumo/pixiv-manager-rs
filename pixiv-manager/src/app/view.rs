use crate::app::message::Message;
use crate::app::state::{AppPhase, AppState, GalleryTab, Tab, ThemePreference};
use crate::theme;
use crate::widgets::{artist, control_pane, gallery, onboarding, settings, viewer};

/// Stable per-tab identity passed to the virtual gallery so a tab switch
/// resets its scroll to the top (the widget tree would otherwise keep the
/// previous tab's offset while the row-build window was already reset).
fn tab_scroll_key(tab: Tab) -> u64 {
    (tab as u8 as u64) + 1
}

/// A vertically-centered full-size panel used by the empty/loading states.
fn centered_panel<'a>(el: iced::widget::Column<'a, Message>) -> iced::Element<'a, Message> {
    iced::widget::container(el.spacing(8).align_x(iced::Alignment::Center))
        .center_x(iced::Length::Fill)
        .center_y(iced::Length::Fill)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into()
}

pub fn view(state: &AppState) -> iced::Element<'_, Message> {
    log::trace!(
        "[view] phase={:?} tab={:?} client={}",
        state.phase,
        state.active_tab,
        state.client.is_some()
    );
    // Resolved once per frame; borrowed by widgets that need theme-aware
    // text colors (their lifetimes are untied to the element).
    let theme = theme::theme(state);
    if state.phase == AppPhase::Onboarding {
        log::trace!(
            "[view] rendering onboarding (step={:?})",
            state.onboarding.step
        );
        return onboarding::onboarding_view(state);
    }

    let tab_bar = iced::widget::row![
        tab_button("Recommend", Tab::Recommend, state.active_tab),
        tab_button("Feed", Tab::Feed, state.active_tab),
        // The three Pixiv tabs stay together: leaderboard right after the
        // recommended/follow feeds.
        tab_button("Leaderboard", Tab::Ranking, state.active_tab),
        tab_button("Twitter", Tab::Twitter, state.active_tab),
        tab_button("Bilibili", Tab::Bilibili, state.active_tab),
        tab_button("Bookmarks", Tab::Bookmarks, state.active_tab),
        tab_button("Search", Tab::Search, state.active_tab),
        tab_button("Extra", Tab::Extra, state.active_tab),
        iced::widget::Space::new().width(iced::Length::Fill),
        theme_button(state.settings.theme),
        help_button(),
        settings_button(),
    ]
    .spacing(2)
    .padding(iced::Padding::from([2, 6]));

    let active = state.active_tab_data();
    let (illusts, filters, sort) = (&active.filtered, &active.filters, &active.sort);

    let min_tile = state.settings.grid_size.max(1);
    // Responsive grid: columns are derived from the window width with a
    // per-tile minimum, and tiles expand to fill the row so there's no empty
    // space on the right. A new column is added once the width crosses the
    // threshold (tiles would otherwise get too small).
    let available_width = if state.window_size.0 > 0.0 {
        state.window_size.0
    } else {
        1600.0
    };
    let column_count = {
        let max_cols = (available_width / min_tile as f32).floor().max(1.0) as u32;
        // Show at least one column; cap so tiles don't get absurdly small.
        max_cols.clamp(1, 12)
    };
    let controls = control_pane::control_pane(
        filters,
        sort,
        state.show_advanced,
        state.active_tab == Tab::Search,
        state.active_tab == Tab::Bookmarks,
        // Continuous loading toggle: the Search tab plus the paginated
        // Pixiv tabs (Recommended/Feed/Leaderboard).
        matches!(
            state.active_tab,
            Tab::Search | Tab::Recommend | Tab::Feed | Tab::Ranking
        ),
        active.auto_load,
        active.illustrations.len(),
        &state.autocomplete_tags,
        &state.gallery_tags,
        state.tag_dropdown_open,
        &state.gallery_tools,
        state.autocomplete_selected,
        if state.active_tab == Tab::Search {
            Some(&state.tabs.search.search_api)
        } else {
            None
        },
    );
    let parallax = crate::widgets::parallax_image::ParallaxOptions {
        enabled: state.settings.parallax,
        mouse: state.settings.parallax_mouse,
        scroll: state.settings.parallax_scroll,
        mode: state.settings.parallax_mode,
    };
    let tile_text = crate::widgets::illust_info::TileText {
        font_size: state.settings.gallery_font_size,
        style: state.settings.gallery_text_style,
        rows: state.settings.tile_rows,
        avg: None,
    };
    // Average tile colors (for the "invert" text style), computed when
    // thumbnails are processed. Locked for the view build; writers are
    // background thumbnail tasks with short critical sections.
    let thumbnail_avg = state
        .images
        .fetcher
        .thumbnail_avg
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let content = gallery::gallery(
        illusts,
        available_width,
        column_count,
        &state.images.thumbnail_handles,
        parallax,
        state.gallery_scroll,
        viewport_height(state),
        &active.downloaded,
        &state.jobs.downloads,
        tile_text,
        &thumbnail_avg,
        min_tile as f32,
        // Content identity: a tab switch restores the tab's saved scroll.
        tab_scroll_key(state.active_tab),
        active.scroll,
        // Bookmarked items: dim/hide everywhere except the Bookmarks tab
        // (dimming only; "hide" is done through the filter in update).
        state.settings.bookmarked_display == crate::app::state::BookmarkedDisplay::Dim
            && state.active_tab != Tab::Bookmarks,
        state.settings.blur_r18,
        &state.hovered_tiles,
    );

    // ── Empty-state differentiation ─────────────────────────────────
    // Loading, "no matches for the active filters", and truly-empty tabs get
    // distinct panels instead of one generic "No illustrations".
    let content: iced::Element<'_, Message> =
        if active.filtered.is_empty() && active.error.is_none() {
            if active.loading {
                centered_panel(iced::widget::column![
                    iced::widget::text("⏳ Loading…").size(15)
                ])
            } else if state.active_tab == Tab::Twitter && !state.twitter.status.is_ready() {
                twitter_status_panel(state, &theme)
            } else if state.active_tab == Tab::Bilibili
                && (!state.bilibili.status.is_ready()
                    || (state.bilibili.feed_mode == crate::bilibili::BilibiliFeedMode::Users
                        && state.settings.bilibili_user_ids.is_empty()))
            {
                bilibili_status_panel(state, &theme)
            } else if filters.is_active() {
                centered_panel(iced::widget::column![
                    iced::widget::text("No matches").size(15),
                    iced::widget::text("Nothing fits the current filters.")
                        .size(12)
                        .color(theme::muted(&theme)),
                    iced::widget::button(iced::widget::text("⌫ Clear filters").size(12))
                        .on_press(Message::ResetFilters)
                        .padding(iced::Padding::from([4, 12])),
                ])
            } else {
                let hint = match state.active_tab {
                    Tab::Search => "Type a search above and press Enter.",
                    Tab::Extra => "No local illustrations found in the gallery directory.",
                    _ => "Nothing here yet.",
                };
                centered_panel(iced::widget::column![iced::widget::text(hint)
                    .size(13)
                    .color(theme::muted(&theme))])
            }
        } else {
            content
        };

    // ── Bookmarks tab toolbar: library sync + download-all job ───────
    let content: iced::Element<'_, Message> = if state.active_tab == Tab::Bookmarks {
        iced::widget::column![bookmark_toolbar(state), content,].into()
    } else {
        content
    };

    // ── Twitter tab toolbar: timeline mode + connection state ──────
    let content: iced::Element<'_, Message> = if state.active_tab == Tab::Twitter {
        iced::widget::column![twitter_toolbar(state, &theme), content].into()
    } else {
        content
    };

    // ── Bilibili tab toolbar: feed mode + connection state ─────────
    let content: iced::Element<'_, Message> = if state.active_tab == Tab::Bilibili {
        iced::widget::column![bilibili_toolbar(state, &theme), content].into()
    } else {
        content
    };

    // ── Leaderboard (ranking) toolbar: mode + date selectors ────────
    let content: iced::Element<'_, Message> = if state.active_tab == Tab::Ranking {
        iced::widget::column![ranking_toolbar(&state.tabs.ranking), content,].into()
    } else {
        content
    };

    // ── Per-tab fetch error: inline panel/strip with retry ──────────
    // Tab-scoped failures (`GalleryTab.error`) are shown where they happen:
    // as a full replacement panel when nothing loaded yet, or as a strip
    // above the tiles when only a pagination step failed.
    let content: iced::Element<'_, Message> = if let Some(error) = &active.error {
        let retry = || {
            iced::widget::button(iced::widget::text("Retry").size(12))
                .on_press(Message::LoadMore)
                .padding(iced::Padding::from([4, 12]))
        };
        let error_style = |_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Color::from_rgb(0.30, 0.10, 0.10).into()),
            border: iced::Border {
                radius: 4.0.into(),
                ..Default::default()
            },
            text_color: Some(iced::Color::from_rgb(1.0, 0.92, 0.92)),
            ..iced::widget::container::Style::default()
        };
        if active.filtered.is_empty() {
            // Nothing loaded: the error replaces the empty grid.
            iced::widget::container(
                iced::widget::column![
                    iced::widget::text("⚠ Something went wrong").size(15),
                    iced::widget::text(error.as_str()).size(12),
                    retry(),
                ]
                .spacing(10)
                .align_x(iced::Alignment::Center),
            )
            .center_x(iced::Length::Fill)
            .center_y(iced::Length::Fill)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .style(error_style)
            .into()
        } else {
            // Mid-list failure: keep the loaded tiles, surface a retry strip.
            iced::widget::column![
                iced::widget::container(
                    iced::widget::row![iced::widget::text(format!("⚠ {error}")).size(12), retry(),]
                        .spacing(8)
                        .align_y(iced::Alignment::Center)
                )
                .width(iced::Length::Fill)
                .padding([4, 8])
                .style(error_style),
                content,
            ]
            .into()
        }
    } else {
        content
    };

    let main: iced::Element<'_, Message> = iced::widget::column![tab_bar, controls, content].into();

    // ── Overlay stack ───────────────────────────────────────────────
    // IMPORTANT: the root is ALWAYS a Stack whose first child is the gallery
    // `main`. iced diffs the widget tree by index, so keeping `main` at index
    // 0 means opening/closing an overlay only appends/removes *trailing*
    // children — the gallery's own tree state (scroll offset, parallax
    // springs, scrollbar drag) survives. Before this, the root type flipped
    // between Column and Stack, which discarded the whole tree and reset the
    // gallery to the top (losing the scroll position) and re-initialized the
    // parallax tiles (causing the "flash to a random position").
    let mut layers: Vec<iced::Element<'_, Message>> = vec![main];

    // ── Debug overlay (F3): floating, half-transparent panel over the gallery ──
    if state.debug_overlay && !state.viewer.open && !state.artist.open {
        let data = crate::widgets::debug_overlay::DebugData {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            cores: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            mem_used_mb: memory_used_mb(),
            mem_total_mb: memory_total_mb(),
            fps: 0.0,
            gallery_items: illusts.len(),
            thumbnails: state.images.thumbnail_handles.len(),
            thumb_attempts: state
                .images
                .fetcher
                .thumbnail_attempts
                .load(std::sync::atomic::Ordering::Relaxed),
            thumb_cache_hits: state
                .images
                .fetcher
                .thumbnail_cache_hits
                .load(std::sync::atomic::Ordering::Relaxed),
            cache_mb: state.images.image_cache.memory_bytes() as f64 / (1024.0 * 1024.0),
            downloads: state.jobs.download_queue.done(),
            download_speed: state.jobs.download_speeds.values().sum::<f64>() / (1024.0 * 1024.0),
            queue_done: state.jobs.download_queue.done(),
            queue_total: state.jobs.download_queue.total,
            sync_loaded: state.jobs.bookmark_sync.done,
            sync_total: state.jobs.bookmark_sync.total,
            rate_limit_wait: rate_limit_remaining(state)
                .map(|d| d.as_secs_f32())
                .unwrap_or(0.0),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        layers.push(
            iced::widget::container(crate::widgets::debug_overlay::debug_overlay(data))
                .align_top(iced::Length::Fill)
                .align_right(iced::Length::Fill)
                .padding(8)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into(),
        );
    }

    // ── Artist page overlay ─────────────────────────────────────────
    // Pushed BEFORE the viewer so a tile clicked inside the artist page opens
    // the detail viewer ON TOP of the artist page (not hidden behind it).
    if state.artist.open {
        let width = if state.window_size.0 > 0.0 {
            state.window_size.0
        } else {
            1600.0
        };
        layers.push(artist::artist_page(
            state.artist.user.as_ref(),
            state.artist.pfp.as_ref(),
            &state.artist.illusts,
            width,
            state.settings.grid_size.max(1) as f32,
            &state.images.thumbnail_handles,
            &state.jobs.downloads,
            tile_text,
            &thumbnail_avg,
            state.artist.loading,
            parallax,
            state.settings.bookmarked_display == crate::app::state::BookmarkedDisplay::Dim,
            state.settings.blur_r18,
            &state.hovered_tiles,
        ));
    }

    // ── Viewer overlay ──────────────────────────────────────────────
    if state.viewer.open {
        layers.push(viewer::viewer(state));
    }

    // ── Settings screen overlay ─────────────────────────────────────
    if state.settings_open {
        layers.push(settings::settings_screen(state));
    }

    // ── Keyboard-shortcuts overlay (F1) ─────────────────────────────
    if state.show_shortcuts {
        layers.push(crate::widgets::shortcuts::shortcuts_overlay(&theme));
    }

    // ── Help screen (the "?" button): usage, hotkeys, author, license ──
    if state.show_help {
        layers.push(crate::widgets::help::help_screen(&theme));
    }

    // ── Download-manager overlay ────────────────────────────────────
    if state.download_manager_open {
        layers.push(crate::widgets::download_manager::download_manager(state));
    }

    // ── Global error banner: floats above every overlay until dismissed ──
    if let Some(error) = &state.last_error {
        layers.push(
            iced::widget::container(
                iced::widget::row![
                    iced::widget::text(format!("⚠ {error}")).size(13),
                    iced::widget::Space::new().width(iced::Length::Fill),
                    iced::widget::button(iced::widget::text("✕").size(12))
                        .on_press(Message::DismissError)
                        .padding(iced::Padding::from([2, 6])),
                ]
                .spacing(8),
            )
            .align_top(iced::Length::Fill)
            .padding(8)
            .style(|_theme: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Color::from_rgb(0.30, 0.10, 0.10).into()),
                text_color: Some(iced::Color::from_rgb(1.0, 0.92, 0.92)),
                ..iced::widget::container::Style::default()
            })
            .into(),
        );
    }

    iced::widget::Stack::with_children(layers).into()
}

fn tab_button(label: &str, tab: Tab, active: Tab) -> iced::Element<'_, Message> {
    let is_active = tab == active;
    // Segmented-tab look: active tab gets a subtle panel background + an
    // accent underline; inactive tabs are dimmed.
    let content = iced::widget::column![
        iced::widget::container(iced::widget::text(label).size(12))
            .padding(iced::Padding::from([3, 10])),
        iced::widget::container(if is_active {
            iced::widget::container(iced::widget::text("")).width(iced::Length::Fill)
        } else {
            iced::widget::container(iced::widget::text(""))
        })
        .height(2)
        .width(iced::Length::Fill)
        .style(move |theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(if is_active {
                crate::theme::accent(theme)
            } else {
                iced::Color::TRANSPARENT
            })),
            ..Default::default()
        }),
    ]
    .spacing(0);

    iced::widget::button(content)
        .on_press(Message::TabSelected(tab))
        .padding(iced::Padding::ZERO)
        .style(move |theme, _| iced::widget::button::Style {
            background: Some(iced::Background::Color(if is_active {
                iced::Color {
                    a: 0.12,
                    ..crate::theme::accent(theme)
                }
            } else {
                iced::Color::TRANSPARENT
            })),
            text_color: if is_active {
                crate::theme::fg(theme)
            } else {
                crate::theme::muted(theme)
            },
            border: iced::Border {
                radius: 6.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// Toolbar for the Bookmarks tab: sync the local library and download all
/// bookmarked images (both through the shared job queue), with progress bars.
fn bookmark_toolbar(state: &AppState) -> iced::Element<'_, Message> {
    let mut children: Vec<iced::Element<'_, Message>> = Vec::new();

    let sync_btn: iced::Element<'_, Message> = if state.jobs.bookmark_sync.running {
        iced::widget::button(iced::widget::text("✕ Stop sync").size(12))
            .on_press(Message::CancelBookmarkSync)
            .into()
    } else {
        iced::widget::button(iced::widget::text("🔄 Sync bookmarks").size(12))
            .on_press(Message::StartBookmarkSync)
            .into()
    };
    let download_btn: iced::Element<'_, Message> =
        if state.jobs.download_job.running || !state.jobs.download_queue.is_finished() {
            iced::widget::button(iced::widget::text("✕ Stop downloads").size(12))
                .on_press(Message::CancelDownloadAll)
                .into()
        } else {
            iced::widget::button(iced::widget::text("⬇ Download all").size(12))
                .on_press(Message::StartDownloadAll)
                .into()
        };
    let manage_btn: iced::Element<'_, Message> =
        iced::widget::button(iced::widget::text("Manage").size(12))
            .on_press(Message::ToggleDownloadManager)
            .into();
    let process_btn: iced::Element<'_, Message> = if state.images.color_processing {
        iced::widget::button(iced::widget::text("Processing colors…").size(12)).into()
    } else {
        iced::widget::button(iced::widget::text("🎨 Process all").size(12))
            .on_press(Message::ProcessAllColors)
            .into()
    };

    children.push(
        iced::widget::row![sync_btn, download_btn, manage_btn, process_btn]
            .spacing(8)
            .into(),
    );
    // Bookmark sync progress: loaded/total bookmarks when known.
    if state.jobs.bookmark_sync.running {
        let frac = state
            .jobs
            .bookmark_sync
            .total
            .filter(|t| *t > 0)
            .map(|t| (state.jobs.bookmark_sync.done as f32 / t as f32).min(1.0))
            .unwrap_or(0.5);
        let bar = iced::widget::container(iced::widget::progress_bar(0.0..=1.0, frac))
            .width(iced::Length::Fill)
            .height(6);
        let text = match state.jobs.bookmark_sync.total {
            Some(total) if total > 0 => {
                format!(
                    "Syncing bookmarks… {}/{}",
                    state.jobs.bookmark_sync.done, total
                )
            }
            _ => format!(
                "Syncing bookmarks… {} loaded",
                state.jobs.bookmark_sync.done
            ),
        };
        children.push(
            iced::widget::column![iced::widget::text(text).size(11), bar]
                .spacing(2)
                .into(),
        );
    }

    // Shared download queue progress + aggregate speed.
    if state.jobs.download_job.running {
        let frac = state.jobs.download_queue.fraction();
        let bar = iced::widget::container(iced::widget::progress_bar(0.0..=1.0, frac))
            .width(iced::Length::Fill)
            .height(6);
        let speed: f64 = state.jobs.download_speeds.values().sum();
        let mut label = format!(
            "Downloading {} · {}/{} ({:.0}%){}",
            if state.jobs.download_job.current.is_empty() {
                "...".to_string()
            } else {
                state.jobs.download_job.current.clone()
            },
            state.jobs.download_queue.done(),
            state.jobs.download_queue.total,
            frac * 100.0,
            if speed > 0.0 {
                format!(" · {}", format_speed(speed))
            } else {
                String::new()
            },
        );
        // Show the backing-off state so the wait is visible, not a hang.
        if let Some(wait) = rate_limit_remaining(state) {
            label = format!(
                "{} · ⏳ rate limited — waiting {:.0}s",
                label,
                wait.as_secs_f64()
            );
        }
        children.push(
            iced::widget::column![iced::widget::text(label).size(11), bar]
                .spacing(2)
                .into(),
        );
    } else if state.jobs.download_queue.total > 0 {
        children.push(
            iced::widget::text(format!(
                "Downloaded {}/{}",
                state.jobs.download_queue.done(),
                state.jobs.download_queue.total
            ))
            .size(11)
            .into(),
        );
    }

    // "Process all" metadata pass: done/total progress bar.
    if state.images.color_processing {
        let total = state.images.color_total.max(1);
        let done = state.images.color_done.min(total);
        let frac = done as f32 / total as f32;
        let bar = iced::widget::container(iced::widget::progress_bar(0.0..=1.0, frac))
            .width(iced::Length::Fill)
            .height(6);
        children.push(
            iced::widget::column![
                iced::widget::text(format!("Processing colors… {done}/{total}")).size(11),
                bar
            ]
            .spacing(2)
            .into(),
        );
    }

    iced::widget::Column::with_children(children)
        .spacing(4)
        .padding([4, 8])
        .into()
}

/// Toolbar for the Twitter tab: timeline mode toggle, connection status and a
/// reload button.
fn twitter_toolbar(state: &AppState, theme: &iced::Theme) -> iced::Element<'static, Message> {
    use crate::twitter::TwitterFeedMode;

    let selected = state.twitter.feed_mode;
    let modes: &[(TwitterFeedMode, &str)] = &[
        (TwitterFeedMode::ForYou, "For You"),
        (TwitterFeedMode::Following, "Following"),
    ];
    let mut mode_buttons: Vec<iced::Element<'_, Message>> = Vec::new();
    for (mode, label) in modes {
        let active = *mode == selected;
        let mut btn = iced::widget::button(iced::widget::text(*label).size(11))
            .on_press(Message::TwitterFeedModeChanged(*mode))
            .padding(iced::Padding::from([2, 8]));
        btn = if active {
            btn.style(|theme, _| iced::widget::button::Style {
                background: Some(iced::Background::Color(crate::theme::panel(theme))),
                text_color: crate::theme::fg(theme),
                border: iced::Border {
                    color: crate::theme::accent(theme),
                    width: 1.5,
                    radius: 4.0.into(),
                },
                ..Default::default()
            })
        } else {
            btn.style(|theme, _| iced::widget::button::Style {
                background: Some(iced::Background::Color(
                    crate::theme::panel(theme).scale_alpha(0.45),
                )),
                text_color: crate::theme::muted(theme),
                ..Default::default()
            })
        };
        mode_buttons.push(btn.into());
    }

    let status = match &state.twitter.status {
        crate::app::state::TwitterStatus::Ready { handle } => format!("@{handle}"),
        crate::app::state::TwitterStatus::Loading => "Connecting…".to_string(),
        crate::app::state::TwitterStatus::Disabled => "Not connected".to_string(),
        crate::app::state::TwitterStatus::Error(_) => "Connection error".to_string(),
    };

    iced::widget::row![
        iced::widget::text("Timeline:").size(12),
        iced::widget::Row::with_children(mode_buttons).spacing(2),
        iced::widget::Space::new().width(iced::Length::Fill),
        iced::widget::text(status)
            .size(11)
            .color(crate::theme::muted(theme)),
        iced::widget::button(iced::widget::text("⟳").size(13))
            .on_press(Message::TwitterFeedReload)
            .padding(iced::Padding::from([2, 8])),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .padding([4, 8])
    .into()
}

/// Centered setup/error panel shown by the Twitter tab when it has no client
/// (cookies missing/expired) or is still connecting.
fn twitter_status_panel(state: &AppState, theme: &iced::Theme) -> iced::Element<'static, Message> {
    use crate::app::state::TwitterStatus;

    let (title, detail, show_retry) = match &state.twitter.status {
        TwitterStatus::Disabled => (
            "Twitter is not connected".to_string(),
            "Set the path to your browser-exported twitter_cookies.txt in Settings — likes and your feed stay in sync automatically."
                .to_string(),
            false,
        ),
        TwitterStatus::Loading => (
            "Connecting to Twitter…".to_string(),
            "Verifying your cookies.".to_string(),
            false,
        ),
        TwitterStatus::Error(e) => ("Twitter connection failed".to_string(), e.clone(), true),
        TwitterStatus::Ready { .. } => (
            "No photo tweets yet".to_string(),
            "Pull to refresh or check back later.".to_string(),
            true,
        ),
    };

    let mut buttons: Vec<iced::Element<'_, Message>> =
        vec![
            iced::widget::button(iced::widget::text("Open Settings").size(12))
                .on_press(Message::OpenSettings)
                .padding(iced::Padding::from([4, 12]))
                .into(),
        ];
    if show_retry {
        buttons.push(
            iced::widget::button(iced::widget::text("Retry").size(12))
                .on_press(Message::ReloadTwitterCookies)
                .padding(iced::Padding::from([4, 12]))
                .into(),
        );
    }

    centered_panel(iced::widget::column![
        iced::widget::text(title).size(15),
        iced::widget::text(detail)
            .size(12)
            .color(crate::theme::muted(theme)),
        iced::widget::Row::with_children(buttons).spacing(8),
    ])
}

/// Toolbar for the Bilibili tab: feed mode toggle (Following / fixed Users),
/// connection status and a reload button.
fn bilibili_toolbar(state: &AppState, theme: &iced::Theme) -> iced::Element<'static, Message> {
    use crate::bilibili::BilibiliFeedMode;

    let selected = state.bilibili.feed_mode;
    let modes: &[(BilibiliFeedMode, &str)] = &[
        (BilibiliFeedMode::Following, "Following"),
        (BilibiliFeedMode::Users, "Users"),
    ];
    let mut mode_buttons: Vec<iced::Element<'static, Message>> = Vec::new();
    for (mode, label) in modes {
        let active = *mode == selected;
        let mut btn = iced::widget::button(iced::widget::text(*label).size(11))
            .on_press(Message::BilibiliFeedModeChanged(*mode))
            .padding(iced::Padding::from([2, 8]));
        btn = if active {
            btn.style(|theme, _| iced::widget::button::Style {
                background: Some(iced::Background::Color(crate::theme::panel(theme))),
                text_color: crate::theme::fg(theme),
                border: iced::Border {
                    color: crate::theme::accent(theme),
                    width: 1.5,
                    radius: 4.0.into(),
                },
                ..Default::default()
            })
        } else {
            btn.style(|theme, _| iced::widget::button::Style {
                background: Some(iced::Background::Color(
                    crate::theme::panel(theme).scale_alpha(0.45),
                )),
                text_color: crate::theme::muted(theme),
                ..Default::default()
            })
        };
        mode_buttons.push(btn.into());
    }

    let status = match &state.bilibili.status {
        crate::app::state::BilibiliStatus::Ready { name } => name.clone(),
        crate::app::state::BilibiliStatus::Loading => "Connecting…".to_string(),
        crate::app::state::BilibiliStatus::Disabled => "Not connected".to_string(),
        crate::app::state::BilibiliStatus::Error(_) => "Connection error".to_string(),
    };

    iced::widget::row![
        iced::widget::text("Feed:").size(12),
        iced::widget::Row::with_children(mode_buttons).spacing(2),
        iced::widget::Space::new().width(iced::Length::Fill),
        iced::widget::text(status)
            .size(11)
            .color(crate::theme::muted(theme)),
        iced::widget::button(iced::widget::text("⟳").size(13))
            .on_press(Message::BilibiliFeedReload)
            .padding(iced::Padding::from([2, 8])),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .padding([4, 8])
    .into()
}

/// Centered setup/error panel shown by the Bilibili tab when it has no client
/// (cookies missing/expired), is connecting, or has no configured users.
fn bilibili_status_panel(state: &AppState, theme: &iced::Theme) -> iced::Element<'static, Message> {
    use crate::app::state::BilibiliStatus;
    use crate::bilibili::BilibiliFeedMode;

    let mut buttons: Vec<iced::Element<'static, Message>> =
        vec![
            iced::widget::button(iced::widget::text("Open Settings").size(12))
                .on_press(Message::OpenSettings)
                .padding(iced::Padding::from([4, 12]))
                .into(),
        ];
    let (title, detail, show_retry) = match &state.bilibili.status {
        BilibiliStatus::Disabled => (
            "Bilibili is not connected".to_string(),
            "Set the path to your browser-exported bilibili_cookies.txt in Settings — the dynamic feed and likes stay in sync automatically."
                .to_string(),
            false,
        ),
        BilibiliStatus::Loading => (
            "Connecting to Bilibili…".to_string(),
            "Verifying your cookies.".to_string(),
            false,
        ),
        BilibiliStatus::Error(e) => ("Bilibili connection failed".to_string(), e.clone(), true),
        BilibiliStatus::Ready { name } => {
            if state.bilibili.feed_mode == BilibiliFeedMode::Users
                && state.settings.bilibili_user_ids.is_empty()
            {
                (
                    "No users configured".to_string(),
                    "Add the user ids whose dynamics you want in Settings, or switch to the Following feed."
                        .to_string(),
                    true,
                )
            } else {
                (
                    format!("No image posts from {name} yet"),
                    "Pull to refresh or check back later.".to_string(),
                    true,
                )
            }
        }
    };
    if show_retry {
        buttons.push(
            iced::widget::button(iced::widget::text("Retry").size(12))
                .on_press(Message::ReloadBilibiliCookies)
                .padding(iced::Padding::from([4, 12]))
                .into(),
        );
    }
    centered_panel(iced::widget::column![
        iced::widget::text(title).size(15),
        iced::widget::text(detail)
            .size(12)
            .color(crate::theme::muted(theme)),
        iced::widget::Row::with_children(buttons).spacing(8),
    ])
}

/// Toolbar for the Leaderboard (ranking) tab: mode + date selectors. Choosing
/// a mode or a valid date refetches the leaderboard.
fn ranking_toolbar(tab: &GalleryTab) -> iced::Element<'_, Message> {
    use pixivj_rs::model::FilterMode;

    let modes: &[(FilterMode, &str)] = &[
        (FilterMode::Day, "Daily"),
        (FilterMode::Week, "Weekly"),
        (FilterMode::Month, "Monthly"),
        (FilterMode::DayMale, "Male"),
        (FilterMode::DayFemale, "Female"),
        (FilterMode::WeekOriginal, "Original"),
        (FilterMode::WeekRookie, "Rookie"),
        (FilterMode::DayManga, "Manga"),
        (FilterMode::DayR18, "R18"),
        (FilterMode::WeekR18, "R18 Weekly"),
    ];

    let selected = tab.ranking_mode.unwrap_or(FilterMode::Day);
    let mut mode_buttons: Vec<iced::Element<'_, Message>> = Vec::new();
    for (mode, label) in modes {
        let active = *mode == selected;
        // Active mode gets the normal button look; inactive ones are dimmed.
        let mut btn = iced::widget::button(iced::widget::text(*label).size(11))
            .on_press(Message::RankingModeChanged(*mode))
            .padding(iced::Padding::from([2, 8]));
        btn = if active {
            btn.style(|theme, _| iced::widget::button::Style {
                background: Some(iced::Background::Color(crate::theme::panel(theme))),
                text_color: crate::theme::fg(theme),
                border: iced::Border {
                    color: crate::theme::accent(theme),
                    width: 1.5,
                    radius: 4.0.into(),
                },
                ..Default::default()
            })
        } else {
            btn.style(|theme, _| iced::widget::button::Style {
                background: Some(iced::Background::Color(
                    crate::theme::panel(theme).scale_alpha(0.45),
                )),
                text_color: crate::theme::muted(theme),
                ..Default::default()
            })
        };
        mode_buttons.push(btn.into());
    }

    let date_input = iced::widget::text_input("YYYY-MM-DD (optional)", &tab.ranking_date)
        .on_input(Message::RankingDateChanged)
        .size(12)
        .width(iced::Length::Fixed(170.0));

    iced::widget::row![
        iced::widget::text("Mode:").size(12),
        iced::widget::Row::with_children(mode_buttons).spacing(2),
        iced::widget::Space::new().width(iced::Length::Fill),
        iced::widget::text("Date:").size(12),
        date_input,
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .padding([4, 8])
    .into()
}

/// Estimated gallery viewport height (window minus the chrome above it).
fn viewport_height(state: &AppState) -> f32 {
    (state.window_size.1 - 180.0).max(200.0)
}

/// How long until the rate-limit backoff window ends (API or image CDN),
/// if any limiter is currently backing off.
fn rate_limit_remaining(state: &AppState) -> Option<std::time::Duration> {
    if let Some(client) = state.client.as_ref() {
        if let Some(remaining) = client.rate_limiter().remaining() {
            return Some(remaining);
        }
    }
    state.images.fetcher.limiter.remaining()
}

/// Format a byte rate (bytes/sec) as a human-readable speed.
fn format_speed(bytes_per_sec: f64) -> String {
    const UNITS: &[&str] = &["B/s", "KB/s", "MB/s", "GB/s"];
    let mut value = bytes_per_sec;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", value, UNITS[unit])
}

/// Current process memory usage in MB (best effort).
fn memory_used_mb() -> f64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/self/statm") {
            let mut parts = content.split_whitespace();
            if let (Some(total), Some(resident)) = (parts.next(), parts.next()) {
                if let (Ok(_total), Ok(resident)) = (total.parse::<f64>(), resident.parse::<f64>())
                {
                    let page = 4096.0;
                    return resident * page / (1024.0 * 1024.0);
                }
                let _ = total;
            }
        }
    }
    0.0
}

/// Total system memory in MB (best effort).
fn memory_total_mb() -> f64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    if let Some(kb) = rest.trim().strip_suffix("kB") {
                        if let Ok(kb) = kb.trim().parse::<f64>() {
                            return kb / 1024.0;
                        }
                    }
                }
            }
        }
    }
    0.0
}

/// An icon-only button that cycles the dark/light preference:
/// System → Light → Dark. The glyph shows the current mode.
fn theme_button(pref: ThemePreference) -> iced::Element<'static, Message> {
    let label = match pref {
        ThemePreference::System => "◐",
        ThemePreference::Light => "☀",
        ThemePreference::Dark => "☾",
    };
    let next = match pref {
        ThemePreference::System => ThemePreference::Light,
        ThemePreference::Light => ThemePreference::Dark,
        ThemePreference::Dark => ThemePreference::System,
    };
    iced::widget::button(iced::widget::text(label).size(14))
        .on_press(Message::ThemePreferenceChanged(next))
        .padding(iced::Padding::from([2, 8]))
        .into()
}

/// An icon-only button that opens the settings screen.
fn settings_button() -> iced::Element<'static, Message> {
    iced::widget::button(iced::widget::text("⚙").size(14))
        .on_press(Message::OpenSettings)
        .padding(iced::Padding::from([2, 8]))
        .into()
}

/// An icon-only button that opens the help screen (usage, hotkeys, author,
/// license).
fn help_button() -> iced::Element<'static, Message> {
    iced::widget::button(iced::widget::text("?").size(14))
        .on_press(Message::ToggleHelp)
        .padding(iced::Padding::from([2, 10]))
        .into()
}
