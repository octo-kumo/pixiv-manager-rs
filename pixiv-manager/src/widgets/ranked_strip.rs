//! The Feed tab's horizontal daily-ranking strip (Java's `HorizontalGallery` +
//! `RankedItem`): an infinitely-pleasant horizontal scroll of small tiles with
//! rank badges (#1 gold, #2 silver, #3 bronze).

use crate::app::message::Message;
use crate::theme;
use pixivj_rs::model::Illustration;
use std::collections::HashMap;

/// A horizontal strip of rank-badged tiles under a small "Today's ranking"
/// header. `thumbnails` are the shared thumbnail handles (falling back to a
/// placeholder like the main gallery). Renders nothing while empty.
pub fn ranked_strip<'a>(
    ranked: &'a [Illustration],
    thumbnails: &'a HashMap<u64, iced::widget::image::Handle>,
    // Only read while building; lifetime deliberately untied to the element.
    t: &iced::Theme,
) -> iced::Element<'a, Message> {
    if ranked.is_empty() {
        return iced::widget::text("").into();
    }

    let header = iced::widget::container(
        iced::widget::text("🏆 Today's ranking")
            .size(11)
            .color(theme::muted(t)),
    )
    .width(iced::Length::Fill)
    .padding([0, 4]);

    let mut tiles: Vec<iced::Element<'_, Message>> = Vec::new();
    for (i, illust) in ranked.iter().take(60).enumerate() {
        let rank = i + 1;
        let thumb: iced::Element<'_, Message> = match thumbnails.get(&illust.id) {
            Some(handle) => iced::widget::image(handle.clone())
                .width(96)
                .height(96)
                .into(),
            None => iced::widget::container(iced::widget::text(format!("#{}", illust.id)).size(9))
                .width(96)
                .height(96)
                .center_x(iced::Length::Fill)
                .center_y(iced::Length::Fill)
                .style(|theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(theme::panel(theme))),
                    ..Default::default()
                })
                .into(),
        };

        let badge_color = match rank {
            1 => [0.95, 0.8, 0.15], // gold
            2 => [0.75, 0.78, 0.8], // silver
            3 => [0.8, 0.5, 0.25],  // bronze
            _ => [0.35, 0.35, 0.35],
        };
        let badge = iced::widget::container(iced::widget::text(format!("#{rank}")).size(9))
            .padding(iced::Padding::from([1, 4]))
            .style(move |_| iced::widget::container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    badge_color[0],
                    badge_color[1],
                    badge_color[2],
                ))),
                border: iced::Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        let tile = iced::widget::stack![thumb, badge]
            .width(96)
            .height(96)
            .clip(true);

        tiles.push(
            iced::widget::button(tile)
                .on_press(Message::GalleryItemClicked(illust.id))
                .padding(iced::Padding::ZERO)
                .style(|theme, _| iced::widget::button::Style {
                    background: Some(iced::Background::Color(theme::panel(theme))),
                    ..Default::default()
                })
                .into(),
        );
    }

    iced::widget::container(
        iced::widget::column![
            header,
            iced::widget::scrollable(iced::widget::Row::with_children(tiles).spacing(4)).direction(
                iced::widget::scrollable::Direction::Horizontal(
                    iced::widget::scrollable::Scrollbar::default(),
                ),
            ),
        ]
        .spacing(2),
    )
    .width(iced::Length::Fill)
    .padding(iced::Padding::from([2, 4]))
    .into()
}
