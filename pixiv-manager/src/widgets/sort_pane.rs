use crate::app::message::Message;
use crate::app::sort::{SortDirection, SortKey, SortState};
use crate::theme;

pub fn sort_pane(sort_state: &SortState) -> iced::Element<'_, Message> {
    // In multi-sort mode the active rules are shown in selection order (rule 1
    // is the primary key); otherwise a single "Sort ↓/↑" label.
    let multi_btn = crate::ui::button(iced::widget::text("Multi").size(12))
        .on_press(Message::MultiSortToggled)
        .padding(iced::Padding::from([3, 10]))
        .style(crate::ui::chip(sort_state.multi_sort));

    let dir_text = if sort_state.multi_sort {
        let mut labels: Vec<String> = Vec::new();
        for (i, (key, dir)) in sort_state.active.iter().enumerate() {
            let (icon, _) = sort_direction_icon_label(*dir);
            labels.push(format!("{}. {} {}", i + 1, sort_label(*key), icon));
        }
        iced::widget::text(if labels.is_empty() {
            "Multi sort: none".to_string()
        } else {
            labels.join("  ")
        })
        .size(12)
    } else {
        let (icon, _color) = sort_direction_display(sort_state);
        iced::widget::text(format!("Sort {icon}")).size(12)
    };

    let sort_buttons = [
        (SortKey::Views, "Views"),
        (SortKey::Bookmarks, "Bookmarks"),
        (SortKey::BookmarksPerView, "BpV"),
        (SortKey::Date, "Date"),
        (SortKey::Id, "ID"),
        (SortKey::Sanity, "Sanity"),
        (SortKey::Pixel, "Pixels"),
        (SortKey::Pages, "Pages"),
        (SortKey::Srgb, "sRGB"),
        (SortKey::Hue, "Hue"),
    ];

    let mut row = iced::widget::Row::new().spacing(3);

    for (key, label) in &sort_buttons {
        let active_dir = sort_state
            .active
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, d)| *d)
            .unwrap_or(SortDirection::Undefined);

        let (icon, _color) = sort_direction_icon_label(active_dir);

        let btn = crate::ui::button(iced::widget::text(format!("{label} {icon}")).size(10))
            .padding(iced::Padding::from([3, 8]))
            .style(crate::ui::chip(active_dir != SortDirection::Undefined));

        row = row.push(btn.on_press(Message::SortKeyClicked(*key)));
    }

    // Color-sort extras: a swatch picker + level selector, shown only while an
    // sRGB/Hue key is active (the user must pick a target color).
    let has_color_sort = sort_state
        .active
        .iter()
        .any(|(k, _)| matches!(k, SortKey::Srgb | SortKey::Hue));
    let mut children: Vec<iced::Element<'_, Message>> = vec![
        iced::widget::row![multi_btn, dir_text].spacing(4).into(),
        row.into(),
    ];
    if has_color_sort {
        let mut swatches: Vec<iced::Element<'_, Message>> = Vec::new();
        for color in PRESET_COLORS {
            let selected =
                sort_state.custom_color_text.is_none() && sort_state.color == Some(*color);
            let color = *color;
            let swatch = crate::ui::button(iced::widget::text("").width(18).height(18))
                .on_press(Message::ColorSortChanged(color))
                .padding(iced::Padding::ZERO)
                .style(move |theme, _| iced::widget::button::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgb(
                        color[0] as f32 / 255.0,
                        color[1] as f32 / 255.0,
                        color[2] as f32 / 255.0,
                    ))),
                    border: iced::Border {
                        color: if selected {
                            theme::fg(theme)
                        } else {
                            theme::border(theme)
                        },
                        width: if selected { 2.0 } else { 1.0 },
                        radius: 9.0.into(),
                    },
                    ..Default::default()
                });
            swatches.push(swatch.into());
        }
        // Custom color: looks like a preset swatch, tinted by the current
        // color; selecting it reveals a text input (red outline = unparsable).
        let custom_selected = sort_state.custom_color_text.is_some();
        let custom_rgb = sort_state.color.unwrap_or([128, 128, 128]);
        let custom_bg = iced::Color::from_rgb8(custom_rgb[0], custom_rgb[1], custom_rgb[2]);
        // Pick a readable icon color against the swatch.
        let luma = 0.299 * custom_bg.r + 0.587 * custom_bg.g + 0.114 * custom_bg.b;
        let icon_color = if luma > 0.55 {
            iced::Color::BLACK
        } else {
            iced::Color::WHITE
        };
        swatches.push(
            crate::ui::button(
                iced::widget::text("✎")
                    .size(12)
                    .color(icon_color)
                    .width(18)
                    .height(18)
                    .align_x(iced::Alignment::Center)
                    .align_y(iced::Alignment::Center),
            )
            .on_press(Message::CustomColorSelected)
            .padding(iced::Padding::ZERO)
            .style(move |theme, _| iced::widget::button::Style {
                background: Some(iced::Background::Color(custom_bg)),
                border: iced::Border {
                    color: if custom_selected {
                        theme::fg(theme)
                    } else {
                        theme::border(theme)
                    },
                    width: if custom_selected { 2.0 } else { 1.0 },
                    radius: 9.0.into(),
                },
                ..Default::default()
            })
            .into(),
        );
        if let Some(text) = &sort_state.custom_color_text {
            let invalid = crate::app::sort::parse_color(text).is_none();
            swatches.push(
                crate::ui::text_input("#rrggbb, rgb(…)", text)
                    .on_input(Message::CustomColorTextChanged)
                    .size(11)
                    .padding(iced::Padding::from([2, 6]))
                    .width(130)
                    .style(move |theme, status| {
                        let mut style = crate::ui::input(theme, status);
                        if invalid {
                            style.border.color = theme::DANGER;
                            style.border.width = 1.5;
                        }
                        style
                    })
                    .into(),
            );
        }
        let level = sort_state.color_level.clamp(1, 5);
        let level_row = iced::widget::row![
            iced::widget::text("Level").size(11),
            crate::ui::button(iced::widget::text("−").size(13))
                .on_press_maybe((level > 1).then_some(Message::ColorLevelChanged(level - 1)))
                .padding(iced::Padding::from([1, 6])),
            iced::widget::text(format!("{}", level)).size(11).width(16),
            crate::ui::button(iced::widget::text("+").size(13))
                .on_press_maybe((level < 5).then_some(Message::ColorLevelChanged(level + 1)))
                .padding(iced::Padding::from([1, 6])),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center);
        children.push(
            iced::widget::row![
                iced::widget::text("Color").size(11),
                iced::widget::Row::with_children(swatches).spacing(3),
                iced::widget::Space::new().width(iced::Length::Fill),
                level_row,
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .into(),
        );
    }

    iced::widget::Column::with_children(children)
        .spacing(4)
        .into()
}

