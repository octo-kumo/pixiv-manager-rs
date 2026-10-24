//! Custom dark theme for Pixiv Manager.
//!
//! The app follows the OS dark/light preference by default (`System`), with
//! an optional manual override. All built-in iced widgets read the resolved
//! [`iced::Theme`]; custom-styled widgets should use the helpers below so
//! foreground/background colors adapt in both dark and light modes.

#![allow(dead_code)]

use crate::app::state::{AppState, ThemePreference};
use iced::color;
use iced::theme::Mode;
use iced::theme::Palette;
use iced::{Color, Theme};
use std::sync::OnceLock;

// ── Palette ────────────────────────────────────────────────────────────

/// The app's own palettes. Everything else — button/input/scrollbar styles in
/// [`crate::ui`], the helpers below — derives from these, so retuning the
/// look means editing only this block.
const DARK: Palette = Palette {
    background: color!(0x0f, 0x11, 0x16),
    text: color!(0xe8, 0xea, 0xf0),
    primary: color!(0x6e, 0x8b, 0xff),
    success: color!(0x3d, 0xd6, 0x8c),
    warning: color!(0xf5, 0xa5, 0x24),
    danger: color!(0xf4, 0x60, 0x7a),
};

const LIGHT: Palette = Palette {
    background: color!(0xf5, 0xf6, 0xfa),
    text: color!(0x17, 0x1a, 0x21),
    primary: color!(0x4a, 0x63, 0xe7),
    success: color!(0x16, 0xa0, 0x68),
    warning: color!(0xd9, 0x8a, 0x0b),
    danger: color!(0xd9, 0x3a, 0x55),
};

/// Resolve the [`iced::Theme`] for the current state.
///
/// * `System` → follow the OS dark/light mode (defaults to dark until the
///   OS reports its preference).
/// * `Light` / `Dark` → manual override.
pub fn theme(state: &AppState) -> Theme {
    let light = match state.settings.theme {
        ThemePreference::Light => true,
        ThemePreference::Dark => false,
        ThemePreference::System => state.theme_mode == Mode::Light,
    };
    static THEMES: OnceLock<(Theme, Theme)> = OnceLock::new();
    let (dark, bright) = THEMES.get_or_init(|| {
        (
            Theme::custom("Pixiv Manager Dark", DARK),
            Theme::custom("Pixiv Manager Light", LIGHT),
        )
    });
    if light {
        bright.clone()
    } else {
        dark.clone()
    }
}

/// `a` blended toward `b` by `t` (0 = a, 1 = b), opaque.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: 1.0,
    }
}

// ── Theme-derived colors ───────────────────────────────────────────────
// These read from the active theme so foreground/background adapt to
// dark/light everywhere, instead of hardcoded colors that only fit one.

/// App background color.
pub fn bg(theme: &Theme) -> Color {
    theme.extended_palette().background.base.color
}

/// Primary foreground (text) color.
pub fn fg(theme: &Theme) -> Color {
    theme.extended_palette().background.base.text
}

/// Raised surface (cards, inputs, inactive controls): the background nudged
/// toward the text color.
pub fn panel(theme: &Theme) -> Color {
    mix(bg(theme), fg(theme), 0.06)
}

/// Hovered surface.
pub fn panel_hover(theme: &Theme) -> Color {
    mix(bg(theme), fg(theme), 0.11)
}

/// Pressed surface.
pub fn panel_pressed(theme: &Theme) -> Color {
    mix(bg(theme), fg(theme), 0.16)
}

/// Hairline border color.
pub fn border(theme: &Theme) -> Color {
    mix(bg(theme), fg(theme), 0.16)
}

/// Border color of emphasized/hovered controls.
pub fn border_strong(theme: &Theme) -> Color {
    mix(bg(theme), fg(theme), 0.30)
}

/// Muted/secondary text color.
pub fn muted(theme: &Theme) -> Color {
    mix(bg(theme), fg(theme), 0.62)
}

/// The theme's primary/accent color.
pub fn accent(theme: &Theme) -> Color {
    theme.extended_palette().primary.base.color
}

/// Readable text/icon color on top of an [`accent`] background.
pub fn on_accent(theme: &Theme) -> Color {
    theme.extended_palette().primary.base.text
}

// ── Semantic status glyphs ─────────────────────────────────────────────
// Deliberate constants (not theme-derived): they must read as success /
// danger / neutral over BOTH dark and light surfaces.

/// Success / active direction (✓, ↓).
pub const SUCCESS: Color = Color::from_rgb(0.16, 0.65, 0.30);
/// Danger / exclusion (✗, ↑, remove buttons).
pub const DANGER: Color = Color::from_rgb(0.80, 0.22, 0.22);
/// Neutral / indeterminate glyph.
pub const NEUTRAL: Color = Color::from_rgb(0.55, 0.55, 0.58);
