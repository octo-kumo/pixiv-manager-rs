//! Keyboard-shortcuts cheat sheet (F1). A centered translucent panel listing
//! every binding; Esc or F1 closes it.

use crate::app::message::Message;
use crate::theme;

/// Every keyboard shortcut the app supports, shown in the overlay (and the
/// help screen's hotkeys section).
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Esc", "Close topmost overlay / panel"),
    ("←  →", "Viewer: previous / next page"),
    ("Alt+← / Alt+→", "Viewer: back / forward in history"),
    ("Home / End", "Viewer: first / last page"),
    ("↑  ↓", "Search: move autocomplete selection"),
    ("Enter", "Search: pick suggestion / run query"),
    ("Ctrl+F", "Focus the search box"),
    ("Ctrl+ +/−/0", "Grid size bigger / smaller / reset"),
    ("F1", "Toggle this help"),
    ("F3", "Toggle the debug overlay"),
];

pub fn shortcuts_overlay(t: &iced::Theme) -> iced::Element<'static, Message> {
    let rows: Vec<iced::Element<'static, Message>> = SHORTCUTS
        .iter()
        .map(|(key, action)| {
            iced::widget::row![
                iced::widget::text(*key)
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
