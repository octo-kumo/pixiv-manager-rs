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
use iced::{Color, Theme};

// ── Palette ────────────────────────────────────────────────────────────

pub const BASE: Color = color!(0x1e, 0x1e, 0x2e);
pub const SURFACE: Color = color!(0x31, 0x32, 0x44);
pub const OVERLAY: Color = color!(0x45, 0x47, 0x5a);
pub const MUTED: Color = color!(0x6c, 0x70, 0x86);
pub const SUBTLE: Color = color!(0x58, 0x5b, 0x70);
pub const TEXT: Color = color!(0xcd, 0xd6, 0xf4);
pub const ACCENT: Color = color!(0x89, 0xb4, 0xfa);
pub const GREEN: Color = color!(0xa6, 0xe3, 0xa1);
pub const RED: Color = color!(0xf3, 0x8b, 0xa8);
pub const YELLOW: Color = color!(0xf9, 0xe2, 0xaf);
pub const PEACH: Color = color!(0xfa, 0xb3, 0x87);

/// Resolve the [`iced::Theme`] for the current state.
///
/// * `System` → follow the OS dark/light mode (defaults to dark until the
///   OS reports its preference).
/// * `Light` / `Dark` → manual override.
pub fn theme(state: &AppState) -> Theme {
    match state.settings.theme {
        ThemePreference::Light => Theme::Light,
        ThemePreference::Dark => Theme::Dark,
        ThemePreference::System => match state.theme_mode {
            Mode::Light => Theme::Light,
            _ => Theme::Dark,
        },
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

/// Elevated panel/surface background.
pub fn panel(theme: &Theme) -> Color {
    theme.extended_palette().background.weak.color
}

/// Border color.
pub fn border(theme: &Theme) -> Color {
    theme.extended_palette().background.strong.color
}

/// Muted/secondary text color.
pub fn muted(theme: &Theme) -> Color {
    theme.extended_palette().background.weak.text
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
