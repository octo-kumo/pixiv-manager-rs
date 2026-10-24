use crate::app::filter::Tristate;
use crate::app::message::Message;
use crate::theme;

/// Shared glyph+color for a tristate state: ✓ selected, ✗ excluded, ─ off.
fn tristate_glyph(state: Tristate) -> (&'static str, iced::Color) {
    match state {
        Tristate::Selected => ("✓", theme::SUCCESS),
        Tristate::Deselected => ("✗", theme::DANGER),
        Tristate::Indeterminate => ("─", theme::NEUTRAL),
    }
}

/// The icon glyph for a tristate state.
pub fn tristate_icon(state: Tristate) -> iced::Element<'static, Message> {
    let (label, color) = tristate_glyph(state);
    iced::widget::text(label).size(13).color(color).into()
}

/// Renders a three-state checkbox as a button.
/// - Indeterminate: dash `─`
/// - Excluded: empty ` `
pub fn tristate_checkbox(state: Tristate, on_click: Message) -> iced::Element<'static, Message> {
    let (label, color) = tristate_glyph(state);

    crate::ui::button(iced::widget::text(label).size(14).color(color))
        .on_press(on_click)
        .padding(iced::Padding::from([2, 6]))
        .style(|theme, _| iced::widget::button::Style {
            background: Some(iced::Background::Color(theme::panel(theme))),
            border: iced::Border {
                color: theme::border(theme),
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        })
        .into()
}
