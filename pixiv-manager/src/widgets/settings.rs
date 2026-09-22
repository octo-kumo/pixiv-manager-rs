//! The settings screen — a modal overlay listing every configurable option
//! with a per-item reset button and a global reset (which never touches the
//! saved refresh token).

use crate::app::message::Message;
use crate::app::state::{AppState, SettingsKey, ThemePreference};

/// The settings screen: rows of label / control / per-item reset, plus the
/// global reset at the bottom.
#[allow(clippy::vec_init_then_push)] // the rows read top-to-bottom as sections
pub fn settings_screen(state: &AppState) -> iced::Element<'_, Message> {
    let mut rows: Vec<iced::Element<'_, Message>> = Vec::new();

    // ── Theme ──────────────────────────────────────────────────────
    rows.push(setting_row(
        "Theme",
        theme_selector(state.settings.theme),
        Some(SettingsKey::Theme),
        state,
    ));

    // ── Grid size ──────────────────────────────────────────────────
    rows.push(setting_row(
        "Grid size (px)",
        stepper(
            state.settings.grid_size,
            80,
            400,
            10,
            Message::GridSizeChanged,
        ),
        Some(SettingsKey::GridSize),
        state,
    ));

    // ── High-resolution thumbnails ─────────────────────────────────
    rows.push(setting_row(
        "High-res thumbnails",
        checkbox_row(
            "Use 1200px sources for sharper tiles",
            state.settings.high_res_thumbnails,
            Message::ThumbnailHighResChanged,
        ),
        Some(SettingsKey::ThumbnailHighRes),
        state,
    ));

    // ── Tile text (font size + style) ──────────────────────────────
    rows.push(setting_row(
        "Tile text size (px)",
        stepper(
            state.settings.gallery_font_size,
            crate::widgets::illust_info::TileText::MIN_FONT_SIZE,
            crate::widgets::illust_info::TileText::MAX_FONT_SIZE,
            1,
            Message::GalleryFontSizeChanged,
        ),
        Some(SettingsKey::GalleryFontSize),
        state,
    ));
    rows.push(setting_row(
        "Tile text style",
        text_style_selector(state.settings.gallery_text_style),
        Some(SettingsKey::GalleryTextStyle),
        state,
    ));

    // ── Tile info rows (which lines show on a gallery tile) ────────
    rows.push(setting_row(
        "Tile info rows",
        iced::widget::column![
            checkbox_row("Author", state.settings.tile_rows.author, |on| {
                Message::TileRowsChanged(crate::app::state::TileRowKey::Author, on)
            },),
            checkbox_row("Bookmarks / views", state.settings.tile_rows.counts, |on| {
                Message::TileRowsChanged(crate::app::state::TileRowKey::Counts, on)
            },),
            checkbox_row("Id / size / date", state.settings.tile_rows.meta, |on| {
                Message::TileRowsChanged(crate::app::state::TileRowKey::Meta, on)
            },),
            checkbox_row(
                "Badges (R-18 / AI / pages)",
                state.settings.tile_rows.badges,
                |on| Message::TileRowsChanged(crate::app::state::TileRowKey::Badges, on),
            ),
        ]
        .spacing(4)
        .into(),
        Some(SettingsKey::TileRows),
        state,
    ));

    // ── Bookmarked items outside the Bookmarks tab ─────────────────
    rows.push(setting_row(
        "Bookmarked items (other tabs)",
        iced::widget::row![
            mode_btn(
                "Visible",
                state.settings.bookmarked_display == crate::app::state::BookmarkedDisplay::Off,
                || Message::BookmarkedDisplayChanged(crate::app::state::BookmarkedDisplay::Off),
            ),
            mode_btn(
                "Dimmed",
                state.settings.bookmarked_display == crate::app::state::BookmarkedDisplay::Dim,
                || Message::BookmarkedDisplayChanged(crate::app::state::BookmarkedDisplay::Dim),
            ),
            mode_btn(
                "Hidden",
                state.settings.bookmarked_display == crate::app::state::BookmarkedDisplay::Hide,
                || Message::BookmarkedDisplayChanged(crate::app::state::BookmarkedDisplay::Hide),
            ),
        ]
        .spacing(4)
        .into(),
        Some(SettingsKey::BookmarkedDisplay),
        state,
    ));

    // ── R-18 blur ──────────────────────────────────────────────────
    rows.push(setting_row(
        "Blur R-18 tiles",
        checkbox_row(
            "Cover R-18 thumbnails until hovered",
            state.settings.blur_r18,
            Message::BlurR18Changed,
        ),
        Some(SettingsKey::BlurR18),
        state,
    ));

    // ── Memory preset ──────────────────────────────────────────────
    let preset = state.settings.memory_preset;
    rows.push(setting_row(
        "Memory usage",
        iced::widget::row![
            mode_btn(
                "Low",
                preset == crate::app::state::MemoryPreset::Low,
                || Message::MemoryPresetChanged(crate::app::state::MemoryPreset::Low),
            ),
            mode_btn(
                "High",
                preset == crate::app::state::MemoryPreset::High,
                || Message::MemoryPresetChanged(crate::app::state::MemoryPreset::High),
            ),
            iced::widget::text(
                "Low keeps memory small; High keeps more images cached for responsiveness."
            )
            .size(10),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center)
        .into(),
        Some(SettingsKey::MemoryPreset),
        state,
    ));

    // ── Viewer preload-ahead ───────────────────────────────────────
    rows.push(setting_row(
        "Viewer preload pages",
        stepper(
            state.settings.auto_preload,
            0,
            10,
            1,
            Message::AutoPreloadChanged,
        ),
        Some(SettingsKey::AutoPreload),
        state,
    ));

    // ── Download concurrency ───────────────────────────────────────
    rows.push(setting_row(
        "Download concurrency",
        stepper(state.settings.download_concurrency as u32, 1, 8, 1, |v| {
            Message::DownloadConcurrencyChanged(v as usize)
        }),
        Some(SettingsKey::DownloadConcurrency),
        state,
    ));

    // ── Gallery path ───────────────────────────────────────────────
    let path_text = state
        .settings
        .gallery_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    rows.push(setting_row(
        "Gallery directory",
        iced::widget::row![
            iced::widget::text_input("e.g. ~/Pictures/Pixiv", &path_text)
                .on_input(Message::GalleryPathInputChanged)
                .size(13)
                .width(iced::Length::FillPortion(4)),
            iced::widget::button(iced::widget::text("Browse...").size(12))
                .on_press(Message::PickDirectory)
                .padding(iced::Padding::from([4, 10])),
        ]
        .spacing(6)
        .into(),
        Some(SettingsKey::GalleryPath),
        state,
    ));

    // ── Proxy ──────────────────────────────────────────────────────
    let proxy_text = state.settings.proxy.clone().unwrap_or_default();
    rows.push(setting_row(
        "Proxy (e.g. http://127.0.0.1:7890)",
        iced::widget::text_input("None", &proxy_text)
            .on_input(Message::ProxyInputChanged)
            .size(13)
            .width(iced::Length::FillPortion(4))
            .into(),
        Some(SettingsKey::Proxy),
        state,
    ));

    // ── Twitter cookies ────────────────────────────────────────────
    let cookie_text = state
        .settings
        .twitter_cookie_path
        .clone()
        .unwrap_or_default();
    let twitter_status = match &state.twitter.status {
        crate::app::state::TwitterStatus::Ready { handle } => format!("Connected as @{handle}"),
        crate::app::state::TwitterStatus::Loading => "Connecting…".to_string(),
        crate::app::state::TwitterStatus::Disabled => {
            "Not configured — Twitter features are disabled".to_string()
        }
        crate::app::state::TwitterStatus::Error(e) => format!("Error: {e}"),
    };
    rows.push(setting_row(
        "Twitter cookies file",
        iced::widget::column![
            iced::widget::row![
                iced::widget::text_input(
                    "~/.config/pixiv-manager/twitter_cookies.txt",
                    &cookie_text,
                )
                .on_input(Message::TwitterCookiePathChanged)
                .size(13)
                .width(iced::Length::FillPortion(4)),
                iced::widget::button(iced::widget::text("Reload").size(12))
                    .on_press(Message::ReloadTwitterCookies)
                    .padding(iced::Padding::from([4, 10])),
            ]
            .spacing(6),
            iced::widget::text(twitter_status).size(11),
        ]
        .spacing(2)
        .into(),
        Some(SettingsKey::TwitterCookies),
        state,
    ));

    // Query ids rotate every few months; this refetches them from X's web
    // bundles and writes the runtime override, so no rebuild is needed.
    let updating_ids = state.twitter.query_ids_loading;
    let can_refresh_ids = state.twitter.client.is_some() && !updating_ids;
    rows.push(setting_row(
        "Twitter GraphQL query ids",
        iced::widget::row![
            iced::widget::text(if updating_ids {
                "Updating from x.com…"
            } else {
                "Fetch the current ids when Twitter starts returning 404s."
            })
            .size(11),
            iced::widget::button(
                iced::widget::text(if updating_ids {
                    "Updating…"
                } else {
                    "Update query ids"
                })
                .size(12)
            )
            .on_press_maybe(can_refresh_ids.then_some(Message::RefreshTwitterQueryIds))
            .padding(iced::Padding::from([4, 10])),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center)
        .into(),
        None,
        state,
    ));

    // ── Bilibili cookies + user list ───────────────────────────────
    let bili_cookie_text = state
        .settings
        .bilibili_cookie_path
        .clone()
        .unwrap_or_default();
    let bili_status = match &state.bilibili.status {
        crate::app::state::BilibiliStatus::Ready { name } => format!("Connected as {name}"),
        crate::app::state::BilibiliStatus::Loading => "Connecting…".to_string(),
        crate::app::state::BilibiliStatus::Disabled => {
            "Not configured — Bilibili features are disabled".to_string()
        }
        crate::app::state::BilibiliStatus::Error(e) => format!("Error: {e}"),
    };
    rows.push(setting_row(
        "Bilibili cookies file",
        iced::widget::column![
            iced::widget::row![
                iced::widget::text_input(
                    "~/.config/pixiv-manager/bilibili_cookies.txt",
                    &bili_cookie_text,
                )
                .on_input(Message::BilibiliCookiePathChanged)
                .size(13)
                .width(iced::Length::FillPortion(4)),
                iced::widget::button(iced::widget::text("Reload").size(12))
                    .on_press(Message::ReloadBilibiliCookies)
                    .padding(iced::Padding::from([4, 10])),
            ]
            .spacing(6),
            iced::widget::text(bili_status).size(11),
        ]
        .spacing(2)
        .into(),
        Some(SettingsKey::BilibiliCookies),
        state,
    ));

    let bili_users = if state.bilibili.user_ids_input.is_empty() {
        state
            .settings
            .bilibili_user_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        state.bilibili.user_ids_input.clone()
    };
    rows.push(setting_row(
        "Bilibili users (ids, comma separated)",
        iced::widget::text_input("e.g. 2105955, 12345678", &bili_users)
            .on_input(Message::BilibiliUserIdsChanged)
            .size(13)
            .width(iced::Length::FillPortion(4))
            .into(),
        Some(SettingsKey::BilibiliUsers),
        state,
    ));

    // ── FPS limit ──────────────────────────────────────────────────
    let limit = state.settings.fps_limit;
    let limit_btn = |label: &'static str, value: u32| {
        mode_btn(label, limit == value, move || {
            Message::FpsLimitChanged(value)
        })
    };
    rows.push(setting_row(
        "Animation FPS limit",
        iced::widget::row![
            limit_btn("Off", 0),
            limit_btn("30", 30),
            limit_btn("60", 60),
            limit_btn("120", 120),
            limit_btn("240", 240),
            iced::widget::text("Caps the animation loops; input and loads stay instant.").size(10),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center)
        .into(),
        Some(SettingsKey::FpsLimit),
        state,
    ));

    // ── Parallax ───────────────────────────────────────────────────
    let mode = state.settings.parallax_mode;
    let mode_row = iced::widget::row![
        iced::widget::text("Scroll parallax:").size(12),
        mode_btn(
            "Centered",
            mode == crate::widgets::parallax_image::ParallaxMode::Centered,
            {
                let m = crate::widgets::parallax_image::ParallaxMode::Centered;
                move || Message::ParallaxModeChanged(m)
            }
        ),
        mode_btn(
            "Linear",
            mode == crate::widgets::parallax_image::ParallaxMode::Linear,
            {
                let m = crate::widgets::parallax_image::ParallaxMode::Linear;
                move || Message::ParallaxModeChanged(m)
            }
        ),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);

    rows.push(setting_row(
        "Parallax effect",
        iced::widget::column![
            checkbox_row("Enabled", state.settings.parallax, Message::ParallaxChanged),
            checkbox_row(
                "Follow mouse on hover",
                state.settings.parallax_mouse,
                Message::ParallaxMouseChanged,
            ),
            checkbox_row(
                "Follow scroll position",
                state.settings.parallax_scroll,
                Message::ParallaxScrollChanged,
            ),
            mode_row,
        ]
        .spacing(4)
        .into(),
        Some(SettingsKey::Parallax),
        state,
    ));

    // ── Session / token ────────────────────────────────────────────
    // Read-only: per-item and global resets deliberately never touch the
    // refresh token (the saved token is what keeps you logged in).
    let token_display = state
        .settings
        .refresh_token
        .as_deref()
        .map(mask_token)
        .unwrap_or_else(|| "not configured — run onboarding".to_string());
    rows.push(setting_row(
        "Pixiv session",
        iced::widget::text(token_display).size(12).into(),
        // No reset button for the token.
        None,
        state,
    ));

    // ── Global reset ───────────────────────────────────────────────
    let reset_all = iced::widget::button(iced::widget::text("Reset all settings").size(13))
        .on_press(Message::ResetAllSettings)
        .padding(iced::Padding::from([8, 16]));

    let panel = iced::widget::container(
        iced::widget::column![
            header(),
            // Scrollable: the settings list is long (cookies, parallax,
            // memory, tile rows…); it must not overflow small windows.
            iced::widget::scrollable(iced::widget::Column::with_children(rows).spacing(10))
                .width(iced::Length::Fill)
                .height(iced::Length::Fill),
            reset_all
        ]
        .spacing(16)
        .padding(24)
        .width(720),
    )
    .style(|theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(crate::theme::panel(theme))),
        border: iced::Border {
            radius: 8.0.into(),
            color: crate::theme::border(theme),
            width: 1.0,
        },
        ..Default::default()
    });

    iced::widget::container(panel)
        .center_x(iced::Length::Fill)
        .center_y(iced::Length::Fill)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .padding(16)
        .into()
}

