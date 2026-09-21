use crate::app::message::Message;
use pixivj_rs::model::Illustration;
/// How the gallery tile's info text is drawn over the thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum GalleryTextStyle {
    /// Plain text (the original look).
    #[default]
    #[serde(rename = "default")]
    Default,
    /// Inverted (XOR-style) text: each glyph takes the complement of the
    /// thumbnail's average color, so the text flips against whatever is
    /// behind it. (iced exposes no per-pixel blend modes, so this uses the
    /// tile's average color as the XOR target.)
    #[serde(rename = "invert")]
    Invert,
    /// Text over a half-transparent dark strip.
    #[serde(rename = "background")]
    Background,
}

/// The user-configurable text rendering for one gallery tile.
#[derive(Debug, Clone, Copy)]
pub struct TileText {
    /// Base font size in px (the author line; other lines derive from it).
    pub font_size: u32,
    /// How the text is drawn over the thumbnail.
    pub style: GalleryTextStyle,
    /// Which info rows are shown (author/counts/meta/badges — all default
    /// on, toggled in Settings).
    pub rows: crate::app::state::TileRows,
    /// Average RGB of the tile's thumbnail — required by
    /// [`GalleryTextStyle::Invert`] (tiles without one fall back to the
    /// plain theme color).
    pub avg: Option<[u8; 3]>,
}

impl Default for TileText {
    fn default() -> Self {
        Self {
            font_size: Self::DEFAULT_FONT_SIZE,
            style: GalleryTextStyle::Default,
            rows: crate::app::state::TileRows::default(),
            avg: None,
        }
    }
}

impl TileText {
    /// Font size bounds offered by the settings screen.
    pub const MIN_FONT_SIZE: u32 = 7;
    pub const MAX_FONT_SIZE: u32 = 24;
    /// The default base size (matches the pre-setting look).
    pub const DEFAULT_FONT_SIZE: u32 = 11;

    fn base(&self) -> u32 {
        self.font_size
            .clamp(Self::MIN_FONT_SIZE, Self::MAX_FONT_SIZE)
    }

    /// The author line's size (the configured base).
    pub fn author_size(&self) -> u32 {
        self.base()
    }

    /// Counts/meta line size (currently base − 2).
    pub fn secondary_size(&self) -> u32 {
        self.base().saturating_sub(2).max(7)
    }

    /// Badge label size (currently base − 3).
    pub fn badge_size(&self) -> u32 {
        self.base().saturating_sub(3).max(6)
    }

    /// The explicit text color for the info lines, or `None` to inherit the
    /// theme's default text color. The "background" style puts the text on a
    /// dark translucent strip, so it always uses white to stand out.
    pub fn color(&self) -> Option<iced::Color> {
        match self.style {
            GalleryTextStyle::Default => None,
            GalleryTextStyle::Background => Some(iced::Color::WHITE),
            GalleryTextStyle::Invert => self.avg.map(inverted_text_color),
        }
    }
}

/// The "inverted" text color for a thumbnail's average color: the complement,
/// snapped to black/white when the complement is too close in luminance to be
/// readable (a mid-gray tile would otherwise render invisible glyphs).
fn inverted_text_color(avg: [u8; 3]) -> iced::Color {
    let complement = [255 - avg[0], 255 - avg[1], 255 - avg[2]];
    let lum = |c: [u8; 3]| 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
    if (lum(complement) - lum(avg)).abs() < 60.0 {
        // XOR of a mid-tone stays mid-tone; use the readable extreme instead.
        if lum(avg) < 128.0 {
            iced::Color::WHITE
        } else {
            iced::Color::BLACK
        }
    } else {
        iced::Color::from_rgb8(complement[0], complement[1], complement[2])
    }
}

