//! The help screen (the "?" button next to settings): very basic usage
//! info, every keyboard shortcut, author info and the MIT license. Esc or
//! the close button dismisses it.

use crate::app::message::Message;
use crate::theme;
use crate::widgets::shortcuts::SHORTCUTS;

/// The app's author (matches the git identity / repo ownership).
const AUTHOR: &str = "yun (yun@yun.ng)";
const LICENSE: &str = "MIT License\n\n\
Copyright (c) 2026 yun\n\n\
Permission is hereby granted, free of charge, to any person obtaining a copy \
of this software and associated documentation files (the \"Software\"), to deal \
in the Software without restriction, including without limitation the rights \
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell \
copies of the Software, and to permit persons to whom the Software is \
furnished to do so, subject to the following conditions:\n\n\
The above copyright notice and this permission notice shall be included in all \
copies or substantial portions of the Software.\n\n\
THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR \
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, \
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE \
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER \
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, \
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE \
SOFTWARE.";

/// A basic-usage cheat sheet, shown above the hotkeys.
const USAGE: &[(&str, &str)] = &[
    ("Tabs", "Recommend, Feed and Leaderboard are the Pixiv feeds. Twitter and Bilibili need their cookies set in Settings. Bookmarks mixes the bookmark library of every connected source."),
    ("Search", "Type in the search bar and press Enter on the Search tab. The ⚙ Filters toggle reveals tag / tool / option filters and sorting."),
    ("Tiles", "Click a tile to open the viewer. Shift-click toggles its bookmark. Click the author name to open their page."),
    ("Viewer", "Scroll or press ← → to flip pages; the page box jumps directly. Zoom with the mouse wheel."),
    ("Downloads", "The Bookmarks tab has sync + download-all jobs; the ⬇ button in the viewer downloads one work."),
    ("Settings", "The gear button opens every option; each one has its own Reset button."),
];

fn section(title: &str, t: &iced::Theme) -> iced::Element<'static, Message> {
    iced::widget::text(title.to_string())
        .size(14)
        .color(theme::accent(t))
        .into()
}

pub fn help_screen(t: &iced::Theme) -> iced::Element<'static, Message> {
    let usage_rows: Vec<iced::Element<'static, Message>> = USAGE
        .iter()
        .map(|(topic, text)| {
            iced::widget::row![
                iced::widget::text((*topic).to_string())
                    .size(12)
                    .color(theme::accent(t))
                    .width(iced::Length::Fixed(90.0)),
                iced::widget::text((*text).to_string()).size(12),
            ]
            .spacing(8)
            .into()
        })
        .collect();

    let shortcut_rows: Vec<iced::Element<'static, Message>> = SHORTCUTS
        .iter()
        .map(|(key, action)| {
            iced::widget::row![
                iced::widget::text((*key).to_string())
                    .size(12)
                    .color(theme::accent(t))
                    .width(iced::Length::Fixed(150.0)),
                iced::widget::text((*action).to_string()).size(12),
            ]
            .spacing(8)
            .into()
        })
        .collect();

    let version = format!(
        "Pixiv Manager v{} — GUI + CLI for Pixiv/Twitter/Bilibili media.",
        env!("CARGO_PKG_VERSION")
    );

    let panel = iced::widget::container(
        iced::widget::scrollable(
            iced::widget::column![
                // Header with the close button.
                iced::widget::row![
                    iced::widget::text("Help").size(20),
                    iced::widget::Space::new().width(iced::Length::Fill),
                    iced::widget::button(iced::widget::text("✕").size(14))
                        .on_press(Message::ToggleHelp)
                        .padding(iced::Padding::from([4, 10])),
                ],
                iced::widget::text(version).size(12).color(theme::muted(t)),
                section("Basic usage", t),
                iced::widget::Column::with_children(usage_rows).spacing(4),
                section("Keyboard shortcuts", t),
                iced::widget::Column::with_children(shortcut_rows).spacing(4),
                section("Author", t),
                iced::widget::text(AUTHOR).size(12),
                section("License", t),
                iced::widget::text(LICENSE).size(11),
            ]
            .spacing(10)
            .padding(24)
            .width(720),
        )
        .height(iced::Length::Fill),
    )
    .width(720)
    .height(720)
    .padding(0)
    .style(|theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(theme::panel(theme))),
        border: iced::Border {
            color: theme::border(theme),
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    });

    // Full-screen dim layer so the panel floats above everything.
    iced::widget::container(panel)
        .center_x(iced::Length::Fill)
        .center_y(iced::Length::Fill)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_screen_builds() {
        let t = iced::Theme::Dark;
        let el = help_screen(&t);
        drop(el);
    }
}
