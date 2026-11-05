//! The info overlay drawn on top of a gallery tile.
//!
//! The tile is mostly image: at rest it shows only a slim bottom bar (author
//! and compact counts) on a soft gradient scrim, plus small translucent chips
//! in the top corners (downloaded, page count, R-18, AI, provider). Hovering
//! the tile reveals the rest — title, dimensions, date and id — above the
//! bar. Every tint is derived from the thumbnail's average color, so the
//! overlay blends with the image underneath instead of sitting on it as a
//! flat black block.

use crate::app::message::Message;
use iced::widget::{container, row, svg, text as txt, Column, Row, Space};
use iced::{Background, Border, Color, Element, Font, Length, Padding};
use pixivj_rs::model::Illustration;
use std::sync::OnceLock;

/// How the gallery tile's info text is drawn over the thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum GalleryTextStyle {
    /// White text on a soft gradient scrim tinted with the thumbnail's
    /// average color ("Adaptive" in Settings).
    #[default]
    #[serde(rename = "default")]
    Default,
    /// Inverted (XOR-style) text: each glyph takes the complement of the
    /// thumbnail's average color, with no scrim at all. (iced exposes no
    /// per-pixel blend modes, so this uses the tile's average color as the
    /// XOR target.)
    #[serde(rename = "invert")]
    Invert,
    /// Text over a solid half-transparent dark strip.
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
    /// Average RGB of the tile's thumbnail: tints the scrim and chips, and
    /// is the XOR target of [`GalleryTextStyle::Invert`].
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
    /// The default base size.
    pub const DEFAULT_FONT_SIZE: u32 = 13;

    fn base(&self) -> u32 {
        self.font_size
            .clamp(Self::MIN_FONT_SIZE, Self::MAX_FONT_SIZE)
    }

    /// The author line's size (the configured base).
    pub fn author_size(&self) -> u32 {
        self.base()
    }

    /// Counts/meta line size (base − 1: close enough to stay readable).
    pub fn secondary_size(&self) -> u32 {
        self.base().saturating_sub(1).max(7)
    }

    /// Chip label size (base − 2).
    pub fn badge_size(&self) -> u32 {
        self.base().saturating_sub(2).max(7)
    }

    /// The hover title's size (base + 1).
    pub fn title_size(&self) -> u32 {
        self.base() + 1
    }

    /// The explicit text color for the info lines, or `None` to inherit the
    /// theme's default text color. The scrim styles always use white.
    pub fn color(&self) -> Option<Color> {
        match self.style {
            GalleryTextStyle::Default | GalleryTextStyle::Background => Some(Color::WHITE),
            GalleryTextStyle::Invert => self.avg.map(inverted_text_color),
        }
    }

    /// The thumbnail's dominant color pushed dark: the base of every
    /// translucent surface on the tile, so scrims and chips pick up the
    /// image's hue instead of being a flat gray.
    pub fn shade(&self) -> Color {
        shade_of(self.avg)
    }
}

/// A dark, slightly saturated version of `avg` (black when unknown). Kept
/// dark enough that white text on it is always readable.
fn shade_of(avg: Option<[u8; 3]>) -> Color {
    match avg {
        Some([r, g, b]) => {
            let f = |c: u8| c as f32 / 255.0;
            let (r, g, b) = (f(r), f(g), f(b));
            // Boost saturation a little around the mean, then darken hard.
            let mean = (r + g + b) / 3.0;
            let sat = |c: f32| (mean + (c - mean) * 1.4).clamp(0.0, 1.0) * 0.22;
            Color::from_rgb(sat(r), sat(g), sat(b))
        }
        None => Color::from_rgb(0.03, 0.03, 0.04),
    }
}

/// The "inverted" text color for a thumbnail's average color: the complement,
/// snapped to black/white when the complement is too close in luminance to be
/// readable (a mid-gray tile would otherwise render invisible glyphs).
fn inverted_text_color(avg: [u8; 3]) -> Color {
    let complement = [255 - avg[0], 255 - avg[1], 255 - avg[2]];
    let lum = |c: [u8; 3]| 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
    if (lum(complement) - lum(avg)).abs() < 60.0 {
        // XOR of a mid-tone stays mid-tone; use the readable extreme instead.
        if lum(avg) < 128.0 {
            Color::WHITE
        } else {
            Color::BLACK
        }
    } else {
        Color::from_rgb8(complement[0], complement[1], complement[2])
    }
}

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

/// Small monochrome SVG icons (24×24, tinted at draw time). The handles are
/// built once so the renderer's raster cache keys stay stable.
pub(crate) mod icons {
    use super::*;

