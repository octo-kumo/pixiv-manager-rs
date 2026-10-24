use crate::app::filter::{FilterMode, FilterState, SourceKey, Tristate};
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
    let query_mode = filter.mode == FilterMode::Query;

    // ── Search bar + advanced-toggle + clear ──
    let (placeholder, text) = if query_mode {
        (
            "tag  -excluded  ~either  user:name  views:>1000  is:r18  order:bookmarks",
            &filter.query_text,
        )
    } else {
        ("Search...", &filter.search_text)
    };
    let mut search = crate::ui::text_input(placeholder, text)
        .id(iced::widget::Id::new(SEARCH_INPUT_ID))
        .on_input(Message::SearchTextChanged)
        .size(12)
        .width(iced::Length::Fill);
    if is_search_tab {
        // Enter picks the highlighted suggestion, or runs the query.
        search = search.on_submit(Message::AutocompleteCommit);
    }

    let toggle = crate::ui::button(
        iced::widget::text(if query_mode {
            "? Syntax"
        } else {
            "⚙ Filters"
        })
        .size(12),
    )
    .on_press(Message::ToggleAdvancedFilters)
    .padding(iced::Padding::from([4, 10]));

    let clear = crate::ui::button(iced::widget::text("⌫ Clear filters").size(12))
        .on_press(Message::ResetFilters)
        .padding(iced::Padding::from([4, 10]));

    // Continuous "load all pages" button with progress (Search tab plus the
    // paginated Pixiv tabs) — sits inline with the filter buttons, not on
    // its own row.
    let auto_load_btn: Option<iced::Element<'a, Message>> = if show_auto_load {
        Some(if auto_load {
            crate::ui::button(
                iced::widget::row![
                    iced::widget::text("⏳").size(12),
                    iced::widget::text(format!("loading… {auto_load_loaded}")).size(11),
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
            )
            .on_press(Message::ToggleAutoLoad)
            .padding(iced::Padding::from([4, 10]))
            .style(crate::ui::primary)
            .into()
        } else {
            crate::ui::button(
                iced::widget::text(if auto_load_loaded > 0 {
                    format!("▶ Load all ({auto_load_loaded})")
                } else {
                    "▶ Load all".to_string()
                })
                .size(11),
            )
            .on_press(Message::ToggleAutoLoad)
            .padding(iced::Padding::from([4, 10]))
            .into()
        })
    } else {
        None
    };

    let mode_switch = iced::widget::row![
        mode_button("Classic", !query_mode, FilterMode::Classic),
        mode_button("Query", query_mode, FilterMode::Query),
    ]
    .spacing(2);
    let mut search_row_elems: Vec<iced::Element<'a, Message>> = vec![
        mode_switch.into(),
        search.into(),
        toggle.into(),
        clear.into(),
    ];
    if let Some(btn) = auto_load_btn {
        search_row_elems.push(btn);
    }
    let search_row = iced::widget::Row::with_children(search_row_elems)
        .spacing(4)
        .align_y(iced::Alignment::Center);

    let mut children: Vec<iced::Element<'a, Message>> = vec![search_row.into()];

    // Autocomplete tag dropdown (search tab only). The keyboard-highlighted
    // entry (↑/↓) gets the accent border.
    if query_mode {
        let query = crate::app::query::Query::parse(&filter.query_text);
        let suggestions = crate::app::query::suggest(&filter.query_text, gallery_tags);
        if !suggestions.is_empty() {
            let chips: Vec<iced::Element<'a, Message>> = suggestions
                .into_iter()
                .map(|s| {
                    crate::ui::button(iced::widget::text(s.clone()).size(11))
                        .on_press(Message::QuerySuggestionClicked(s))
                        .padding(iced::Padding::from([4, 10]))
                        .style(crate::ui::pill(None))
                        .into()
                })
                .collect();
            children.push(crate::widgets::wrap::wrap(chips).spacing(4.0).into());
        }
        if !query.tokens.is_empty() {
            let chips: Vec<iced::Element<'static, Message>> = query
                .tokens
                .iter()
                .enumerate()
                .map(|(i, token)| query_chip(i, token))
                .collect();
            children.push(crate::widgets::wrap::wrap(chips).spacing(4.0).into());
        }
        if show_advanced {
            children.push(query_help());
        }
        return iced::widget::Column::with_children(children)
            .spacing(4)
            .padding(iced::Padding::from([4, 8]))
            .into();
    }

    if is_search_tab && !autocomplete.is_empty() {
        let suggestions: Vec<iced::Element<'_, Message>> = autocomplete
            .iter()
            .take(10)
            .enumerate()
            .map(|(i, tag)| {
                let highlighted = autocomplete_selected == Some(i);
                let mut btn = crate::ui::button(iced::widget::text(tag).size(11))
                    .on_press(Message::AutocompleteTagClicked(tag.clone()))
                    .padding(iced::Padding::from([4, 10]));
                btn = btn.style(crate::ui::chip(highlighted));
                btn.into()
            })
            .collect();
        children.push(
            iced::widget::container(iced::widget::Column::with_children(suggestions).spacing(1))
                .width(iced::Length::Fill)
                .padding(iced::Padding::from([2, 4]))
                .style(crate::ui::card)
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
                crate::ui::button(iced::widget::text(*label).size(11))
                    .on_press(Message::ApiSortChanged(*value))
                    .padding(iced::Padding::from([4, 10]))
                    .style(crate::ui::chip(selected))
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
                crate::ui::button(iced::widget::text(*label).size(11))
                    .on_press(Message::ApiTargetChanged(*value))
                    .padding(iced::Padding::from([4, 10]))
                    .style(crate::ui::chip(selected))
                    .into(),
            );
        }

        children.push(
            iced::widget::row![
                iced::widget::Row::with_children(sort_btns).spacing(2),
                iced::widget::Space::new().width(12),
                iced::widget::Row::with_children(target_btns).spacing(2),
                iced::widget::Space::new().width(12),
                crate::ui::checkbox(prefs.translated)
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
                crate::ui::button(
                    iced::widget::row![
                        tristate::tristate_icon(st),
                        iced::widget::text(tool).size(10),
                    ]
                    .spacing(3)
                    .align_y(iced::Alignment::Center),
                )
                .on_press(Message::ToolFilterToggled(tool.clone()))
                .padding(iced::Padding::from([3, 8]))
                .style(crate::ui::chip(false))
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
    crate::ui::checkbox(checked)
        .label(label)
        .text_size(11.0)
        .size(14.0)
        .on_toggle(on_toggle)
        .into()
}

/// A filter button with its label inside and its tristate state shown as an
/// icon (✓ selected / ✗ excluded / ─ indeterminate). Colors derive from the
/// active theme so the foreground stays readable in dark and light modes.
fn option_btn(label: &str, state: Tristate, msg: Message) -> iced::Element<'_, Message> {
    crate::ui::button(
        iced::widget::row![
            tristate::tristate_icon(state),
            iced::widget::text(label).size(11),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center),
    )
    .on_press(msg)
    .padding(iced::Padding::from([4, 10]))
    .style(crate::ui::chip(state != Tristate::Indeterminate))
    .into()
}

