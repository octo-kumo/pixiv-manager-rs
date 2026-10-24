//! The app's own widget styling.
//!
//! Every control in the app is built through the constructors and style
//! functions here instead of iced's stock look, so the whole UI shares one
//! visual language: flat soft surfaces with hairline borders, rounded
//! corners, an accent color for the one thing that is "on", and calm hover
//! states. Colors all derive from the theme palette ([`crate::theme`]), so
//! dark and light stay consistent.

use crate::theme;
use iced::widget::{
    button as btn, checkbox as chk, container, progress_bar, scrollable as scr, text_input as inp,
};
use iced::{Background, Border, Color, Shadow, Theme};

/// Corner radii.
pub const RADIUS_SM: f32 = 6.0;
pub const RADIUS: f32 = 8.0;
pub const RADIUS_LG: f32 = 12.0;
pub const RADIUS_PILL: f32 = 999.0;

fn rounded(radius: f32) -> Border {
    Border {
        radius: radius.into(),
        ..Default::default()
    }
}

fn outlined(color: Color, radius: f32) -> Border {
    Border {
        color,
        width: 1.0,
        radius: radius.into(),
    }
}

// ── Constructors ───────────────────────────────────────────────────────

/// A button with the default (secondary) look; chain `.style(..)` with one of
/// the functions below for another role.
pub fn button<'a, Message>(
    content: impl Into<iced::Element<'a, Message>>,
) -> iced::widget::Button<'a, Message> {
    iced::widget::button(content)
        .padding(iced::Padding::from([4, 12]))
        .style(secondary)
}

/// A single-line input with the app look.
pub fn text_input<'a, Message: Clone>(
    placeholder: &str,
    value: &str,
) -> iced::widget::TextInput<'a, Message> {
    iced::widget::text_input(placeholder, value)
        .padding(iced::Padding::from([5, 10]))
        .style(input)
}

/// A scrollable with slim, rounded scrollbars.
pub fn scrollable<'a, Message>(
    content: impl Into<iced::Element<'a, Message>>,
) -> iced::widget::Scrollable<'a, Message> {
    iced::widget::scrollable(content).style(scroll)
}

/// A checkbox with the app look.
pub fn checkbox<'a, Message>(is_checked: bool) -> iced::widget::Checkbox<'a, Message> {
    iced::widget::checkbox(is_checked).style(check)
}

// ── Buttons ────────────────────────────────────────────────────────────

fn disabled(style: btn::Style, theme: &Theme) -> btn::Style {
    btn::Style {
        background: style.background.map(|b| match b {
            Background::Color(c) => Background::Color(c.scale_alpha(0.5)),
            other => other,
        }),
        text_color: theme::muted(theme).scale_alpha(0.7),
        border: Border {
            color: style.border.color.scale_alpha(0.5),
            ..style.border
        },
        ..style
    }
}

/// Default button: a soft raised surface.
pub fn secondary(theme: &Theme, status: btn::Status) -> btn::Style {
    let base = btn::Style {
        background: Some(Background::Color(theme::panel(theme))),
        text_color: theme::fg(theme),
        border: outlined(theme::border(theme), RADIUS),
        shadow: Shadow::default(),
        snap: false,
    };
    match status {
        btn::Status::Active => base,
        btn::Status::Hovered => btn::Style {
            background: Some(Background::Color(theme::panel_hover(theme))),
            border: outlined(theme::border_strong(theme), RADIUS),
            ..base
        },
        btn::Status::Pressed => btn::Style {
            background: Some(Background::Color(theme::panel_pressed(theme))),
            ..base
        },
        btn::Status::Disabled => disabled(base, theme),
    }
}

/// The one call-to-action: filled with the accent.
pub fn primary(theme: &Theme, status: btn::Status) -> btn::Style {
    let accent = theme::accent(theme);
    let base = btn::Style {
        background: Some(Background::Color(accent)),
        text_color: theme::on_accent(theme),
        border: rounded(RADIUS),
        shadow: Shadow::default(),
        snap: false,
    };
    match status {
        btn::Status::Active => base,
        btn::Status::Hovered => btn::Style {
            background: Some(Background::Color(theme::mix(accent, Color::WHITE, 0.14))),
            ..base
        },
        btn::Status::Pressed => btn::Style {
            background: Some(Background::Color(theme::mix(accent, Color::BLACK, 0.14))),
            ..base
        },
        btn::Status::Disabled => disabled(base, theme),
    }
}

/// Borderless, transparent until hovered — for icon buttons and dismissals.
pub fn ghost(theme: &Theme, status: btn::Status) -> btn::Style {
    let base = btn::Style {
        background: None,
        text_color: theme::fg(theme),
        border: rounded(RADIUS),
        shadow: Shadow::default(),
        snap: false,
    };
    match status {
        btn::Status::Active => base,
        btn::Status::Hovered => btn::Style {
            background: Some(Background::Color(theme::panel_hover(theme))),
            ..base
        },
        btn::Status::Pressed => btn::Style {
            background: Some(Background::Color(theme::panel_pressed(theme))),
            ..base
        },
        btn::Status::Disabled => disabled(base, theme),
    }
}