/// Compact per-tile info overlay (Java's `IllustrationInfo`): clickable
/// author, ★bookmarks / 👁views, id, relative date, dimensions, and the
/// R-18 / AI / sanity / page-count badges. Which rows appear is
/// user-configurable via [`TileText::rows`].
pub fn illust_info(illust: &Illustration, text: TileText) -> iced::Element<'_, Message> {
    let color = text.color();
    let author: iced::Element<'_, Message> = match &illust.user {
        Some(user) => {
            let name = ellipsize(&user.name, 14);
            let label = iced::widget::text(name).size(text.author_size());
            let label = if let Some(c) = color {
                label.color(c)
            } else {
                label
            };
            // Every provider has an author page now (Pixiv profile, the
            // user's tweets, their image dynamics) — the id is namespaced
            // and the reducer dispatches on it.
            iced::widget::tooltip(
                iced::widget::button(label)
                    .on_press(Message::ArtistOpened(user.id))
                    .padding(iced::Padding::from([0, 2]))
                    .style(iced::widget::button::text),
                iced::widget::text(user.name.clone()).size(text.author_size()),
                iced::widget::tooltip::Position::Top,
            )
            .into()
        }
        None => {
            let label = iced::widget::text("unknown").size(text.author_size());
            if let Some(c) = color {
                label.color(c).into()
            } else {
                label.into()
            }
        }
    };

    let views = illust.total_view.unwrap_or(0);
    let bookmarks = illust.total_bookmarks.unwrap_or(0);
    let bookmark_glyph = if crate::media::is_pixiv(illust.id) {
        "★"
    } else {
        "♥"
    };

    let mut badge_elems: Vec<iced::Element<'_, Message>> = Vec::new();
    if illust.is_r18() {
        badge_elems.push(badge("R-18".into(), [0.9, 0.2, 0.2], text.badge_size()));
    }
    if illust.is_ai() {
        badge_elems.push(badge("AI".into(), [0.5, 0.3, 0.8], text.badge_size()));
    }
    if let Some(sanity) = illust.sanity_level {
        if sanity <= 4 {
            badge_elems.push(badge(
                format!("S{sanity}"),
                [0.85, 0.55, 0.1],
                text.badge_size(),
            ));
        }
    }
    if illust.page_count.unwrap_or(1) > 1 {
        badge_elems.push(badge(
            format!("{}p", illust.page_count.unwrap()),
            [0.3, 0.6, 0.3],
            text.badge_size(),
        ));
    }

    let badge_row = iced::widget::Row::with_children(badge_elems).spacing(2);

    // Second line: id · W×H (when known).
    let mut meta_parts: Vec<iced::Element<'_, Message>> = vec![colored_text(
        format!("#{}", crate::media::native_id(illust.id)),
        text.secondary_size(),
        color,
    )];
    if let (Some(w), Some(h)) = (illust.width, illust.height) {
        meta_parts.push(colored_text(
            format!("{w}×{h}"),
            text.secondary_size(),
            color,
        ));
    }
    if let Some(date) = illust.create_date {
        meta_parts.push(colored_text(
            relative_date(date),
            text.secondary_size(),
            color,
        ));
    }
    let meta_row = iced::widget::Row::with_children(meta_parts).spacing(4);

    let counts = iced::widget::row![
        colored_text(
            format!("{bookmark_glyph}{bookmarks}"),
            text.secondary_size(),
            color
        ),
        colored_text(format!("👁{views}"), text.secondary_size(), color),
    ]
    .spacing(4);

    let mut info = iced::widget::Column::<Message>::new().spacing(2);
    info = info.push(author);
    info = info.push(counts);
    info = info.push(meta_row);
    info = info.push(badge_row);

    // "Background" style: put the whole info block on a translucent strip.
    if text.style == GalleryTextStyle::Background {
        iced::widget::container(info)
            .width(iced::Length::Fill)
            .padding(iced::Padding::from([3, 5]))
            .style(|_theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.55,
                ))),
                ..Default::default()
            })
            .into()
    } else {
        info.into()
    }
}

/// A sized text widget with an optional explicit color.
fn colored_text<'a>(
    label: impl iced::widget::text::IntoFragment<'a>,
    size: u32,
    color: Option<iced::Color>,
) -> iced::Element<'a, Message> {
    let text = iced::widget::text(label).size(size);
    if let Some(color) = color {
        text.color(color).into()
    } else {
        text.into()
    }
}

/// A human "time ago" string (Java used PrettyTime).
pub(crate) fn relative_date(date: chrono::DateTime<chrono::Utc>) -> String {
    use chrono::Utc;
    let now = Utc::now();
    let secs = (now - date).num_seconds().max(0);
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 86400 * 7 {
        format!("{}d ago", secs / 86400)
    } else if secs < 86400 * 30 {
        format!("{}w ago", secs / (86400 * 7))
    } else if secs < 86400 * 365 {
        format!("{}mo ago", secs / (86400 * 30))
    } else {
        format!("{}y ago", secs / (86400 * 365))
    }
}