/// A source chip of the Bookmarks tab's multi-select source filter: filled
/// (✓) when the source is shown, hollow when it is filtered out.
fn source_chip(label: &'static str, on: bool, key: SourceKey) -> iced::Element<'static, Message> {
    crate::ui::button(
        iced::widget::row![
            iced::widget::text(if on { "✓" } else { "✗" }).size(10),
            iced::widget::text(label).size(11),
        ]
        .spacing(3)
        .align_y(iced::Alignment::Center),
    )
    .on_press(Message::SourceFilterToggled(key))
    .padding(iced::Padding::from([3, 9]))
    .style(crate::ui::chip(on))
    .into()
}

/// One half of the Classic / Query switch.
fn mode_button(
    label: &'static str,
    selected: bool,
    mode: FilterMode,
) -> iced::Element<'static, Message> {
    crate::ui::button(iced::widget::text(label).size(11))
        .on_press(Message::FilterModeChanged(mode))
        .padding(iced::Padding::from([4, 12]))
        .style(move |theme, status| {
            if selected {
                crate::ui::primary(theme, status)
            } else {
                crate::ui::ghost(theme, status)
            }
        })
        .into()
}

/// A parsed query token as a chip; clicking it removes the token. Colors tell
/// the role at a glance: green include, red exclude, amber any-of, accent
/// sort order, red-filled for a token that could not be understood (the
/// tooltip says why).
fn query_chip(index: usize, token: &crate::app::query::Token) -> iced::Element<'static, Message> {
    use crate::app::query::Kind;
    let (color, invalid): (iced::Color, Option<String>) = match &token.kind {
        Kind::Include(_) => (theme::SUCCESS, None),
        Kind::Exclude(_) => (theme::DANGER, None),
        Kind::AnyOf(_) => (iced::Color::from_rgb(0.85, 0.6, 0.15), None),
        Kind::Order(..) => (iced::Color::from_rgb(0.45, 0.45, 0.9), None),
        Kind::Invalid(reason) => (theme::DANGER, Some(reason.clone())),
    };
    let chip = crate::ui::button(
        iced::widget::row![
            iced::widget::text(token.raw.clone()).size(11),
            iced::widget::text("×").size(11),
        ]
        .spacing(5),
    )
    .on_press(Message::QueryTokenRemoved(index))
    .padding(iced::Padding::from([3, 10]))
    .style(crate::ui::pill(Some(color)));
    match invalid {
        Some(reason) => iced::widget::tooltip(
            chip,
            iced::widget::container(iced::widget::text(reason).size(11))
                .padding(iced::Padding::from([3, 8]))
                .style(crate::ui::tooltip),
            iced::widget::tooltip::Position::Bottom,
        )
        .into(),
        None => chip.into(),
    }
}