/// The screen header: title + close button.
fn header() -> iced::Element<'static, Message> {
    iced::widget::row![
        iced::widget::text("Settings").size(20),
        iced::widget::Space::new().width(iced::Length::Fill),
        iced::widget::button(iced::widget::text("✕").size(14))
            .on_press(Message::CloseSettings)
            .padding(iced::Padding::from([4, 10])),
    ]
    .into()
}

/// A label + control + per-item reset button row.
fn setting_row<'a>(
    label: &'a str,
    control: iced::Element<'a, Message>,
    key: Option<SettingsKey>,
    state: &AppState,
) -> iced::Element<'a, Message> {
    let mut right: Vec<iced::Element<'a, Message>> = vec![control];
    if let Some(key) = key {
        // Per-item reset: enabled only when the setting differs from default.
        let reset: iced::Element<'a, Message> =
            iced::widget::button(iced::widget::text("Reset").size(11))
                .on_press_maybe(
                    (!state.settings.is_default(key)).then_some(Message::ResetSetting(key)),
                )
                .padding(iced::Padding::from([3, 8]))
                .into();
        right.push(reset);
    }
    iced::widget::row![
        iced::widget::text(label).size(13).width(230),
        iced::widget::Row::with_children(right).spacing(8),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A labeled checkbox.
fn checkbox_row<'a>(
    label: &'a str,
    checked: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> iced::Element<'a, Message> {
    iced::widget::checkbox(checked)
        .label(label)
        .on_toggle(on_toggle)
        .into()
}

