//! Keyboard-shortcuts cheat sheet (F1). A centered translucent panel listing
//! every binding; Esc or F1 closes it.

use crate::app::message::Message;
use crate::platform::shortcuts::primary_shortcut;
use crate::theme;

/// Every keyboard shortcut, shown in the overlay and the help screen. The
/// primary-modifier rows come from the platform layer.
pub fn shortcuts() -> Vec<(String, &'static str)> {
    vec![
        ("Esc".to_string(), "Close topmost overlay / panel"),
        ("←  →".to_string(), "Viewer: previous / next page"),
        (
            "Alt+← / Alt+→".to_string(),
            "Viewer: back / forward in history",
        ),
        ("Home / End".to_string(), "Viewer: first / last page"),
        ("↑  ↓".to_string(), "Search: move autocomplete selection"),
        ("Enter".to_string(), "Search: pick suggestion / run query"),
        (primary_shortcut("F"), "Focus the search box"),
        (primary_shortcut("+"), "Gallery grid: bigger"),
        (primary_shortcut("-"), "Gallery grid: smaller"),
        (primary_shortcut("0"), "Gallery grid: reset size"),
        ("F1".to_string(), "Toggle this help"),
        ("F3".to_string(), "Toggle the debug overlay"),
    ]
}

pub fn shortcuts_overlay(t: &iced::Theme) -> iced::Element<'static, Message> {
    let rows: Vec<iced::Element<'static, Message>> = shortcuts()
        .iter()
        .map(|(key, action)| {
            iced::widget::row![
                iced::widget::text(key.clone())
                    .size(12)
                    .color(theme::accent(t))
                    .width(iced::Length::Fixed(150.0)),
                iced::widget::text(*action).size(12),
            ]
            .spacing(8)
            .into()
        })
        .collect();

    let panel = iced::widget::container(
        iced::widget::column![
            iced::widget::text("Keyboard shortcuts").size(16),
            iced::widget::Column::with_children(rows).spacing(4),
            iced::widget::text("F1 or Esc to close")
                .size(10)
                .color(theme::muted(t)),
        ]
        .spacing(10),
    )
    .padding(16)
    .style(crate::ui::card);

    // Full-screen click-catcher so clicks outside dismiss nothing accidentally
    // but the panel floats above everything.
    iced::widget::container(panel)
        .center_x(iced::Length::Fill)
        .center_y(iced::Length::Fill)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .style(crate::ui::scrim)
        .into()
}