/// A destructive action (stop, remove).
pub fn danger(theme: &Theme, status: btn::Status) -> btn::Style {
    let color = theme.extended_palette().danger.base.color;
    let base = btn::Style {
        background: Some(Background::Color(color.scale_alpha(0.14))),
        text_color: color,
        border: outlined(color.scale_alpha(0.55), RADIUS),
        shadow: Shadow::default(),
        snap: false,
    };
    match status {
        btn::Status::Active => base,
        btn::Status::Hovered | btn::Status::Pressed => btn::Style {
            background: Some(Background::Color(color.scale_alpha(0.28))),
            ..base
        },
        btn::Status::Disabled => disabled(base, theme),
    }
}

/// A choice chip / segmented option: tinted with the accent while selected,
/// quiet otherwise.
pub fn chip(selected: bool) -> impl Fn(&Theme, btn::Status) -> btn::Style {
    move |theme, status| {
        let accent = theme::accent(theme);
        let hovered = matches!(status, btn::Status::Hovered | btn::Status::Pressed);
        let base = if selected {
            btn::Style {
                background: Some(Background::Color(accent.scale_alpha(if hovered {
                    0.30
                } else {
                    0.20
                }))),
                text_color: theme::fg(theme),
                border: outlined(accent.scale_alpha(0.85), RADIUS),
                shadow: Shadow::default(),
                snap: false,
            }
        } else {
            btn::Style {
                background: Some(Background::Color(if hovered {
                    theme::panel_hover(theme)
                } else {
                    theme::panel(theme).scale_alpha(0.55)
                })),
                text_color: if hovered {
                    theme::fg(theme)
                } else {
                    theme::muted(theme)
                },
                border: outlined(theme::border(theme).scale_alpha(0.7), RADIUS),
                shadow: Shadow::default(),
                snap: false,
            }
        };
        if status == btn::Status::Disabled {
            disabled(base, theme)
        } else {
            base
        }
    }
}

/// A fully rounded pill — tags, parsed query tokens, suggestions.
pub fn pill(tint: Option<Color>) -> impl Fn(&Theme, btn::Status) -> btn::Style {
    move |theme, status| {
        let hovered = matches!(status, btn::Status::Hovered | btn::Status::Pressed);
        let (bg, line) = match tint {
            Some(c) => (
                c.scale_alpha(if hovered { 0.30 } else { 0.16 }),
                c.scale_alpha(0.8),
            ),
            None => (
                if hovered {
                    theme::panel_hover(theme)
                } else {
                    theme::panel(theme)
                },
                theme::border(theme),
            ),
        };
        btn::Style {
            background: Some(Background::Color(bg)),
            text_color: theme::fg(theme),
            border: outlined(line, RADIUS_PILL),
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

/// A tab in the top bar: text with an accent underline when active.
pub fn tab(active: bool) -> impl Fn(&Theme, btn::Status) -> btn::Style {
    move |theme, status| {
        let hovered = matches!(status, btn::Status::Hovered | btn::Status::Pressed);
        btn::Style {
            background: if active {
                Some(Background::Color(theme::accent(theme).scale_alpha(0.14)))
            } else if hovered {
                Some(Background::Color(theme::panel(theme)))
            } else {
                None
            },
            text_color: if active || hovered {
                theme::fg(theme)
            } else {
                theme::muted(theme)
            },
            border: rounded(RADIUS),
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

/// A button floating over an image: dark translucent glass.
pub fn glass(active: bool) -> impl Fn(&Theme, btn::Status) -> btn::Style {
    move |theme, status| {
        let hovered = matches!(status, btn::Status::Hovered | btn::Status::Pressed);
        btn::Style {
            background: Some(Background::Color(if active {
                theme::accent(theme)
            } else if hovered {
                Color::from_rgba(1.0, 1.0, 1.0, 0.18)
            } else {
                Color::TRANSPARENT
            })),
            text_color: Color::from_rgb(0.95, 0.95, 0.98),
            border: rounded(RADIUS_SM),
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

// ── Containers ─────────────────────────────────────────────────────────

/// A card: raised surface with a hairline border.
pub fn card(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme::panel(theme))),
        text_color: Some(theme::fg(theme)),
        border: outlined(theme::border(theme), RADIUS_LG),
        ..Default::default()
    }
}

/// The page background.
pub fn page(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme::bg(theme))),
        text_color: Some(theme::fg(theme)),
        ..Default::default()
    }
}

/// A translucent floating bar/pill over imagery.
pub fn floating(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme::bg(theme).scale_alpha(0.78))),
        text_color: Some(theme::fg(theme)),
        border: outlined(theme::border(theme).scale_alpha(0.6), RADIUS_LG),
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.35),
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: 10.0,
        },
        ..Default::default()
    }
}