/// The query syntax cheat sheet (shown by the "? Syntax" button).
fn query_help<'a>() -> iced::Element<'a, Message> {
    let line = |term: &'static str, what: &'static str| {
        iced::widget::row![
            iced::widget::text(term)
                .size(11)
                .font(iced::Font::MONOSPACE)
                .width(340),
            iced::widget::text(what).size(11),
        ]
        .spacing(8)
    };
    iced::widget::container(
        iced::widget::column![
            iced::widget::text("Every word must match. Click a chip to remove it.").size(11),
            line(
                "blue_sky",
                "has the tag exactly (_ = space, * = wildcard: blue*)"
            ),
            line("-boy", "does NOT have the tag"),
            line("~cat ~dog", "has at least one of the ~ tags"),
            line(
                "user:name  title:word  caption:word",
                "text contains (quote spaces: title:\"a b\")"
            ),
            line(
                "tool:clipstudio  source:pixiv|twitter|bilibili",
                "tool / site"
            ),
            line(
                "views:>1000  bookmarks:100..500  bpv:>=5%",
                "numbers: > >= < <= = a..b, 2k = 2000"
            ),
            line(
                "width:>=2k  height:  pixels:  pages:>1  ratio:>1",
                "size / pages / width÷height"
            ),
            line(
                "age:<7d  date:2024-05  date:>=2024-01-01",
                "posted within / on / after (h d w mo y)"
            ),
            line(
                "is:r18 -is:ai is:bookmarked is:landscape",
                "also portrait visible restricted missing downloaded multi"
            ),
            line(
                "order:bookmarks  order:date_asc",
                "sort (views bookmarks bpv date id pixels pages sanity; _asc)"
            ),
            line("id:>123456", "provider-local work id"),
        ]
        .spacing(2),
    )
    .padding(iced::Padding::from([6, 10]))
    .width(iced::Length::Fill)
    .style(crate::ui::card)
    .into()
}