/// A small two-state mode button (highlighted when selected).
fn mode_btn(
    label: &'static str,
    selected: bool,
    on_click: impl Fn() -> Message + 'static,
) -> iced::Element<'static, Message> {
    let mut btn = iced::widget::button(iced::widget::text(label).size(12))
        .on_press(on_click())
        .padding(iced::Padding::from([3, 10]));
    if selected {
        btn = btn.style(|theme, _| iced::widget::button::Style {
            background: Some(iced::Background::Color(crate::theme::panel(theme))),
            text_color: crate::theme::fg(theme),
            border: iced::Border {
                color: crate::theme::accent(theme),
                width: 1.5,
                radius: 4.0.into(),
            },
            ..Default::default()
        });
    }
    btn.into()
}

/// Three-button tile-text style selector: Default / Invert / Background.
fn text_style_selector(
    style: crate::widgets::illust_info::GalleryTextStyle,
) -> iced::Element<'static, Message> {
    use crate::widgets::illust_info::GalleryTextStyle as Style;
    iced::widget::row![
        mode_btn("Default", style == Style::Default, || {
            Message::GalleryTextStyleChanged(Style::Default)
        }),
        mode_btn("Invert", style == Style::Invert, || {
            Message::GalleryTextStyleChanged(Style::Invert)
        }),
        mode_btn("Background", style == Style::Background, || {
            Message::GalleryTextStyleChanged(Style::Background)
        }),
    ]
    .spacing(4)
    .into()
}