    fn handle(body: &'static str) -> svg::Handle {
        svg::Handle::from_memory(
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##
            )
            .into_bytes(),
        )
    }

    /// Two stacked sheets: a multi-page work.
    pub fn pages() -> &'static svg::Handle {
        static H: OnceLock<svg::Handle> = OnceLock::new();
        H.get_or_init(|| {
        handle(
            r##"<rect x="8" y="3" width="13" height="15" rx="2" fill="#fff" fill-opacity="0.25"/><path d="M4 7v12a2 2 0 0 0 2 2h10"/>"##,
        )
    })
    }
    /// A filled heart (likes on Twitter/Bilibili).
    pub fn heart() -> &'static svg::Handle {
        static H: OnceLock<svg::Handle> = OnceLock::new();
        H.get_or_init(|| {
        handle(
            r##"<path fill="#fff" stroke-width="1.5" d="M12 20.5s-7.5-4.6-9.2-9.4C1.6 7.6 3.9 4.5 7.3 4.5c2 0 3.6 1.1 4.7 2.7 1.1-1.6 2.7-2.7 4.7-2.7 3.4 0 5.7 3.1 4.5 6.6-1.7 4.8-9.2 9.4-9.2 9.4z"/>"##,
        )
    })
    }
    /// A filled star (Pixiv bookmarks).
    pub fn star() -> &'static svg::Handle {
        static H: OnceLock<svg::Handle> = OnceLock::new();
        H.get_or_init(|| {
        handle(
            r##"<path fill="#fff" stroke-width="1.5" d="M12 2.8l2.8 5.8 6.3.9-4.6 4.4 1.1 6.3L12 17.2l-5.6 3 1.1-6.3-4.6-4.4 6.3-.9z"/>"##,
        )
    })
    }
    /// An open eye (views).
    pub fn eye() -> &'static svg::Handle {
        static H: OnceLock<svg::Handle> = OnceLock::new();
        H.get_or_init(|| {
        handle(
            r#"<path d="M1.5 12S5.5 4.5 12 4.5 22.5 12 22.5 12 18.5 19.5 12 19.5 1.5 12 1.5 12z"/><circle cx="12" cy="12" r="3.2"/>"#,
        )
    })
    }
    /// A check mark (downloaded to disk).
    pub fn check() -> &'static svg::Handle {
        static H: OnceLock<svg::Handle> = OnceLock::new();
        H.get_or_init(|| handle(r#"<path stroke-width="3" d="M4.5 12.5l5 5 10-11"/>"#))
    }

    /// A tinted icon sized to sit inline with text of `size` px.
    pub fn icon<'a>(handle: &svg::Handle, size: u32, color: Color) -> Element<'a, Message> {
        let px = size as f32 * 0.95;
        svg(handle.clone())
            .width(px)
            .height(px)
            .style(move |_, _| svg::Style { color: Some(color) })
            .into()
    }
}

// ---------------------------------------------------------------------------
// Corner chips
// ---------------------------------------------------------------------------

/// What a chip's surface looks like.
#[derive(Clone, Copy)]
enum ChipKind {
    /// Glassy chip tinted with the image: neutral info (page count,
    /// provider).
    Glass,
    /// A solid-ish colored chip that must stand out (R-18, AI, downloaded).
    Signal([f32; 3]),
}

fn chip<'a>(
    content: Element<'a, Message>,
    kind: ChipKind,
    shade: Color,
    padding: Padding,
) -> Element<'a, Message> {
    container(content)
        .padding(padding)
        .style(move |_| {
            let (bg, border) = match kind {
                ChipKind::Glass => (
                    Color { a: 0.55, ..shade },
                    Color::from_rgba(1.0, 1.0, 1.0, 0.22),
                ),
                ChipKind::Signal([r, g, b]) => (
                    Color::from_rgba(r, g, b, 0.82),
                    Color::from_rgba(1.0, 1.0, 1.0, 0.28),
                ),
            };
            container::Style {
                background: Some(Background::Color(bg)),
                border: Border {
                    color: border,
                    width: 1.0,
                    radius: 999.0.into(),
                },
                text_color: Some(Color::WHITE),
                ..Default::default()
            }
        })
        .into()
}

fn chip_label<'a>(label: impl txt::IntoFragment<'a>, size: u32) -> Element<'a, Message> {
    txt(label)
        .size(size)
        .font(SEMIBOLD)
        .color(Color::WHITE)
        .into()
}

/// Below this tile width the chips tighten their padding.
const COMPACT_CHIPS_BELOW: f32 = 170.0;

