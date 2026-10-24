use crate::app::message::Message;
use crate::theme;
use pixivj_rs::model::Illustration;

/// Detailed illustration info shown inside the viewer's right panel.
/// Shows: title, author (+follow), clickable ID, bookmark toggle, download
/// button, copy-URL / open-folder actions, creation date, dimensions,
/// caption, clickable tag labels, tool labels.
#[allow(clippy::too_many_arguments)]
pub fn illust_detail<'a>(
    illust: &'a Illustration,
    author_pfp: Option<&'a iced::widget::image::Handle>,
    download: iced::Element<'a, Message>,
    on_disk: bool,
    // Only read while building (colors are copied out), so its lifetime is
    // deliberately NOT tied to the returned element.
    t: &iced::Theme,
) -> iced::Element<'a, Message> {
    let provider = crate::media::provider_of(illust.id);
    let direct = provider != crate::media::Provider::Pixiv;
    let mut rows: Vec<iced::Element<'a, Message>> = Vec::new();

    // Title
    if let Some(title) = &illust.title {
        if !title.is_empty() {
            rows.push(
                iced::widget::text(title)
                    .size(15)
                    .width(iced::Length::Fill)
                    .into(),
            );
        }
    }

    // Author (clickable → artist page) with pfp + a follow toggle below it.
    // Tweet authors open on x.com instead (their id is Twitter-namespaced)
    // and have no Pixiv follow action.
    if let Some(user) = &illust.user {
        let mut author_row_children: Vec<iced::Element<'a, Message>> = Vec::new();
        if let Some(handle) = author_pfp {
            author_row_children.push(
                iced::widget::container(crate::widgets::cover_image::cover_image(
                    handle.clone(),
                    20.0,
                ))
                .width(20)
                .height(20)
                .clip(true)
                .into(),
            );
        }
        if direct {
            let label = match &user.account {
                Some(account) => format!("{} (@{})", user.name, account),
                None => user.name.clone(),
            };
            author_row_children.push(iced::widget::text(label).size(12).into());
        } else {
            author_row_children.push(
                crate::ui::button(iced::widget::text(user.name.clone()).size(12))
                    .on_press(Message::ArtistOpened(user.id))
                    .padding(iced::Padding::from([1, 2]))
                    .style(iced::widget::button::text)
                    .into(),
            );
        }
        rows.push(
            iced::widget::Row::with_children(author_row_children)
                .spacing(6)
                .align_y(iced::Alignment::Center)
                .into(),
        );
        if !direct {
            rows.push(
                crate::ui::button(
                    iced::widget::text(if user.followed() {
                        "☑ Following"
                    } else {
                        "☐ Follow"
                    })
                    .size(12),
                )
                .on_press(Message::FollowToggled(user.id))
                .padding(iced::Padding::from([3, 10]))
                .style(crate::ui::chip(user.followed()))
                .into(),
            );
        }
    }

    // Clickable ID (opens in browser: pixiv artwork page or the tweet).
    rows.push(
        crate::ui::button(
            iced::widget::text(format!("#{}", crate::media::native_id(illust.id)))
                .size(11)
                .color(theme::accent(t)),
        )
        .on_press(Message::OpenInBrowser(illust.id))
        .padding(iced::Padding::from([1, 2]))
        .style(iced::widget::button::text)
        .into(),
    );

    // Action row: bookmark/like (with count) + download + copy URL + folder.
    let bookmarked = illust.bookmarked();
    let count = illust.total_bookmarks.unwrap_or(0);
    let (bookmark_glyph, bookmark_tip) = if direct {
        (if bookmarked { "♥" } else { "♡" }, "Like")
    } else {
        (if bookmarked { "★" } else { "☆" }, "Bookmark")
    };
    let bookmark_label = format!("{} {}", bookmark_glyph, count);
    let copy_tip = provider.copy_url_tip();
    let mut action_row = iced::widget::row![
        iced::widget::tooltip(
            crate::ui::button(iced::widget::text(bookmark_label).size(12))
                .on_press(Message::BookmarkToggled(illust.id))
                .width(90)
                .height(26)
                .style(crate::ui::chip(bookmarked)),
            bookmark_tip,
            iced::widget::tooltip::Position::Top,
        )
        .style(crate::ui::tooltip)
        .padding(4),
        download,
        icon_action(
            "⧉",
            copy_tip,
            Some(Message::CopyIllustUrl(illust.id)),
            false
        ),
    ]
    .spacing(4)
    .align_y(iced::Alignment::Center);
    // Open-folder action: always visible; a no-op (dimmed) until the work
    // exists on disk. ("▤" over the emoji folder glyph, which some system
    // fonts don't carry.)
    action_row = action_row.push(icon_action(
        "▤",
        "Open download folder",
        on_disk.then_some(Message::RevealIllustFiles(illust.id)),
        !on_disk,
    ));

    rows.push(action_row.into());

    // Creation date
    if let Some(date) = illust.create_date {
        rows.push(
            iced::widget::text(format!("Created: {}", date.format("%Y-%m-%d %H:%M")))
                .size(11)
                .color(theme::muted(t))
                .into(),
        );
    }

    // Dimensions
    if let (Some(w), Some(h)) = (illust.width, illust.height) {
        rows.push(
            iced::widget::text(format!("{}×{} px", w, h))
                .size(11)
                .color(theme::muted(t))
                .into(),
        );
    }

    // Caption (Pixiv captions are HTML-ish; render a readable plain-text
    // preview. Tweet text is already plain.)
    if let Some(caption) = &illust.caption {
        let plain = if direct {
            // Don't repeat the title when the post is a single line.
            if illust.title.as_deref() == Some(caption.as_str()) {
                String::new()
            } else {
                truncate(caption, 280)
            }
        } else {
            plain_caption(caption, 220)
        };
        if !plain.is_empty() {
            rows.push(
                iced::widget::text(plain)
                    .size(11)
                    .color(theme::muted(t))
                    .into(),
            );
        }
    }

    // Tags (wrap to multiple lines; a plain Row would push the last tag
    // onto its own line). On Pixiv each chip searches that tag; tweets show
    // static hashtag chips (Pixiv search wouldn't find them).
    if !illust.tags.is_empty() {
        let tag_labels: Vec<iced::Element<'_, Message>> = illust
            .tags
            .iter()
            .filter_map(|t| {
                if t.name.is_empty() {
                    None
                } else if direct {
                    Some(
                        iced::widget::container(
                            iced::widget::text(format!("#{}", t.name)).size(10),
                        )
                        .padding(iced::Padding::from([2, 8]))
                        .style(crate::ui::tag)
                        .into(),
                    )
                } else {
                    Some(
                        crate::ui::button(iced::widget::text(&t.name).size(10))
                            .on_press(Message::TagSearchRequested(t.name.clone()))
                            .padding(iced::Padding::from([2, 8]))
                            .style(crate::ui::pill(None))
                            .into(),
                    )
                }
            })
            .collect();

        if !tag_labels.is_empty() {
            rows.push(crate::widgets::wrap::wrap(tag_labels).spacing(4.0).into());
        }
    }

    // Tools
    if !illust.tools.is_empty() {
        let tool_labels: Vec<iced::Element<'_, Message>> = illust
            .tools
            .iter()
            .map(|tool| {
                iced::widget::container(iced::widget::text(tool).size(10))
                    .padding(iced::Padding::from([2, 8]))
                    .style(crate::ui::tag)
                    .into()
            })
            .collect();

        if !tool_labels.is_empty() {
            rows.push(
                iced::widget::row![
                    iced::widget::text("Tools:").size(11),
                    crate::widgets::wrap::wrap(tool_labels).spacing(4.0),
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center)
                .into(),
            );
        }
    }

    iced::widget::container(
        iced::widget::Column::with_children(rows)
            .spacing(4)
            .padding(8),
    )
    .width(iced::Length::Fill)
    .style(|theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(theme::panel(theme))),
        ..Default::default()
    })
    .into()
}

