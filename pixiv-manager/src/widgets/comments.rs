use crate::app::message::Message;
use crate::theme;
use crate::widgets::illust_info::relative_date;
use pixivj_rs::model::Comment;
use std::collections::HashMap;

/// Scrollable list of comments filling the right panel's bottom half.
/// Rows alternate between two very close backgrounds; comment authors' profile
/// pictures are shown when loaded. Scrolling near the bottom emits
/// [`Message::CommentsLoadMore`] so older comments paginate in (a
/// "loading older…" footer shows while a page is in flight).
#[allow(clippy::too_many_arguments)]
pub fn comments<'a>(
    comments: &'a [Comment],
    pfps: &'a HashMap<u64, iced::widget::image::Handle>,
    stamps: &'a HashMap<u64, iced::widget::image::Handle>,
    loading_older: bool,
    // Only read while building; lifetime deliberately untied to the element.
    t: &iced::Theme,
) -> iced::Element<'a, Message> {
    comments_with_label(comments, pfps, stamps, loading_older, t, None)
}

/// Like [`comments`], but with a provider-specific empty-state label
/// ("No replies yet" for tweets).
pub fn comments_with_label<'a>(
    comments: &'a [Comment],
    pfps: &'a HashMap<u64, iced::widget::image::Handle>,
    stamps: &'a HashMap<u64, iced::widget::image::Handle>,
    loading_older: bool,
    t: &iced::Theme,
    empty_label: Option<&'static str>,
) -> iced::Element<'a, Message> {
    if comments.is_empty() {
        return iced::widget::container(
            iced::widget::text(empty_label.unwrap_or("No comments yet")).size(12),
        )
        .center_x(iced::Length::Fill)
        .center_y(iced::Length::Fill)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into();
    }

    let mut comment_elems: Vec<iced::Element<'_, Message>> = Vec::new();
    for (i, comment) in comments.iter().enumerate() {
        comment_elems.push(comment_row(comment, pfps, stamps, i, t));
    }
    if loading_older {
        comment_elems.push(
            iced::widget::container(
                iced::widget::text("Loading older…")
                    .size(10)
                    .color(theme::muted(t)),
            )
            .width(iced::Length::Fill)
            .padding([6, 0])
            .into(),
        );
    }

    let list = iced::widget::Column::with_children(comment_elems)
        .spacing(0)
        .padding(0);

    iced::widget::container(crate::ui::scrollable(list).on_scroll(
        |viewport: iced::widget::scrollable::Viewport| {
            if viewport.relative_offset().y > 0.85 {
                Message::CommentsLoadMore
            } else {
                Message::Noop
            }
        },
    ))
    .width(iced::Length::Fill)
    .height(iced::Length::Fill)
    .into()
}

fn comment_row<'a>(
    comment: &'a Comment,
    pfps: &'a HashMap<u64, iced::widget::image::Handle>,
    stamps: &'a HashMap<u64, iced::widget::image::Handle>,
    index: usize,
    t: &iced::Theme,
) -> iced::Element<'a, Message> {
    let author = comment
        .user
        .as_ref()
        .map(|u| u.name.clone())
        .unwrap_or_else(|| "anonymous".into());

    // Relative age ("3h ago") with the full timestamp on hover.
    let date_elem: iced::Element<'_, Message> = match comment.date {
        Some(d) => iced::widget::tooltip(
            iced::widget::text(relative_date(d.with_timezone(&chrono::Utc)))
                .size(9)
                .color(theme::muted(t)),
            iced::widget::text(d.format("%Y-%m-%d %H:%M").to_string()).size(11),
            iced::widget::tooltip::Position::Top,
        )
        .style(crate::ui::tooltip)
        .padding(4)
        .into(),
        None => iced::widget::text("").into(),
    };

    // Profile picture (24x24, cover-cropped circle) if loaded; otherwise a
    // subtle placeholder circle.
    let pfp: iced::Element<'_, Message> = match comment.user.as_ref().and_then(|u| pfps.get(&u.id))
    {
        Some(handle) => iced::widget::container(crate::widgets::cover_image::cover_image(
            handle.clone(),
            24.0,
        ))
        .width(24)
        .height(24)
        .clip(true)
        .style(|_theme| iced::widget::container::Style {
            border: iced::Border {
                radius: 12.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into(),
        None => iced::widget::container(iced::widget::text(""))
            .width(24)
            .height(24)
            .style(|theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(theme::panel(theme))),
                border: iced::Border {
                    radius: 12.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
    };

    // Header: pfp, name, a transparent gap, then the relative date.
    let header = iced::widget::row![
        pfp,
        iced::widget::text(author).size(12),
        iced::widget::Space::new().width(iced::Length::Fill),
        date_elem,
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);

    // Body: the comment text, or the stamp image when one is attached (pixiv
    // sends `(stamp-name)` placeholders in the text alongside the image).
    let body: iced::Element<'a, Message> = match comment.stamp.as_ref() {
        Some(stamp) => {
            if let Some(_url) = stamp.stamp_url.as_deref() {
                if let Some(handle) = stamps.get(&comment.id) {
                    // Stamp image (48x48, cover-cropped).
                    crate::widgets::cover_image::cover_image(handle.clone(), 48.0)
                } else if comment.comment.trim().is_empty() || comment.comment.starts_with('(') {
                    iced::widget::text(format!("[stamp: {}]", stamp_name(&comment.comment)))
                        .size(10)
                        .color(theme::muted(t))
                        .into()
                } else {
                    iced::widget::text(&comment.comment).size(11).into()
                }
            } else {
                iced::widget::text(&comment.comment).size(11).into()
            }
        }
        None => iced::widget::text(&comment.comment).size(11).into(),
    };

    let body = iced::widget::container(body)
        .width(iced::Length::Fill)
        .padding(iced::Padding::from([2, 4]));

    // Alternating, very slightly different row backgrounds.
    let row = iced::widget::column![header, body].spacing(2);
    iced::widget::container(row)
        .width(iced::Length::Fill)
        .padding(iced::Padding::from([4, 6]))
        .style(move |theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(if index % 2 == 0 {
                theme::bg(theme)
            } else {
                theme::panel(theme).scale_alpha(0.55)
            })),
            ..Default::default()
        })
        .into()
}

/// Extract a readable name from a pixiv stamp placeholder like `(love2)`.
fn stamp_name(placeholder: &str) -> String {
    placeholder
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comments_empty() {
        let map = HashMap::new();
        let t = iced::Theme::Dark;
        let el = comments(&[], &map, &map, false, &t);
        drop(el);
    }

    #[test]
    fn test_comments_with_loading_footer() {
        use pixivj_rs::model::Author;
        let map = HashMap::new();
        let t = iced::Theme::Dark;
        let comment = Comment {
            id: 1,
            comment: "hi".into(),
            date: None,
            has_replies: None,
            user: Some(Author {
                id: 7,
                name: "tester".into(),
                account: None,
                profile_image_urls: None,
                comment: None,
                is_followed: None,
            }),
            stamp: None,
        };
        let comments_slice = [comment];
        let el = comments(&comments_slice, &map, &map, true, &t);
        drop(el);
    }

    #[test]
    fn test_relative_date_units() {
        let now = chrono::Utc::now();
        assert_eq!(relative_date(now), "just now");
        assert_eq!(relative_date(now - chrono::Duration::hours(5)), "5h ago");
    }

    #[test]
    fn test_stamp_name() {
        assert_eq!(stamp_name("(love2)"), "love2");
        assert_eq!(stamp_name("(heart)"), "heart");
        assert_eq!(stamp_name(""), "");
    }
}