/// The top-of-tile chip strip: status/content chips on the left
/// (downloaded ✓, page count, R-18, AI) and the provider on the right.
/// Chips that the user turned off via the "badges" row are omitted, but the
/// downloaded check always shows (it is state, not metadata).
///
/// `tile_width` is the tile's logical width. Chip widths are estimated and
/// the strip is budgeted: on narrow tiles the lowest-priority chips (AI,
/// page count, provider) are dropped and the rest tighten their padding, so
/// the strip never spills past the tile edge.
pub fn corner_chips(
    illust: &Illustration,
    text: TileText,
    downloaded: bool,
    tile_width: Option<f32>,
) -> Element<'_, Message> {
    let size = text.badge_size();
    let shade = text.shade();
    let width = tile_width.unwrap_or(f32::INFINITY);
    let compact = width < COMPACT_CHIPS_BELOW;
    let padding = if compact {
        Padding::from([1, 4])
    } else {
        Padding::from([2, 6])
    };
    let pad_x = if compact { 4.0 } else { 6.0 };
    let border = 2.0; // 1 px on each side
    let icon_w = size as f32 * 0.95;
    let label_w = |label: &str| text_width(label, size as f32) + 2.0 * pad_x + border;
    let check_w = icon_w + 2.0 * pad_x + border;
    let pages = illust.page_count();
    let pages_w = icon_w + 3.0 + text_width(&pages.to_string(), size as f32) + 2.0 * pad_x + border;
    let r18_w = label_w("R-18");
    let ai_w = label_w("AI");
    let provider = crate::media::provider_of(illust.id).badge();
    let provider_w = provider.map(label_w).unwrap_or(0.0);

    // Strip width inside the container's 6 px horizontal padding; 4 px gaps.
    let available = width - 12.0;
    let gap = 4.0;
    let strip_width =
        |widths: &[f32]| widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;

    let show_r18 = text.rows.badges && illust.is_r18();
    let mut widths: Vec<f32> = Vec::new();
    if downloaded {
        widths.push(check_w);
    }
    if show_r18 {
        widths.push(r18_w);
    }
    // Keep the provider when it fits next to the mandatory chips.
    let mut show_provider = provider.is_some();
    if show_provider {
        let mut with_provider = widths.clone();
        with_provider.push(provider_w);
        if strip_width(&with_provider) > available {
            show_provider = false;
        }
    }
    // Optional chips are added by priority (page count first, then AI).
    let mut show_pages = false;
    if text.rows.badges && pages > 1 {
        let mut test = widths.clone();
        test.push(pages_w);
        if show_provider {
            test.push(provider_w);
        }
        if strip_width(&test) <= available {
            show_pages = true;
            widths.push(pages_w);
        }
    }
    let mut show_ai = false;
    if text.rows.badges && illust.is_ai() {
        let mut test = widths.clone();
        test.push(ai_w);
        if show_provider {
            test.push(provider_w);
        }
        if strip_width(&test) <= available {
            show_ai = true;
        }
    }

    // Rebuild the strip in visual order: check, pages, R-18, AI, provider.
    let mut left: Vec<Element<'_, Message>> = Vec::new();
    if downloaded {
        left.push(chip(
            icons::icon(icons::check(), size, Color::WHITE),
            ChipKind::Signal([0.18, 0.62, 0.32]),
            shade,
            padding,
        ));
    }
    if show_pages {
        left.push(chip(
            row![
                icons::icon(icons::pages(), size, Color::WHITE),
                chip_label(pages.to_string(), size)
            ]
            .spacing(3)
            .align_y(iced::Alignment::Center)
            .into(),
            ChipKind::Glass,
            shade,
            padding,
        ));
    }
    if show_r18 {
        left.push(chip(
            chip_label("R-18", size),
            ChipKind::Signal([0.86, 0.16, 0.24]),
            shade,
            padding,
        ));
    }
    if show_ai {
        left.push(chip(
            chip_label("AI", size),
            ChipKind::Signal([0.47, 0.30, 0.82]),
            shade,
            padding,
        ));
    }

    let mut strip = Row::with_children(left)
        .spacing(4)
        .align_y(iced::Alignment::Center)
        .push(Space::new().width(Length::Fill));
    if let (true, Some(label)) = (show_provider, provider) {
        strip = strip.push(chip(
            chip_label(label, size),
            ChipKind::Glass,
            shade,
            padding,
        ));
    }
    container(strip).width(Length::Fill).padding(6).into()
}

// ---------------------------------------------------------------------------
// Bottom info
// ---------------------------------------------------------------------------

const SEMIBOLD: Font = Font {
    weight: iced::font::Weight::Semibold,
    ..Font::DEFAULT
};

