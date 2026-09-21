use crate::app::filter::{FilterState, SourceKey, Tristate};
use crate::app::message::Message;
use crate::app::sort::SortState;
use crate::theme;
use crate::widgets::{sort_pane, tag_filter, tristate};

/// Stable id of the search text input (Ctrl+F focuses it).
pub const SEARCH_INPUT_ID: &str = "pixiv-manager-search-input";

/// Full filter + sort control bar shown above the gallery.
///
/// The search input is always visible, with a toggle on its right that
/// expands/collapses the advanced filter controls. On the Search tab the
/// input also submits a Pixiv search (Enter) and shows autocomplete tags.
/// The continuous-loading toggle ("Load all") shows on the Search tab and
/// the paginated Pixiv tabs (Recommended/Feed/Leaderboard).
#[allow(clippy::too_many_arguments)]
pub fn control_pane<'a>(
    filter: &'a FilterState,
    sort_state: &'a SortState,
    show_advanced: bool,
    is_search_tab: bool,
    is_bookmarks_tab: bool,
    show_auto_load: bool,
    auto_load: bool,
    auto_load_loaded: usize,
    autocomplete: &'a [String],
    gallery_tags: &'a [String],
    tag_dropdown_open: bool,
    gallery_tools: &'a [String],
    autocomplete_selected: Option<usize>,
    api_prefs: Option<&'a crate::app::state::SearchApiPrefs>,
) -> iced::Element<'a, Message> {
    // ── Search bar + advanced-toggle + clear ──
    let mut search = iced::widget::text_input("Search...", &filter.search_text)
        .id(iced::widget::Id::new(SEARCH_INPUT_ID))
        .on_input(Message::SearchTextChanged)
        .size(12)
        .width(iced::Length::Fill);
    if is_search_tab {
        // Enter picks the highlighted suggestion, or runs the query.
        search = search.on_submit(Message::AutocompleteCommit);
    }

    let toggle = iced::widget::button(iced::widget::text("⚙ Filters").size(12))
        .on_press(Message::ToggleAdvancedFilters)
        .padding(iced::Padding::from([2, 8]));

    let clear = iced::widget::button(iced::widget::text("⌫ Clear filters").size(12))
        .on_press(Message::ResetFilters)
        .padding(iced::Padding::from([2, 8]));

    let search_row = iced::widget::row![search, toggle, clear]
        .spacing(4)
        .align_y(iced::Alignment::Center);

    let mut children: Vec<iced::Element<'a, Message>> = vec![search_row.into()];

    // Continuous "load all pages" button with progress (Search tab plus the
    // paginated Pixiv tabs).
    if show_auto_load {
        let btn: iced::Element<'a, Message> = if auto_load {
            iced::widget::button(
                iced::widget::row![
                    iced::widget::text("⏳").size(12),
                    iced::widget::text(format!("loading… {auto_load_loaded}")).size(11),
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
            )
            .on_press(Message::ToggleAutoLoad)
            .padding(iced::Padding::from([2, 8]))
            .style(|theme, _| iced::widget::button::Style {
                background: Some(iced::Background::Color(theme::accent(theme))),
                text_color: theme::fg(theme),
                ..Default::default()
            })
            .into()
        } else {
            iced::widget::button(
                iced::widget::text(if auto_load_loaded > 0 {
                    format!("▶ Load all ({auto_load_loaded})")
                } else {
                    "▶ Load all".to_string()
                })
                .size(11),
            )
            .on_press(Message::ToggleAutoLoad)
            .padding(iced::Padding::from([2, 8]))
            .into()
        };
        children.push(btn);
    }

    // Autocomplete tag dropdown (search tab only). The keyboard-highlighted
    // entry (↑/↓) gets the accent border.
    if is_search_tab && !autocomplete.is_empty() {
        let suggestions: Vec<iced::Element<'_, Message>> = autocomplete
            .iter()
            .take(10)
            .enumerate()
            .map(|(i, tag)| {
                let highlighted = autocomplete_selected == Some(i);
                let mut btn = iced::widget::button(iced::widget::text(tag).size(11))
                    .on_press(Message::AutocompleteTagClicked(tag.clone()))
                    .padding(iced::Padding::from([2, 8]));
                if highlighted {
                    btn = btn.style(|theme, _| iced::widget::button::Style {
                        background: Some(iced::Background::Color(theme::panel(theme))),
                        text_color: theme::fg(theme),
                        border: iced::Border {
                            color: theme::accent(theme),
                            width: 1.5,
                            radius: 4.0.into(),
                        },
                        ..Default::default()
                    });
                }
                btn.into()
            })
            .collect();
        children.push(
            iced::widget::container(iced::widget::Column::with_children(suggestions).spacing(1))
                .width(iced::Length::Fill)
                .padding(iced::Padding::from([2, 4]))
                .style(|theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(theme::panel(theme))),
                    border: iced::Border {
                        radius: 4.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into(),
        );
    }

    if !show_advanced {
        return iced::widget::Column::with_children(children)
            .padding(iced::Padding::from([4, 8]))
            .into();
    }

    // ── Option filter row (7 tristate buttons) ──
    let option_buttons = iced::widget::row![
        option_btn(
            "R-18",
            filter.options.r18,
            Message::OptionFilterToggled(crate::app::filter::OptionFilterKey::R18)
        ),
        option_btn(
            "AI",
            filter.options.ai,
            Message::OptionFilterToggled(crate::app::filter::OptionFilterKey::Ai)
        ),
        option_btn(
            "Bookmarked",
            filter.options.bookmarked,
            Message::OptionFilterToggled(crate::app::filter::OptionFilterKey::Bookmarked)
        ),
        option_btn(
            "Landscape",
            filter.options.landscape,
            Message::OptionFilterToggled(crate::app::filter::OptionFilterKey::Landscape)
        ),
        option_btn(
            "Visible",
            filter.options.visible,
            Message::OptionFilterToggled(crate::app::filter::OptionFilterKey::Visible)
        ),
        option_btn(
            "Restricted",
            filter.options.restricted,
            Message::OptionFilterToggled(crate::app::filter::OptionFilterKey::Restricted)
        ),
        option_btn(
            "Missing",
            filter.options.missing,
            Message::OptionFilterToggled(crate::app::filter::OptionFilterKey::Missing)
        ),
    ]
    .spacing(2);

    // ── Source filter (Bookmarks tab: mixed Pixiv/Twitter/Bilibili) ──
    if is_bookmarks_tab {
        let sources = filter.sources;
        children.push(
            iced::widget::row![
                iced::widget::text("Sources:").size(11),
                source_chip("Pixiv", sources.pixiv, SourceKey::Pixiv),
                source_chip("Twitter", sources.twitter, SourceKey::Twitter),
                source_chip("Bilibili", sources.bilibili, SourceKey::Bilibili),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center)
            .into(),
        );
    }

    // ── Tag filter (dropdown of the current gallery's tags) ──
    let tags = tag_filter::tag_filter(
        &filter.selected_tags,
        &filter.tag_input,
        gallery_tags,
        tag_dropdown_open,
    );

    // ── Search targets: where the search box looks ──
    let search_targets = iced::widget::row![
        iced::widget::text("Search in:").size(11),
        checkbox("Tags", filter.search_tags, |on| {
            Message::SearchTargetChanged(crate::app::filter::SearchTargetKey::Tags, on)
        }),
        checkbox("Author", filter.search_author, |on| {
            Message::SearchTargetChanged(crate::app::filter::SearchTargetKey::Author, on)
        }),
        checkbox("Title", filter.search_title, |on| {
            Message::SearchTargetChanged(crate::app::filter::SearchTargetKey::Title, on)
        }),
        checkbox("Caption", filter.search_caption, |on| {
            Message::SearchTargetChanged(crate::app::filter::SearchTargetKey::Caption, on)
        }),
        checkbox("ID", filter.search_id, |on| {
            Message::SearchTargetChanged(crate::app::filter::SearchTargetKey::Id, on)
        }),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    // ── Server-side Pixiv search options (Search tab only) ──
    if let Some(prefs) = api_prefs {
        let sort_options: &[(&str, Option<pixivj_rs::model::SortOption>)] = &[
            ("Newest", None),
            ("Oldest", Some(pixivj_rs::model::SortOption::DateAsc)),
        ];
        let mut sort_btns: Vec<iced::Element<'a, Message>> =
            vec![iced::widget::text("Order:").size(11).into()];
        for (label, value) in sort_options {
            let selected = prefs.sort == *value;
            sort_btns.push(
                iced::widget::button(iced::widget::text(*label).size(11))
                    .on_press(Message::ApiSortChanged(*value))
                    .padding(iced::Padding::from([2, 8]))
                    .style(move |theme, _| pick_style(theme, selected))
                    .into(),
            );
        }

        let target_options: &[(&str, Option<pixivj_rs::model::SearchTarget>)] = &[
            ("Any", None),
            (
                "Tags~",
                Some(pixivj_rs::model::SearchTarget::PartialMatchForTags),
            ),
            (
                "Tags=",
                Some(pixivj_rs::model::SearchTarget::ExactMatchForTags),
            ),
            (
                "Tags+kw",
                Some(pixivj_rs::model::SearchTarget::KeywordAndTags),
            ),
            ("Text", Some(pixivj_rs::model::SearchTarget::Text)),
        ];
        let mut target_btns: Vec<iced::Element<'a, Message>> =
            vec![iced::widget::text("Match:").size(11).into()];
        for (label, value) in target_options {
            let selected = prefs.target == *value;
            target_btns.push(
                iced::widget::button(iced::widget::text(*label).size(11))
                    .on_press(Message::ApiTargetChanged(*value))
                    .padding(iced::Padding::from([2, 8]))
                    .style(move |theme, _| pick_style(theme, selected))
                    .into(),
            );
        }

        children.push(
            iced::widget::row![
                iced::widget::Row::with_children(sort_btns).spacing(2),
                iced::widget::Space::new().width(12),
                iced::widget::Row::with_children(target_btns).spacing(2),
                iced::widget::Space::new().width(12),
                iced::widget::checkbox(prefs.translated)
                    .label("translated tags")
                    .text_size(11)
                    .size(14)
                    .on_toggle(Message::ApiTranslatedChanged),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center)
            .into(),
        );
    }

    // ── Tool filter (tristate per tool found in the gallery) ──
    let tools: iced::Element<'a, Message> = if gallery_tools.is_empty() {
        iced::widget::text("").into()
    } else {
        let mut tool_buttons: Vec<iced::Element<'_, Message>> = Vec::new();
        for tool in gallery_tools {
            let st = filter
                .tool_states
                .get(tool)
                .copied()
                .unwrap_or(Tristate::Indeterminate);
            tool_buttons.push(
                iced::widget::button(
                    iced::widget::row![
                        tristate::tristate_icon(st),
                        iced::widget::text(tool).size(10),
                    ]
                    .spacing(3)
                    .align_y(iced::Alignment::Center),
                )
                .on_press(Message::ToolFilterToggled(tool.clone()))
                .padding(iced::Padding::from([2, 6]))
                .style(|theme, _| iced::widget::button::Style {
                    background: Some(iced::Background::Color(theme::panel(theme))),
                    text_color: theme::fg(theme),
                    border: iced::Border {
                        color: theme::border(theme),
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..Default::default()
                })
                .into(),
            );
        }
        iced::widget::row![
            iced::widget::text("Tools:").size(11),
            crate::widgets::wrap::wrap(tool_buttons).spacing(2.0),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center)
        .into()
    };

    // ── Sort pane ──
    let sort = sort_pane::sort_pane(sort_state);

    children.push(option_buttons.into());
    children.push(tags);
    children.push(search_targets.into());
    children.push(tools);
    children.push(sort);

    iced::widget::Column::with_children(children)
        .spacing(4)
        .padding(iced::Padding::from([4, 8]))
        .into()
}

/// The field a search-target toggle affects.
fn checkbox<'a>(
    label: &'a str,
    checked: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> iced::Element<'a, Message> {
    iced::widget::checkbox(checked)
        .label(label)
        .text_size(11.0)
        .size(14.0)
        .on_toggle(on_toggle)
        .into()
}