/// A small palette of common colors for the color sort.
const PRESET_COLORS: &[[u8; 3]] = &[
    [255, 0, 0],
    [255, 128, 0],
    [255, 255, 0],
    [0, 255, 0],
    [0, 200, 200],
    [0, 0, 255],
    [150, 0, 255],
    [255, 0, 200],
    [0, 0, 0],
    [255, 255, 255],
];

fn sort_label(key: SortKey) -> &'static str {
    match key {
        SortKey::Views => "Views",
        SortKey::Bookmarks => "Bookmarks",
        SortKey::BookmarksPerView => "BpV",
        SortKey::Date => "Date",
        SortKey::Id => "ID",
        SortKey::Sanity => "Sanity",
        SortKey::Pixel => "Pixels",
        SortKey::Pages => "Pages",
        SortKey::Srgb => "Srgb",
        SortKey::Hue => "Hue",
    }
}

fn sort_direction_icon_label(dir: SortDirection) -> (&'static str, iced::Color) {
    match dir {
        SortDirection::Down => ("↓", crate::theme::SUCCESS),
        SortDirection::Up => ("↑", crate::theme::DANGER),
        SortDirection::Undefined => ("↕", crate::theme::NEUTRAL),
    }
}

fn sort_direction_display(sort: &SortState) -> (&'static str, iced::Color) {
    let dir = sort
        .active
        .first()
        .map(|(_, d)| *d)
        .unwrap_or(SortDirection::Undefined);
    sort_direction_icon_label(dir)
}