/// Compact human count: 999, 1.2k, 12k, 1.2M, 12M.
pub fn compact_count(n: u64) -> String {
    fn one_decimal(v: f64, unit: &str) -> String {
        let s = format!("{v:.1}");
        let s = s.strip_suffix(".0").unwrap_or(&s);
        format!("{s}{unit}")
    }
    match n {
        0..=999 => n.to_string(),
        1_000..=9_999 => one_decimal(n as f64 / 1e3, "k"),
        10_000..=999_999 => format!("{}k", n / 1_000),
        1_000_000..=9_999_999 => one_decimal(n as f64 / 1e6, "M"),
        _ => format!("{}M", n / 1_000_000),
    }
}

fn colored_text<'a>(
    label: impl txt::IntoFragment<'a>,
    size: u32,
    color: Option<Color>,
) -> iced::widget::Text<'a> {
    let t = txt(label).size(size).wrapping(txt::Wrapping::None);
    match color {
        Some(c) => t.color(c),
        None => t,
    }
}

/// A text line that wraps at word boundaries instead of overflowing its
/// tile: long hover details (dimensions/age/id) must never be cut mid-string.
fn colored_text_wrapped<'a>(
    label: impl txt::IntoFragment<'a>,
    size: u32,
    color: Option<Color>,
) -> iced::widget::Text<'a> {
    let t = txt(label).size(size).wrapping(txt::Wrapping::WordOrGlyph);
    match color {
        Some(c) => t.color(c),
        None => t,
    }
}

/// `color` faded for secondary information.
fn soft(color: Option<Color>) -> Option<Color> {
    color.map(|c| Color { a: c.a * 0.78, ..c })
}

/// Below this tile width the view count is dropped: the bar is too narrow
/// for author + two stats, and the bookmark count is the more useful one.
const VIEWS_MIN_WIDTH: f32 = 170.0;

/// iced's default paragraph line height, as a multiple of the font size.
const LINE_HEIGHT_FACTOR: f32 = 1.3;
/// Approximate advance of an ASCII glyph, as a fraction of the font size.
/// Non-ASCII glyphs count as one full em: CJK names and titles are common
/// here, and overestimating their width only makes the layout budget safer.
const ASCII_ADVANCE_FACTOR: f32 = 0.62;

fn line_height(size: u32) -> f32 {
    size as f32 * LINE_HEIGHT_FACTOR
}

fn char_advance(c: char, size: f32) -> f32 {
    if c.is_ascii() {
        size * ASCII_ADVANCE_FACTOR
    } else {
        size
    }
}

/// Approximate rendered width of `s` at `size` px. Only used to budget the
/// overlay; the widgets themselves do the final layout and wrapping.
fn text_width(s: &str, size: f32) -> f32 {
    s.chars().map(|c| char_advance(c, size)).sum()
}

/// Char-safe truncation to a pixel budget, ending in `…` when it cuts.
fn ellipsize_to_width(s: &str, size: u32, budget: f32) -> String {
    let size = size as f32;
    if text_width(s, size) <= budget {
        return s.to_string();
    }
    let ellipsis = char_advance('…', size);
    let mut out = String::new();
    let mut used = 0.0;
    for c in s.chars() {
        let advance = char_advance(c, size);
        if used + advance + ellipsis > budget {
            break;
        }
        out.push(c);
        used += advance;
    }
    out.push('…');
    out
}

/// Height (px) of the resting bar; the hover details sit directly above it.
/// Zero when the user hid both the author and count rows (no bar at rest).
pub fn resting_bar_height(text: TileText) -> f32 {
    let line = if text.rows.author {
        line_height(text.author_size())
    } else if text.rows.counts {
        line_height(text.secondary_size())
    } else {
        return 0.0;
    };
    line + 6.0 // the block's bottom padding
}

/// Approximate width of one `icon + label` count stat.
fn stat_width(text: TileText, label: &str) -> f32 {
    let size = text.secondary_size() as f32;
    size * 0.95 + 3.0 + text_width(label, size)
}