/// Button style for a selected vs unselected choice chip (used by the
/// server-side sort/match pickers).
fn pick_style(theme: &iced::Theme, selected: bool) -> iced::widget::button::Style {
    if selected {
        iced::widget::button::Style {
            background: Some(iced::Background::Color(theme::panel(theme))),
            text_color: theme::fg(theme),
            border: iced::Border {
                color: theme::accent(theme),
                width: 1.5,
                radius: 4.0.into(),
            },
            ..Default::default()
        }
    } else {
        iced::widget::button::Style {
            background: Some(iced::Background::Color(
                theme::panel(theme).scale_alpha(0.45),
            )),
            text_color: theme::muted(theme),
            ..Default::default()
        }
    }
}

/// A filter button with its label inside and its tristate state shown as an
/// icon (✓ selected / ✗ excluded / ─ indeterminate). Colors derive from the
/// active theme so the foreground stays readable in dark and light modes.
fn option_btn(label: &str, state: Tristate, msg: Message) -> iced::Element<'_, Message> {
    iced::widget::button(
        iced::widget::row![
            tristate::tristate_icon(state),
            iced::widget::text(label).size(11),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center),
    )
    .on_press(msg)
    .padding(iced::Padding::from([3, 8]))
    .style(|theme, _| iced::widget::button::Style {
        background: Some(iced::Background::Color(theme::panel(theme))),
        text_color: theme::fg(theme),
        border: iced::Border {
            color: theme::border(theme),
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// A source chip of the Bookmarks tab's multi-select source filter: filled
/// (✓) when the source is shown, hollow when it is filtered out.
fn source_chip(label: &'static str, on: bool, key: SourceKey) -> iced::Element<'static, Message> {
    iced::widget::button(
        iced::widget::row![
            iced::widget::text(if on { "✓" } else { "✗" }).size(10),
            iced::widget::text(label).size(11),
        ]
        .spacing(3)
        .align_y(iced::Alignment::Center),
    )
    .on_press(Message::SourceFilterToggled(key))
    .padding(iced::Padding::from([2, 7]))
    .style(move |theme, _| {
        if on {
            iced::widget::button::Style {
                background: Some(iced::Background::Color(theme::panel(theme))),
                text_color: theme::fg(theme),
                border: iced::Border {
                    color: theme::accent(theme),
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
            }
        } else {
            iced::widget::button::Style {
                background: Some(iced::Background::Color(
                    theme::panel(theme).scale_alpha(0.45),
                )),
                text_color: theme::muted(theme),
                ..Default::default()
            }
        }
    })
    .into()
}
