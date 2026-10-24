use crate::app::message::Message;
use crate::widgets::tag_input::FocusTrackTextInput;

/// Tag filter bar: an input with a dropdown of the current gallery's tags
/// (click to add) plus removable selected-tag labels.
///
/// The suggestion dropdown is shown only while the "Add tags" input is
/// focused (tracked via [`FocusTrackTextInput`]), and its options are laid
/// out as inline blocks that wrap — never one-per-line.
pub fn tag_filter<'a>(
    tags: &'a [String],
    input: &'a str,
    gallery_tags: &'a [String],
    dropdown_open: bool,
) -> iced::Element<'a, Message> {
    let input_field: iced::Element<'a, Message> = FocusTrackTextInput::new(
        crate::ui::text_input("Add tag...", input)
            .on_input(Message::TagInputChanged)
            .on_submit(Message::TagAdded(input.to_string()))
            .width(iced::Length::Fixed(220.0))
            .size(12),
    )
    .on_focus_changed(Message::TagFocusChanged)
    .into();

    // The dropdown's visibility is driven by app state (`tag_dropdown_open`),
    // which stays open through a candidate click's brief blur so the click
    // registers (closing on focus loss instantly hid the list mid-click).
    let needle = input.trim().to_lowercase();
    let mut options: Vec<iced::Element<'a, Message>> = Vec::new();
    if dropdown_open {
        for tag in gallery_tags {
            if tag.is_empty() || tags.contains(tag) {
                continue;
            }
            if needle.is_empty() || tag.to_lowercase().contains(&needle) {
                options.push(
                    crate::ui::button(iced::widget::text(tag).size(11))
                        .on_press(Message::TagAdded(tag.clone()))
                        .padding(iced::Padding::from([3, 10]))
                        .style(crate::ui::pill(None))
                        .into(),
                );
            }
            if options.len() >= 12 {
                break;
            }
        }
    }
    let dropdown: Option<iced::Element<'a, Message>> = if options.is_empty() {
        None
    } else {
        Some(
            iced::widget::container(crate::widgets::wrap::wrap(options).spacing(4.0))
                .width(iced::Length::Fill)
                .padding(iced::Padding::from([6, 6]))
                .style(crate::ui::card)
                .into(),
        )
    };

    // Selected tag labels with a remove button, wrapped across lines.
    let tag_labels: Vec<iced::Element<'a, Message>> = tags
        .iter()
        .map(|tag| {
            let remove_btn = crate::ui::button(iced::widget::text("✕").size(10))
                .on_press(Message::TagRemoved(tag.clone()))
                .padding(iced::Padding::from([0, 4]))
                .style(crate::ui::ghost);
            iced::widget::container(
                iced::widget::row![iced::widget::text(tag.clone()).size(11), remove_btn]
                    .spacing(2)
                    .align_y(iced::Alignment::Center),
            )
            .padding(iced::Padding {
                top: 2.0,
                right: 4.0,
                bottom: 2.0,
                left: 10.0,
            })
            .style(crate::ui::tag)
            .into()
        })
        .collect();

    // Only build rows that have content — no phantom empty space below the
    // textbox when there are no suggestions and no selected tags.
    let mut children: Vec<iced::Element<'a, Message>> = vec![input_field];
    if let Some(dropdown) = dropdown {
        children.push(dropdown);
    }
    if !tag_labels.is_empty() {
        children.push(crate::widgets::wrap::wrap(tag_labels).spacing(4.0).into());
    }

    iced::widget::Column::with_children(children)
        .spacing(4)
        .into()
}