/// The always-visible bar: author on the left, counts on the right.
fn summary_bar(
    illust: &Illustration,
    text: TileText,
    tile_width: Option<f32>,
) -> Option<Element<'_, Message>> {
    let color = text.color();
    let width = tile_width.unwrap_or(f32::INFINITY);
    let mut bar = Row::new().spacing(8).align_y(iced::Alignment::Center);
    let mut any = false;

    let show_views = width >= VIEWS_MIN_WIDTH && illust.total_view.filter(|v| *v > 0).is_some();
    if text.rows.author {
        // Reserve the counts' width and shorten the name to fit: otherwise
        // the tile's clip cuts it mid-glyph with no hint that it continues.
        let counts_width = if text.rows.counts {
            let mut reserved = 8.0 // row spacing
                + stat_width(
                    text,
                    &compact_count(illust.total_bookmarks.unwrap_or(0)),
                );
            if show_views {
                reserved += 8.0 + stat_width(text, &compact_count(illust.total_view.unwrap_or(0)));
            }
            reserved
        } else {
            0.0
        };
        let name = match (&illust.user, tile_width) {
            (Some(user), Some(w)) => {
                // 8 px block padding each side, plus a little slack.
                let budget = (w - 16.0 - counts_width - 4.0).max(text.author_size() as f32 * 1.2);
                ellipsize_to_width(&user.name, text.author_size(), budget)
            }
            (Some(user), None) => ellipsize(&user.name, 28),
            (None, _) => "unknown".into(),
        };
        let label = colored_text(name, text.author_size(), color).font(SEMIBOLD);
        let author: Element<'_, Message> = match &illust.user {
            // Every provider has an author page (Pixiv profile, the user's
            // tweets, their image dynamics) — the id is namespaced and the
            // reducer dispatches on it.
            Some(user) => crate::ui::button(label)
                .on_press(Message::ArtistOpened(user.id))
                .padding(0)
                .style(|theme, status| {
                    let mut s = iced::widget::button::text(theme, status);
                    if matches!(
                        status,
                        iced::widget::button::Status::Hovered
                            | iced::widget::button::Status::Pressed
                    ) {
                        // The label carries an explicit color (readable on
                        // the image), so the affordance is a hit surface, not
                        // a text-color swap.
                        s.background =
                            Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.14)));
                        s.border.radius = 3.0.into();
                    }
                    s
                })
                .into(),
            None => label.into(),
        };
        // Fill + clip: the budget above normally keeps the name short enough,
        // but a very narrow tile still cuts at the edge instead of pushing
        // the counts out.
        bar = bar.push(container(author).width(Length::Fill).clip(true));
        any = true;
    } else {
        bar = bar.push(Space::new().width(Length::Fill));
    }

    if text.rows.counts {
        let size = text.secondary_size();
        let icon_color = soft(color).unwrap_or(Color::WHITE);
        let bookmark_icon = if crate::media::is_pixiv(illust.id) {
            icons::star()
        } else {
            icons::heart()
        };
        let stat = |icon: &svg::Handle, n: u64| -> Element<'_, Message> {
            row![
                icons::icon(icon, size, icon_color),
                colored_text(compact_count(n), size, color)
            ]
            .spacing(3)
            .align_y(iced::Alignment::Center)
            .into()
        };
        let mut counts = Row::new().spacing(8).align_y(iced::Alignment::Center);
        counts = counts.push(stat(bookmark_icon, illust.total_bookmarks.unwrap_or(0)));
        if show_views {
            counts = counts.push(stat(icons::eye(), illust.total_view.unwrap_or(0)));
        }
        bar = bar.push(counts);
        any = true;
    }

    any.then(|| bar.into())
}

/// The hover details reduced until they fit the tile. Lines are already
/// split into renderable chunks so the height budget is deterministic.
#[allow(dead_code)] // the line counts are asserted from the unit tests
struct DetailPlan {
    title: Option<String>,
    meta: Vec<String>,
    /// Estimated rendered line count (for the height budget).
    title_lines: usize,
    meta_lines: usize,
    top_padding: f32,
}

impl DetailPlan {
    fn is_empty(&self) -> bool {
        self.title.is_none() && self.meta.is_empty()
    }
}

/// How many wrapped lines a hover title may take before it is ellipsized:
/// keeps the expanded block from covering most of a tall tile.
const MAX_TITLE_LINES: usize = 3;