/// Three-button theme selector: Auto / Light / Dark.
fn theme_selector(pref: ThemePreference) -> iced::Element<'static, Message> {
    let mut buttons: Vec<iced::Element<'static, Message>> = Vec::new();
    for (label, value) in [
        ("Auto", ThemePreference::System),
        ("Light", ThemePreference::Light),
        ("Dark", ThemePreference::Dark),
    ] {
        let selected = pref == value;
        let mut button: iced::widget::Button<'static, Message, iced::Theme> =
            iced::widget::button(iced::widget::text(label).size(12))
                .on_press(Message::ThemePreferenceChanged(value))
                .padding(iced::Padding::from([4, 12]));
        if selected {
            button = button.style(|theme, _| iced::widget::button::Style {
                background: Some(iced::Background::Color(crate::theme::accent(theme))),
                text_color: crate::theme::on_accent(theme),
                border: iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        }
        buttons.push(button.into());
    }
    iced::widget::row(buttons).spacing(4).into()
}

/// A − / value / + stepper for an integer setting.
fn stepper<'a>(
    value: u32,
    min: u32,
    max: u32,
    step: u32,
    on_change: impl Fn(u32) -> Message + 'a,
) -> iced::Element<'a, Message> {
    iced::widget::row![
        iced::widget::button(iced::widget::text("−").size(14))
            .on_press_maybe(
                (value.saturating_sub(step) >= min)
                    .then_some(on_change(value.saturating_sub(step)))
            )
            .padding(iced::Padding::from([2, 8])),
        iced::widget::text(format!("{}", value)).size(13).width(30),
        iced::widget::button(iced::widget::text("+").size(14))
            .on_press_maybe((value + step <= max).then_some(on_change(value + step)))
            .padding(iced::Padding::from([2, 8])),
    ]
    .spacing(6)
    .into()
}

/// Mask a refresh token for display: keep the last 4 chars, hide the rest.
fn mask_token(token: &str) -> String {
    if token.len() <= 4 {
        return "••••".to_string();
    }
    format!("••••••••{}", &token[token.len() - 4..])
}