/// A compact icon-only action button (used for copy-URL / open-folder).
/// The glyph is centered by shrinking the button to its content with equal
/// horizontal padding on both sides (the fixed asymmetric padding made the
/// copy icon sit off-center, and the fixed width left dead space on the
/// download button's right).
fn icon_action(
    glyph: &'static str,
    tooltip: &'static str,
    msg: Option<Message>,
    _disabled: bool,
) -> iced::Element<'static, Message> {
    let mut btn = crate::ui::button(
        iced::widget::container(iced::widget::text(glyph).size(13))
            .center_x(iced::Length::Shrink)
            .center_y(iced::Length::Shrink),
    )
    .height(26)
    .width(iced::Length::Shrink)
    .padding(iced::Padding::from([2, 7]));
    if let Some(msg) = msg {
        btn = btn.on_press(msg);
    }
    iced::widget::tooltip(btn, tooltip, iced::widget::tooltip::Position::Top)
        .style(crate::ui::tooltip)
        .padding(4)
        .into()
}

/// Cap text at `max` chars, appending an ellipsis when truncated.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max).collect::<String>())
    } else {
        text.to_string()
    }
}

/// Reduce a Pixiv HTML caption to readable plain text: line breaks are kept,
/// all other tags are dropped, and the result is capped at `max` chars.
fn plain_caption(html: &str, max: usize) -> String {
    let with_breaks = html
        .replace("<br />", "\n")
        .replace("<br/>", "\n")
        .replace("<br>", "\n");
    let mut out = String::with_capacity(with_breaks.len());
    let mut in_tag = false;
    for ch in with_breaks.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let collapsed: String = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if collapsed.chars().count() > max {
        format!("{}…", collapsed.chars().take(max).collect::<String>())
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixivj_rs::model::{Author, Illustration};

    fn make_test_illust() -> Illustration {
        Illustration {
            id: 12345,
            title: Some("Test".into()),
            user: Some(Author {
                id: 999,
                name: "TestArtist".into(),
                account: Some("testartist".into()),
                profile_image_urls: None,
                comment: None,
                is_followed: Some(false),
            }),
            tags: vec![pixivj_rs::model::Tag {
                name: "landscape".into(),
                translated_name: None,
            }],
            tools: vec!["CLIP STUDIO PAINT".into()],
            width: Some(1920),
            height: Some(1080),
            create_date: Some(
                chrono::DateTime::parse_from_rfc3339("2024-01-15T12:00:00+00:00")
                    .unwrap()
                    .into(),
            ),
            caption: None,
            illust_ai_type: None,
            illust_type: None,
            image_urls: None,
            restrict: None,
            page_count: None,
            sanity_level: None,
            x_restrict: None,
            series: None,
            meta_single_page: None,
            meta_pages: vec![],
            total_view: None,
            total_bookmarks: None,
            is_bookmarked: None,
            visible: None,
            is_muted: None,
            provider_data: None,
        }
    }

    #[test]
    fn test_illust_detail_renders() {
        let illust = make_test_illust();
        let t = iced::Theme::Dark;
        let el = illust_detail(&illust, None, iced::widget::text("").into(), false, &t);
        drop(el);
    }

    #[test]
    fn plain_caption_strips_tags_and_caps_length() {
        let html = "Line one<br />Line <b>two</b> &amp; more<br/><br />tail";
        let out = plain_caption(html, 200);
        assert_eq!(out, "Line one\nLine two &amp; more\ntail");
        let long = "x".repeat(500);
        assert_eq!(plain_caption(&long, 10).chars().count(), 11); // cap + ellipsis
    }
}