fn plan_details(
    illust: &Illustration,
    text: TileText,
    content_width: f32,
    tile_height: f32,
) -> DetailPlan {
    let title_size = text.title_size();
    let secondary = text.secondary_size();
    let raw_title = illust
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| ellipsize(&t.replace('\n', " "), 90));
    let meta = if text.rows.meta {
        meta_lines(illust, text, content_width)
    } else {
        Vec::new()
    };
    // Greedy word-wrapping can need one more line than the raw pixel width
    // suggests (word boundaries); the 1.06 factor plus the one-line slack at
    // candidate acceptance keeps the estimate on the safe side.
    let estimate_lines = |s: &str, size: u32| {
        (text_width(s, size as f32) * 1.06 / content_width)
            .ceil()
            .max(1.0) as usize
    };
    let bar_gap = resting_bar_height(text);
    let budget = (tile_height - bar_gap - 8.0).max(0.0);

    // Progressively lighter hover layouts, chosen by the tile height:
    // full title + meta → up to two title lines + meta → single-line title.
    // The expanded block must never exceed the space above the resting bar.
    let default_style = text.style == GalleryTextStyle::Default;
    let candidates = if default_style {
        [
            (36.0, MAX_TITLE_LINES, true),
            (20.0, 2, true),
            (12.0, 1, false),
        ]
    } else {
        [
            (4.0, MAX_TITLE_LINES, true),
            (4.0, 2, true),
            (4.0, 1, false),
        ]
    };
    for (top, max_title_lines, with_meta) in candidates {
        let title = raw_title
            .as_deref()
            .map(|t| ellipsize_to_width(t, title_size, content_width * max_title_lines as f32));
        let title_lines = title
            .as_deref()
            .map(|t| estimate_lines(t, title_size).min(max_title_lines))
            .unwrap_or(0);
        let meta: Vec<String> = if with_meta { meta.clone() } else { Vec::new() };
        let meta_line_count = meta
            .iter()
            .map(|line| estimate_lines(line, secondary))
            .sum::<usize>();
        if title.is_none() && meta.is_empty() {
            continue;
        }
        // The Column puts one 3 px gap between rendered elements.
        let elements = title.is_some() as usize + meta.len();
        let gaps = elements.saturating_sub(1) as f32 * 3.0;
        let needed = top
            + title_lines as f32 * line_height(title_size)
            + meta_line_count as f32 * line_height(secondary)
            + gaps
            + 6.0;
        // One line of slack: the estimate must not under-wrap into a taller
        // block than the space above the resting bar.
        if needed + line_height(secondary) <= budget {
            return DetailPlan {
                title,
                meta,
                title_lines,
                meta_lines: meta_line_count,
                top_padding: top,
            };
        }
    }

    // Not even one title line fits: show nothing, so the resting bar is
    // never squeezed out of the tile.
    DetailPlan {
        title: None,
        meta: Vec::new(),
        title_lines: 0,
        meta_lines: 0,
        top_padding: 0.0,
    }
}

/// The hover meta lines: `W×H · age · S4+`, and `#id` on its own line when
/// the combined line does not fit (so the id is never clipped).
fn meta_lines(illust: &Illustration, text: TileText, content_width: f32) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if let (Some(w), Some(h)) = (illust.width, illust.height) {
        parts.push(format!("{w}×{h}"));
    }
    if let Some(date) = illust.create_date {
        parts.push(relative_date(date));
    }
    if let Some(sanity) = illust.sanity_level.filter(|s| *s >= 4) {
        parts.push(format!("S{sanity}"));
    }
    let id = format!("#{}", crate::media::native_id(illust.id));
    if parts.is_empty() {
        return vec![id];
    }
    let head = parts.join(" · ");
    let one = format!("{head} · {id}");
    if text_width(&one, text.secondary_size() as f32) <= content_width {
        vec![one]
    } else {
        vec![head, id]
    }
}

/// The hover-only details block: title and meta lines. Returns `Space` when
/// the tile is too short for anything useful, so the resting bar is never
/// squeezed or covered.
pub fn details_block(
    illust: &Illustration,
    text: TileText,
    tile_width: Option<f32>,
    tile_height: f32,
) -> Element<'_, Message> {
    let content_width = tile_width.map(|w| (w - 16.0).max(24.0)).unwrap_or(224.0);
    let plan = plan_details(illust, text, content_width, tile_height);
    if plan.is_empty() {
        return Space::new().into();
    }
    let color = text.color();
    let mut col = Column::new().spacing(3);
    if let Some(title) = plan.title {
        col = col.push(
            txt(title)
                .size(text.title_size())
                .font(SEMIBOLD)
                .wrapping(txt::Wrapping::WordOrGlyph)
                .color_maybe(color),
        );
    }
    for line in plan.meta {
        col = col.push(colored_text_wrapped(
            line,
            text.secondary_size(),
            soft(color),
        ));
    }
    backdrop(col.into(), text, true, plan.top_padding, 6.0)
}

