//! Window/theme/settings/browser arms.

use crate::app::message::Message;
use crate::app::state::AppState;
use crate::io::config::save_config;
use std::sync::Arc;

pub(super) fn handle(state: &mut AppState, message: Message) -> iced::Task<Message> {
    match message {
        Message::ToggleDebugOverlay => {
            state.debug_overlay = !state.debug_overlay;
            iced::Task::none()
        }

        Message::WindowRegistered(id) => {
            state.window_id = Some(id);
            iced::Task::none()
        }

        // ── Config/queue flush (debounced, off the UI thread) ──────
        Message::FlushDisk => flush_disk(state),

        Message::WindowClosed => {
            log::info!("[update] Window closed");
            iced::Task::none()
        }

        Message::WindowCloseRequested(id) => {
            // OS close: in the detail/author/settings views it just exits the
            // overlay; otherwise it closes the window (which quits the app).
            if state.viewer.open {
                state.viewer.close();
                iced::Task::none()
            } else if state.artist.open {
                state.artist.close();
                iced::Task::none()
            } else if state.settings_open {
                state.settings_open = false;
                iced::Task::none()
            } else if state.show_help {
                state.show_help = false;
                iced::Task::none()
            } else if state.download_manager_open {
                state.download_manager_open = false;
                iced::Task::none()
            } else {
                iced::window::close(id)
            }
        }

        Message::ModifiersChanged(shift) => {
            state.shift_pressed = shift;
            iced::Task::none()
        }

        Message::ScaleFactorChanged(scale) => {
            if scale > 0.0 && (state.scale_factor - scale).abs() > 0.01 {
                state.scale_factor = scale;
                // The DPI changed → thumbnails need regenerating at the new
                // physical size to stay sharp.
                let mut tasks = super::reset_thumbnail_cache(state);
                let _ = &mut tasks;
                iced::Task::batch(tasks)
            } else {
                iced::Task::none()
            }
        }

        // ── Window resize ──────────────────────────────────────────
        Message::WindowResized { width, height } => {
            state.window_size = (width, height);
            // Gallery column count will be recalculated during view
            iced::Task::none()
        }

        // ── Theme ──────────────────────────────────────────────────
        Message::SystemThemeFetched(mode) | Message::SystemThemeChanged(mode) => {
            log::debug!("[update] System theme mode: {:?}", mode);
            state.theme_mode = mode;
            iced::Task::none()
        }

        Message::ThemePreferenceChanged(pref) => {
            state.settings.theme = pref;
            mark_config_dirty(state);
            // Resolve immediately so the override takes effect without
            // waiting for the OS to report a mode.
            state.theme_mode = match pref {
                crate::app::state::ThemePreference::Light => iced::theme::Mode::Light,
                crate::app::state::ThemePreference::Dark => iced::theme::Mode::Dark,
                crate::app::state::ThemePreference::System => state.theme_mode,
            };
            schedule_config_flush()
        }

        // ── Bookmark/follow/download ───────────────────────────────
        Message::OpenInBrowser(id) => {
            let url = browser_url(state, id);
            // Open in the default browser — fire and forget (the `open` crate
            // handles per-OS dispatch without hand-rolled `cmd /c start`).
            let _ = open::that(url);
            iced::Task::none()
        }

        Message::CopyIllustUrl(id) => iced::clipboard::write::<Message>(browser_url(state, id)),

        Message::RevealIllustFiles(id) => {
            let gallery = state
                .settings
                .gallery_path
                .clone()
                .unwrap_or_else(crate::core::default_gallery_dir);
            let title = state
                .viewer
                .illust
                .as_ref()
                .filter(|i| i.id == id)
                .and_then(|i| i.title.clone())
                .unwrap_or_default();
            let dir = crate::core::download_dir(&gallery, id, &title);
            if dir.is_dir() {
                let _ = open::that(dir);
            } else {
                log::warn!("[update] Reveal: {} not on disk", dir.display());
            }
            iced::Task::none()
        }

        // ── Settings ───────────────────────────────────────────────
        Message::GridSizeChanged(size) => {
            state.settings.grid_size = size;
            mark_config_dirty(state);
            // Grid size determines the thumbnail resolution — regenerate
            // cached thumbnails at the new size so tiles stay sharp.
            let mut tasks = vec![schedule_config_flush()];
            tasks.extend(super::reset_thumbnail_cache(state));
            iced::Task::batch(tasks)
        }

        Message::GridSizeStep(delta) => {
            state.settings.grid_size = if delta == 0 {
                200
            } else {
                (state.settings.grid_size as i32 + delta).clamp(80, 400) as u32
            };
            mark_config_dirty(state);
            let mut tasks = vec![schedule_config_flush()];
            tasks.extend(super::reset_thumbnail_cache(state));
            iced::Task::batch(tasks)
        }

        Message::OpenSettings => {
            state.settings_open = true;
            iced::Task::none()
        }

        Message::CloseSettings => {
            state.settings_open = false;
            iced::Task::none()
        }

        Message::ToggleHelp => {
            state.show_help = !state.show_help;
            iced::Task::none()
        }

        Message::BookmarkedDisplayChanged(display) => {
            state.settings.bookmarked_display = display;
            mark_config_dirty(state);
            // Re-seed every tab's Bookmarked tristate ("hide" mode) and
            // re-apply all of them.
            super::gallery::apply_bookmark_filter_defaults(state);
            let palettes = &state.images.palettes;
            for tab in state.tabs.iter_mut() {
                tab.reapply(palettes);
            }
            iced::Task::batch([schedule_config_flush()])
        }

        Message::BlurR18Changed(on) => {
            state.settings.blur_r18 = on;
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::TileRowsChanged(key, on) => {
            match key {
                crate::app::state::TileRowKey::Author => state.settings.tile_rows.author = on,
                crate::app::state::TileRowKey::Counts => state.settings.tile_rows.counts = on,
                crate::app::state::TileRowKey::Meta => state.settings.tile_rows.meta = on,
                crate::app::state::TileRowKey::Badges => state.settings.tile_rows.badges = on,
            }
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::FpsLimitChanged(fps) => {
            state.settings.fps_limit = fps.min(1000);
            crate::frame::set_limit(state.settings.fps_limit);
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::MemoryPresetChanged(preset) => {
            state.settings.memory_preset = preset;
            mark_config_dirty(state);
            // Rebuild the image caches at the new budgets. In-flight fetches
            // keep writing into the old Arcs; new fetches use the new ones.
            let budget =
                crate::image::cache::DEFAULT_MEMORY_BUDGET * preset.cache_budget_multiplier();
            state.images.image_cache = Arc::new(crate::image::cache::ImageCache::new(
                crate::io::config::cache_dir(),
                budget,
            ));
            state.images.thumbnail_cache = Arc::new(crate::image::cache::ImageCache::new(
                crate::io::config::cache_dir().join("thumb"),
                budget.max(crate::image::cache::DEFAULT_MEMORY_BUDGET),
            ));
            schedule_config_flush()
        }

        Message::AutoPreloadChanged(n) => {
            state.settings.auto_preload = n.clamp(0, 10);
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::DownloadConcurrencyChanged(n) => {
            state.settings.download_concurrency = n.clamp(1, 8);
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::GalleryPathInputChanged(path) => {
            let path = path.trim().to_string();
            state.settings.gallery_path = if path.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(&path))
            };
            mark_config_dirty(state);
            // The gallery directory changed — rescan what's downloaded.
            let flush = schedule_config_flush();
            iced::Task::batch([flush, iced::Task::done(Message::RefreshDownloaded)])
        }

        Message::ProxyInputChanged(proxy) => {
            let proxy = proxy.trim().to_string();
            state.settings.proxy = if proxy.is_empty() { None } else { Some(proxy) };
            mark_config_dirty(state);
            // Hot-apply: rebuild the clients after a quiet period (one task
            // per keystroke, but each no-ops unless the proxy still differs).
            let flush = schedule_config_flush();
            let apply = iced::Task::perform(
                async {
                    tokio::time::sleep(CONFIG_FLUSH_DEBOUNCE).await;
                },
                |_| Message::ApplyProxy,
            );
            iced::Task::batch([flush, apply])
        }

        Message::ApplyProxy => {
            if state.settings.proxy == state.applied_proxy {
                return iced::Task::none();
            }
            state.applied_proxy = state.settings.proxy.clone();
            // The image-CDN client is swapped synchronously (no auth needed);
            // in-flight fetches on the old pool simply finish there.
            state.images.fetcher.http = Arc::new(super::build_image_http_client(
                state.settings.proxy.as_deref(),
            ));
            log::info!(
                "[update] Proxy applied: {:?}",
                state.settings.proxy.as_deref().unwrap_or("(direct)")
            );
            match state.settings.refresh_token.clone() {
                Some(token) => iced::Task::perform(
                    super::onboarding::build_client_async(token, state.settings.proxy.clone()),
                    Message::ClientProxied,
                ),
                None => iced::Task::none(),
            }
        }

        Message::ClientProxied(result) => match result {
            Ok(client) => {
                state.client = Some(client);
                iced::Task::none()
            }
            Err(e) => {
                log::error!("[update] Client rebuild after proxy change failed: {}", e);
                iced::Task::done(Message::Error(format!("Proxy change failed: {e}")))
            }
        },

        Message::ParallaxChanged(on) => {
            state.settings.parallax = on;
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::ParallaxMouseChanged(on) => {
            state.settings.parallax_mouse = on;
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::ParallaxScrollChanged(on) => {
            state.settings.parallax_scroll = on;
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::ParallaxModeChanged(mode) => {
            state.settings.parallax_mode = mode;
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::GalleryFontSizeChanged(size) => {
            state.settings.gallery_font_size = size.clamp(
                crate::widgets::illust_info::TileText::MIN_FONT_SIZE,
                crate::widgets::illust_info::TileText::MAX_FONT_SIZE,
            );
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::GalleryTextStyleChanged(style) => {
            state.settings.gallery_text_style = style;
            mark_config_dirty(state);
            schedule_config_flush()
        }

        Message::ThumbnailHighResChanged(on) => {
            state.settings.high_res_thumbnails = on;
            mark_config_dirty(state);
            // The quality option changes the processed-thumbnail target —
            // regenerate them (the raw URL cache survives, so no re-download).
            let mut tasks = vec![schedule_config_flush()];
            tasks.extend(super::reset_thumbnail_cache(state));
            iced::Task::batch(tasks)
        }

        Message::ResetSetting(key) => {
            state.settings.reset(key);
            mark_config_dirty(state);
            // Live-apply side effects: theme override follows the reset.
            if key == crate::app::state::SettingsKey::Theme {
                state.theme_mode = match state.settings.theme {
                    crate::app::state::ThemePreference::Light => iced::theme::Mode::Light,
                    crate::app::state::ThemePreference::Dark => iced::theme::Mode::Dark,
                    crate::app::state::ThemePreference::System => state.theme_mode,
                };
            }
            if key == crate::app::state::SettingsKey::TwitterCookies {
                return iced::Task::batch([
                    schedule_config_flush(),
                    super::twitter::start_client(state),
                ]);
            }
            if key == crate::app::state::SettingsKey::BilibiliCookies {
                return iced::Task::batch([
                    schedule_config_flush(),
                    super::bilibili::start_client(state),
                ]);
            }
            if key == crate::app::state::SettingsKey::BilibiliUsers {
                state.bilibili.users_pending.clear();
                state.bilibili.user_ids_input = String::new();
                return schedule_config_flush();
            }
            if key == crate::app::state::SettingsKey::BookmarkedDisplay {
                super::gallery::apply_bookmark_filter_defaults(state);
                let palettes = &state.images.palettes;
                for tab in state.tabs.iter_mut() {
                    tab.reapply(palettes);
                }
                return schedule_config_flush();
            }
            if key == crate::app::state::SettingsKey::MemoryPreset {
                return iced::Task::done(Message::MemoryPresetChanged(
                    state.settings.memory_preset,
                ));
            }
            if key == crate::app::state::SettingsKey::FpsLimit {
                crate::frame::set_limit(state.settings.fps_limit);
                return schedule_config_flush();
            }
            schedule_config_flush()
        }

        Message::ResetAllSettings => {
            // Never touches the refresh token.
            state.settings.reset_all();
            mark_config_dirty(state);
            state.theme_mode = match state.settings.theme {
                crate::app::state::ThemePreference::Light => iced::theme::Mode::Light,
                crate::app::state::ThemePreference::Dark => iced::theme::Mode::Dark,
                crate::app::state::ThemePreference::System => state.theme_mode,
            };
            // Twitter/Bilibili cookies are re-resolved implicitly; disable
            // them when their files are gone.
            state.bilibili.users_pending.clear();
            state.bilibili.user_ids_input = String::new();
            // Re-seed the bookmarked filters, reset the frame limit and
            // rebuild the caches at the (reset) preset's budgets.
            super::gallery::apply_bookmark_filter_defaults(state);
            crate::frame::set_limit(state.settings.fps_limit);
            let budget = crate::image::cache::DEFAULT_MEMORY_BUDGET
                * state.settings.memory_preset.cache_budget_multiplier();
            state.images.image_cache = Arc::new(crate::image::cache::ImageCache::new(
                crate::io::config::cache_dir(),
                budget,
            ));
            state.images.thumbnail_cache = Arc::new(crate::image::cache::ImageCache::new(
                crate::io::config::cache_dir().join("thumb"),
                budget.max(crate::image::cache::DEFAULT_MEMORY_BUDGET),
            ));
            let twitter = super::twitter::start_client(state);
            let bilibili = super::bilibili::start_client(state);
            iced::Task::batch([schedule_config_flush(), twitter, bilibili])
        }

        // ── Error ──────────────────────────────────────────────────
        Message::Error(msg) => {
            log::error!("[update] {}", msg);
            // Surface user-facing failures in the UI (a banner), not just logs.
            state.last_error = Some(msg);
            iced::Task::none()
        }

        Message::DismissError => {
            state.last_error = None;
            iced::Task::none()
        }

        // ── Onboarding: start ──────────────────────────────────────

        // No-op: emitted by background tasks whose result is irrelevant
        // (e.g. the boot emoji-font load). Must be handled so it never
        // reaches the unreachable arm below.
        Message::Noop => iced::Task::none(),

        // Any message routed here by the dispatcher must be one of the above.
        _ => unreachable!("message routed to the wrong update handler"),
    }
}

// ── Config write debounce ──────────────────────────────────────────────

/// Coalesce rapid setting changes (per-keystroke inputs) into one write.
const CONFIG_FLUSH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(800);

/// Mark settings as changed; the next [`Message::FlushDisk`] writes them.
pub(super) fn mark_config_dirty(state: &mut AppState) {
    state.config_dirty = true;
}

/// Debounce: after a quiet period, emit [`Message::FlushDisk`].
pub(super) fn schedule_config_flush() -> iced::Task<Message> {
    iced::Task::perform(
        async move {
            tokio::time::sleep(CONFIG_FLUSH_DEBOUNCE).await;
        },
        |_| Message::FlushDisk,
    )
}

/// Debounce a dirty queue write (jobs module marks `queue_dirty`).
pub(super) fn schedule_queue_flush() -> iced::Task<Message> {
    iced::Task::perform(
        async move {
            tokio::time::sleep(CONFIG_FLUSH_DEBOUNCE).await;
        },
        |_| Message::FlushDisk,
    )
}

/// The public web URL for a media item: the Pixiv artwork page, the tweet or
/// the Bilibili dynamic. The provider owns the URL shape.
fn browser_url(state: &AppState, id: u64) -> String {
    crate::media::provider_of(id).web_url_of(state.viewer.illust.as_ref(), id)
}

/// Persist the settings + download queue off the UI thread if either changed
/// since the last write. Multiple pending flushes collapse into one write.
///
/// Only the dirty parts are cloned/serialized: the old version snapshotted
/// and rewrote all four files (library included) whenever *anything* was
/// dirty, so a color-palette flush re-serialized the entire library and
/// queue — the main reason "process all" stalled the UI thread.
pub(super) fn flush_disk(state: &mut AppState) -> iced::Task<Message> {
    let config_dirty = std::mem::take(&mut state.config_dirty);
    let queue_dirty = std::mem::take(&mut state.jobs.queue_dirty);
    let colors_dirty = std::mem::take(&mut state.images.colors_dirty);
    let library_dirty = std::mem::take(&mut state.jobs.library_dirty);
    if !(config_dirty || queue_dirty || colors_dirty || library_dirty) {
        return iced::Task::none();
    }

    let settings = config_dirty.then(|| state.settings.clone());
    let queue = queue_dirty.then(|| state.jobs.download_queue.clone());
    let colors = colors_dirty.then(|| state.images.palettes.clone());
    let library = library_dirty.then(|| state.jobs.library.clone());
    let queue_path = state.jobs.download_queue_path.clone();
    let library_path = state.jobs.library_path.clone();
    iced::Task::perform(
        async move {
            let _ = tokio::task::spawn_blocking(move || {
                if let Some(settings) = &settings {
                    save_config(settings);
                }
                if let Some(queue) = &queue {
                    let _ = queue.save_atomic(&queue_path);
                }
                if let Some(library) = &library {
                    let _ = library.save_atomic(&library_path);
                }
                if let Some(colors) = &colors {
                    if !colors.is_empty() {
                        crate::io::colors::save_colors(colors);
                    }
                }
            })
            .await;
        },
        |_| Message::Noop,
    )
}