/// A dark glass bar over images (always dark: images are arbitrary).
pub fn glass_bar(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(0.06, 0.07, 0.09, 0.66))),
        text_color: Some(Color::from_rgb(0.95, 0.95, 0.98)),
        border: rounded(RADIUS_LG),
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.3),
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: 8.0,
        },
        ..Default::default()
    }
}

/// A small static label chip (tags, tools).
pub fn tag(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme::panel(theme))),
        text_color: Some(theme::muted(theme)),
        border: outlined(theme::border(theme).scale_alpha(0.6), RADIUS_PILL),
        ..Default::default()
    }
}

/// Tooltip bubble.
pub fn tooltip(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme::panel_hover(theme))),
        text_color: Some(theme::fg(theme)),
        border: outlined(theme::border_strong(theme), RADIUS_SM),
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.3),
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: 6.0,
        },
        ..Default::default()
    }
}

/// Error banner/panel.
pub fn error_banner(theme: &Theme) -> container::Style {
    let danger = theme.extended_palette().danger.base.color;
    container::Style {
        background: Some(Background::Color(theme::mix(
            theme::bg(theme),
            danger,
            0.22,
        ))),
        text_color: Some(theme::fg(theme)),
        border: outlined(danger.scale_alpha(0.6), RADIUS),
        ..Default::default()
    }
}

/// A scrim that dims what is behind a modal.
pub fn scrim(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.55))),
        ..Default::default()
    }
}

// ── Inputs, scrollbars, checkboxes, progress ───────────────────────────

/// Text input: filled surface, hairline border, accent ring when focused.
pub fn input(theme: &Theme, status: inp::Status) -> inp::Style {
    let accent = theme::accent(theme);
    let border = match status {
        inp::Status::Active => outlined(theme::border(theme), RADIUS),
        inp::Status::Hovered => outlined(theme::border_strong(theme), RADIUS),
        inp::Status::Focused { .. } => Border {
            color: accent,
            width: 1.5,
            radius: RADIUS.into(),
        },
        inp::Status::Disabled => outlined(theme::border(theme).scale_alpha(0.5), RADIUS),
    };
    inp::Style {
        background: Background::Color(match status {
            inp::Status::Disabled => theme::panel(theme).scale_alpha(0.5),
            _ => theme::mix(theme::bg(theme), theme::fg(theme), 0.035),
        }),
        border,
        icon: theme::muted(theme),
        placeholder: theme::muted(theme).scale_alpha(0.8),
        value: theme::fg(theme),
        selection: accent.scale_alpha(0.35),
    }
}

/// Slim rounded scrollbars that brighten on hover/drag.
pub fn scroll(theme: &Theme, status: scr::Status) -> scr::Style {
    let mut style = scr::default(theme, status);
    let (v_hot, h_hot) = match status {
        scr::Status::Active { .. } => (false, false),
        scr::Status::Hovered {
            is_vertical_scrollbar_hovered,
            is_horizontal_scrollbar_hovered,
            ..
        } => (
            is_vertical_scrollbar_hovered,
            is_horizontal_scrollbar_hovered,
        ),
        scr::Status::Dragged {
            is_vertical_scrollbar_dragged,
            is_horizontal_scrollbar_dragged,
            ..
        } => (
            is_vertical_scrollbar_dragged,
            is_horizontal_scrollbar_dragged,
        ),
    };
    let rail = |hot: bool| scr::Rail {
        background: None,
        border: rounded(RADIUS_PILL),
        scroller: scr::Scroller {
            background: Background::Color(if hot {
                theme::accent(theme).scale_alpha(0.8)
            } else {
                theme::fg(theme).scale_alpha(0.22)
            }),
            border: rounded(RADIUS_PILL),
        },
    };
    style.vertical_rail = rail(v_hot);
    style.horizontal_rail = rail(h_hot);
    style.gap = None;
    style
}

/// Rounded checkbox filled with the accent when checked.
pub fn check(theme: &Theme, status: chk::Status) -> chk::Style {
    let accent = theme::accent(theme);
    let (checked, hovered) = match status {
        chk::Status::Active { is_checked } => (is_checked, false),
        chk::Status::Hovered { is_checked } => (is_checked, true),
        chk::Status::Disabled { is_checked } => (is_checked, false),
    };
    chk::Style {
        background: Background::Color(if checked {
            accent
        } else if hovered {
            theme::panel_hover(theme)
        } else {
            theme::panel(theme)
        }),
        icon_color: theme::on_accent(theme),
        border: Border {
            color: if checked {
                accent
            } else if hovered {
                theme::border_strong(theme)
            } else {
                theme::border(theme)
            },
            width: 1.0,
            radius: 5.0.into(),
        },
        text_color: Some(theme::fg(theme)),
    }
}

/// Slim rounded progress bar.
pub fn progress(theme: &Theme) -> progress_bar::Style {
    progress_bar::Style {
        background: Background::Color(theme::panel_hover(theme)),
        bar: Background::Color(theme::accent(theme)),
        border: rounded(RADIUS_PILL),
    }
}