/// Wraps an info block in the style's backdrop. `expanded` selects the
/// stronger hover scrim; `top`/`bottom` are the block's paddings.
fn backdrop<'a>(
    content: Element<'a, Message>,
    text: TileText,
    expanded: bool,
    top: f32,
    bottom: f32,
) -> Element<'a, Message> {
    let shade = text.shade();
    let block = container(content).width(Length::Fill).padding(Padding {
        top: if text.style == GalleryTextStyle::Default {
            top
        } else {
            4.0
        },
        right: 8.0,
        bottom,
        left: 8.0,
    });
    match text.style {
        GalleryTextStyle::Default => block
            .style(move |_| {
                let peak = if expanded { 0.92 } else { 0.78 };
                let gradient = iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI))
                    .add_stop(0.0, Color { a: 0.0, ..shade })
                    .add_stop(
                        0.45,
                        Color {
                            a: peak * 0.6,
                            ..shade
                        },
                    )
                    .add_stop(1.0, Color { a: peak, ..shade });
                container::Style {
                    background: Some(Background::Gradient(gradient.into())),
                    ..Default::default()
                }
            })
            .into(),
        GalleryTextStyle::Background => block
            .style(move |_| container::Style {
                background: Some(Background::Color(Color::from_rgba(
                    0.0,
                    0.0,
                    0.0,
                    if expanded { 0.72 } else { 0.55 },
                ))),
                ..Default::default()
            })
            .into(),
        GalleryTextStyle::Invert => block.into(),
    }
}

/// The resting bottom info block (author + counts on the scrim). Returns
/// `Space` when every row is hidden, so the image stays untouched.
pub fn info_block(
    illust: &Illustration,
    text: TileText,
    tile_width: Option<f32>,
) -> Element<'_, Message> {
    let Some(bar) = summary_bar(illust, text, tile_width) else {
        return Space::new().into();
    };
    let top = if text.style == GalleryTextStyle::Default {
        22.0
    } else {
        4.0
    };
    backdrop(bar, text, false, top, 6.0)
}