/// Char-safe truncation with an ellipsis (tile overlays are narrow, so long
/// author names must never push the layout out).
fn ellipsize(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

fn badge(label: String, color: [f32; 3], size: u32) -> iced::Element<'static, Message> {
    iced::widget::container(
        iced::widget::text(label)
            .size(size)
            .color(iced::Color::WHITE),
    )
    .padding(iced::Padding::from([1, 3]))
    .style(move |_theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(iced::Color::from_rgb(
            color[0], color[1], color[2],
        ))),
        border: iced::Border {
            radius: 2.0.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsize_caps_and_appends_ellipsis() {
        assert_eq!(ellipsize("short", 10), "short");
        assert_eq!(ellipsize("0123456789", 10), "0123456789");
        let out = ellipsize("0123456789x", 10);
        assert_eq!(out.chars().count(), 10, "cap + ellipsis");
        assert!(out.ends_with('…'));
    }

    #[test]
    fn ellipsize_is_char_boundary_safe() {
        // Multibyte characters must never be cut mid-glyph.
        let out = ellipsize("日本語のテキストです", 4);
        assert_eq!(out, "日本語…");
    }

    #[test]
    fn relative_date_units() {
        let now = chrono::Utc::now();
        assert_eq!(relative_date(now), "just now");
        assert_eq!(relative_date(now - chrono::Duration::minutes(5)), "5m ago");
        assert_eq!(relative_date(now - chrono::Duration::hours(3)), "3h ago");
        assert_eq!(relative_date(now - chrono::Duration::days(2)), "2d ago");
        assert_eq!(relative_date(now - chrono::Duration::weeks(3)), "3w ago");
    }

    #[test]
    fn tile_text_sizes_derive_from_base_and_clamp() {
        let default = TileText::default();
        assert_eq!(default.author_size(), 11);
        assert_eq!(default.secondary_size(), 9);
        assert_eq!(default.badge_size(), 8);

        let big = TileText {
            font_size: 18,
            ..Default::default()
        };
        assert_eq!(big.author_size(), 18);
        assert_eq!(big.secondary_size(), 16);
        assert_eq!(big.badge_size(), 15);

        let tiny = TileText {
            font_size: 1,
            ..Default::default()
        };
        assert_eq!(tiny.author_size(), TileText::MIN_FONT_SIZE);
        let huge = TileText {
            font_size: 999,
            ..Default::default()
        };
        assert_eq!(huge.author_size(), TileText::MAX_FONT_SIZE);
    }

    #[test]
    fn invert_color_contrasts_with_the_tile() {
        // Bright tile → dark text; dark tile → light text.
        let on_dark = inverted_text_color([10, 10, 10]);
        assert!(
            on_dark.r + on_dark.g + on_dark.b > 2.0,
            "light text on a dark tile: {on_dark:?}"
        );
        let on_bright = inverted_text_color([245, 245, 245]);
        assert!(
            on_bright.r + on_bright.g + on_bright.b < 1.0,
            "dark text on a bright tile: {on_bright:?}"
        );
        // A mid-gray complement would be unreadable → snaps to an extreme.
        let mid = inverted_text_color([128, 128, 128]);
        assert!(
            mid == iced::Color::WHITE || mid == iced::Color::BLACK,
            "mid-tone tile gets a readable extreme: {mid:?}"
        );
    }

    #[test]
    fn text_color_overrides_for_invert_with_an_average_and_background() {
        assert!(TileText::default().color().is_none());
        // The "background" style always uses white (dark translucent strip).
        assert_eq!(
            TileText {
                style: GalleryTextStyle::Background,
                avg: Some([0, 0, 0]),
                ..Default::default()
            }
            .color(),
            Some(iced::Color::WHITE)
        );
        assert!(TileText {
            style: GalleryTextStyle::Invert,
            avg: None,
            ..Default::default()
        }
        .color()
        .is_none());
        assert!(TileText {
            style: GalleryTextStyle::Invert,
            avg: Some([0, 0, 0]),
            ..Default::default()
        }
        .color()
        .is_some());
    }

    #[test]
    fn tile_rows_toggle_the_info_lines() {
        use pixivj_rs::model::*;

        let mut illust = Illustration {
            id: 1,
            title: None,
            illust_type: None,
            image_urls: None,
            caption: None,
            restrict: None,
            user: Some(Author {
                id: 2,
                profile_image_urls: None,
                name: "artist".into(),
                account: None,
                comment: None,
                is_followed: None,
            }),
            tags: vec![],
            tools: vec![],
            create_date: None,
            page_count: Some(2),
            width: Some(800),
            height: Some(600),
            sanity_level: None,
            x_restrict: None,
            series: None,
            meta_single_page: None,
            meta_pages: vec![],
            total_view: Some(10),
            total_bookmarks: Some(5),
            is_bookmarked: None,
            visible: None,
            is_muted: None,
            illust_ai_type: None,
            provider_data: None,
        };
        illust.x_restrict = Some(1); // R-18 badge on

        // Everything hidden → an (almost) empty overlay.
        let no_rows = TileText {
            rows: crate::app::state::TileRows {
                author: false,
                counts: false,
                meta: false,
                badges: false,
            },
            ..Default::default()
        };
        let el = illust_info(&illust, no_rows);
        drop(el);

        // Author-only → the artist button still renders.
        let only_author = TileText {
            rows: crate::app::state::TileRows {
                author: true,
                counts: false,
                meta: false,
                badges: false,
            },
            ..Default::default()
        };
        let el = illust_info(&illust, only_author);
        drop(el);
    }
}