/// The resting info overlay (kept for callers that do not need the hover
/// layer).
pub fn illust_info(illust: &Illustration, text: TileText) -> Element<'_, Message> {
    info_block(illust, text, None)
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
/// strings must never push the layout out).
fn ellipsize(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{cut}…")
    }
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
        assert_eq!(default.author_size(), 13);
        assert_eq!(default.secondary_size(), 12);
        assert_eq!(default.badge_size(), 11);

        let big = TileText {
            font_size: 18,
            ..Default::default()
        };
        assert_eq!(big.author_size(), 18);
        assert_eq!(big.secondary_size(), 17);
        assert_eq!(big.badge_size(), 16);

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
        // The adaptive scrim always carries white text.
        assert_eq!(TileText::default().color(), Some(iced::Color::WHITE));
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

        // Resting block, hover details and the corner chips build for every
        // style.
        for style in [
            GalleryTextStyle::Default,
            GalleryTextStyle::Invert,
            GalleryTextStyle::Background,
        ] {
            let t = TileText {
                style,
                avg: Some([200, 40, 60]),
                ..Default::default()
            };
            drop(info_block(&illust, t, None));
            drop(details_block(&illust, t, None, 400.0));
            drop(corner_chips(&illust, t, true, None));
        }

        // Narrow and short tiles must build too (chips hide/shrink by width,
        // hover details degrade by height).
        for width in [Some(104.0), Some(164.0), Some(230.0), None] {
            drop(info_block(&illust, TileText::default(), width));
            drop(details_block(&illust, TileText::default(), width, 200.0));
            drop(corner_chips(&illust, TileText::default(), true, width));
        }
        for height in [80.0, 100.0, 140.0, 200.0] {
            drop(details_block(
                &illust,
                TileText::default(),
                Some(230.0),
                height,
            ));
        }
    }

    #[test]
    fn ellipsize_to_width_budgets_pixels_not_chars() {
        // Fits → untouched.
        assert_eq!(ellipsize_to_width("short", 13, 1000.0), "short");
        // Latin glyphs are narrower than CJK ones: same budget, more chars.
        let latin = ellipsize_to_width("abcdefghij", 13, 60.0);
        let cjk = ellipsize_to_width("あいうえおかきくけこ", 13, 60.0);
        assert!(latin.ends_with('…') && cjk.ends_with('…'));
        assert!(latin.chars().count() > cjk.chars().count());
        // The result always fits the budget (with the ellipsis).
        for budget in [24.0, 48.0, 96.0] {
            let out = ellipsize_to_width("日本語のテキストです", 12, budget);
            assert!(
                text_width(&out, 12.0) <= budget,
                "{budget}: {out:?} = {}",
                text_width(&out, 12.0)
            );
        }
    }

    #[test]
    fn hover_details_degrade_on_short_tiles() {
        use pixivj_rs::model::Author;
        let illust = Illustration {
            id: 1,
            title: Some("a long enough title to wrap over several lines".into()),
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
            page_count: Some(1),
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
        let text = TileText::default();
        let content_width = 214.0;
        // Tall tile: full plan (title + meta).
        let full = plan_details(&illust, text, content_width, 300.0);
        assert!(full.title.is_some());
        assert!(!full.meta.is_empty());
        // Medium tile: still gets the meta line.
        let medium = plan_details(&illust, text, content_width, 160.0);
        assert!(medium.title.is_some());
        assert!(!medium.meta.is_empty(), "meta still fits on a 160px tile");
        // Shorter tile: title only, single line.
        let short = plan_details(&illust, text, content_width, 100.0);
        assert!(short.title.is_some());
        assert!(short.meta.is_empty(), "meta dropped on a short tile");
        assert_eq!(short.title_lines, 1);
        // A tile too short even for one title line (below the bar + padding):
        // no details at all, so the resting bar survives.
        let tiny = plan_details(&illust, text, content_width, 40.0);
        assert!(tiny.is_empty());
        // The estimated plan always fits the budget.
        for height in [40.0, 80.0, 100.0, 120.0, 160.0, 200.0, 300.0] {
            let plan = plan_details(&illust, text, content_width, height);
            let needed = plan.top_padding
                + plan.title_lines as f32 * line_height(text.title_size())
                + if plan.meta_lines > 0 {
                    3.0 + plan.meta_lines as f32 * line_height(text.secondary_size())
                } else {
                    0.0
                }
                + 6.0;
            let budget = (height - resting_bar_height(text) - 8.0).max(0.0);
            assert!(
                plan.is_empty() || needed <= budget,
                "height {height}: needed {needed} > budget {budget}"
            );
        }
    }

    #[test]
    fn meta_splits_the_id_onto_its_own_line_when_narrow() {
        use pixivj_rs::model::Author;
        let illust = Illustration {
            id: 148_432_220,
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
            create_date: Some(chrono::Utc::now() - chrono::Duration::days(60)),
            page_count: Some(1),
            width: Some(919),
            height: Some(1300),
            sanity_level: Some(4),
            x_restrict: None,
            series: None,
            meta_single_page: None,
            meta_pages: vec![],
            total_view: Some(1),
            total_bookmarks: Some(1),
            is_bookmarked: None,
            visible: None,
            is_muted: None,
            illust_ai_type: None,
            provider_data: None,
        };
        let text = TileText::default();
        // Wide: one line.
        let wide = meta_lines(&illust, text, 1000.0);
        assert_eq!(wide.len(), 1, "{wide:?}");
        assert!(wide[0].ends_with("#148432220"));
        // Narrow: id gets its own line and is complete.
        let narrow = meta_lines(&illust, text, 120.0);
        assert_eq!(narrow.len(), 2, "{narrow:?}");
        assert_eq!(narrow[1], "#148432220");
    }

    #[test]
    fn narrow_tiles_drop_low_priority_chips() {
        // The thresholds are private, but the built strip must not panic and
        // the builder must be width-sensitive: compile/run all breakpoints.
        use pixivj_rs::model::Author;
        let illust = Illustration {
            id: 1,
            title: Some("t".into()),
            illust_type: None,
            image_urls: None,
            caption: None,
            restrict: None,
            user: Some(Author {
                id: 2,
                profile_image_urls: None,
                name: "a".into(),
                account: None,
                comment: None,
                is_followed: None,
            }),
            tags: vec![],
            tools: vec![],
            create_date: None,
            page_count: Some(9),
            width: Some(10),
            height: Some(10),
            sanity_level: Some(6),
            x_restrict: Some(1),
            series: None,
            meta_single_page: None,
            meta_pages: vec![],
            total_view: Some(1),
            total_bookmarks: Some(1),
            is_bookmarked: None,
            visible: None,
            is_muted: None,
            illust_ai_type: Some(2),
            provider_data: None,
        };
        for width in [Some(60.0), Some(104.0), Some(164.0), Some(230.0)] {
            drop(corner_chips(&illust, TileText::default(), true, width));
        }
    }

    #[test]
    fn compact_counts() {
        assert_eq!(compact_count(0), "0");
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(1_000), "1k");
        assert_eq!(compact_count(1_234), "1.2k");
        assert_eq!(compact_count(11_996), "11k");
        assert_eq!(compact_count(2_500_000), "2.5M");
        assert_eq!(compact_count(42_000_000), "42M");
    }

    #[test]
    fn shade_stays_dark_enough_for_white_text() {
        for avg in [[255, 255, 255], [255, 0, 0], [0, 0, 0], [240, 220, 10]] {
            let c = shade_of(Some(avg));
            let lum = 0.299 * c.r + 0.587 * c.g + 0.114 * c.b;
            assert!(lum < 0.25, "{avg:?} -> {c:?}");
        }
    }
}
