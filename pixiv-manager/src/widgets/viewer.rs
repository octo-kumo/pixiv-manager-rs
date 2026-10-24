use crate::app::message::Message;
use crate::app::state::AppState;
use crate::theme;
use crate::widgets::{comments, illust_detail};

/// Full-screen image viewer overlay.
///
/// Layout: a horizontal split — the left side is just the image with the page
/// selector and a full-width progress bar floating over it; the right side is
/// a vertical split with image info + action buttons + tags on top and a
/// scrollable comments list below, with the close button floating at its
/// bottom-right.
pub fn viewer(state: &AppState) -> iced::Element<'_, Message> {
    let illust_id = state.viewer.illust_id;
    let page = state.viewer.page;
    let total_pages = state.viewer.total_pages.max(1);
    // Resolved once and borrowed for the whole build (theme-aware text colors).
    let t = crate::theme::theme(state);

    // ── Left: image area with floating page selector + progress bar ──
    let image_area = image_with_overlay(state, page, total_pages);

    // ── Right panel: info + actions + tags on top, comments below ───
    let right_panel: iced::Element<'_, Message> = match &state.viewer.illust {
        Some(illust) => {
            let comments_scroll = iced::widget::container(comments::comments_with_label(
                &state.viewer.comments,
                &state.viewer.comment_pfps,
                &state.viewer.comment_stamps,
                state.viewer.comments_loading,
                &t,
                crate::media::is_twitter(state.viewer.illust_id).then_some("No replies yet"),
            ))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill);

            // Close button floats at the bottom-right, on top of the comments.
            let close = iced::widget::container(
                crate::ui::button(iced::widget::text("✕").size(14))
                    .on_press(Message::ViewerClosed)
                    .padding(iced::Padding::from([4, 10])),
            )
            .align_x(iced::alignment::Horizontal::Right)
            .align_y(iced::alignment::Vertical::Bottom)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill);

            iced::widget::column![
                illust_detail::illust_detail(
                    illust,
                    state.viewer.author_pfp.as_ref(),
                    download_icon(state, illust_id),
                    state.active_tab_data().downloaded.contains(&illust_id),
                    &t,
                ),
                iced::widget::Stack::with_children(vec![comments_scroll.into(), close.into(),])
                    .width(iced::Length::Fill)
                    .height(iced::Length::Fill),
            ]
            .into()
        }
        None => iced::widget::container(
            iced::widget::text(if state.viewer.loading {
                "Loading detail…"
            } else {
                "No detail available"
            })
            .size(14)
            .color(theme::muted(&crate::theme::theme(state))),
        )
        .center_x(iced::Length::Fill)
        .center_y(iced::Length::Fill)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into(),
    };

    let center = iced::widget::row![
        image_area,
        iced::widget::container(right_panel)
            .width(300)
            .height(iced::Length::Fill),
    ];

    // ── Assemble full viewer ────────────────────────────────────────
    // The outer mouse_area captures every mouse event so the gallery behind
    // the overlay is neither visible-through nor interactive.
    let content = iced::widget::container(center)
        .style(crate::ui::page)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill);

    iced::widget::mouse_area(content)
        .on_press(Message::Noop)
        .on_release(Message::Noop)
        .on_right_press(Message::Noop)
        .on_right_release(Message::Noop)
        .on_middle_press(Message::Noop)
        .on_middle_release(Message::Noop)
        .on_scroll(|_| Message::Noop)
        .into()
}

/// The image with a floating page selector (top-center) and a full-width
/// load-progress bar at the very top. The bar uses a dark translucent track
/// and a bright fill so it stays readable over any image; it hides itself
/// once loading completes.
fn image_with_overlay(
    state: &AppState,
    page: usize,
    total_pages: usize,
) -> iced::Element<'_, Message> {
    let mut layers: Vec<iced::Element<'_, Message>> = Vec::new();

    // The image / placeholder.
    match crate::app::update::transform::display_handle(state) {
        Some(handle) => {
            let image = iced::widget::image::viewer(handle.clone())
                .content_fit(iced::ContentFit::Contain)
                .min_scale(1.0)
                .max_scale(20.0)
                .scale_step(0.1)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill);
            layers.push(
                iced::widget::container(image)
                    .center_x(iced::Length::Fill)
                    .center_y(iced::Length::Fill)
                    .width(iced::Length::Fill)
                    .height(iced::Length::Fill)
                    .into(),
            );
        }
        _ => {
            layers.push(
                iced::widget::container(
                    iced::widget::text(if state.viewer.loading {
                        "Loading..."
                    } else {
                        "No image"
                    })
                    .size(20)
                    .color(theme::muted(&crate::theme::theme(state))),
                )
                .center_x(iced::Length::Fill)
                .center_y(iced::Length::Fill)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into(),
            );
        }
    }

    // Floating page selector (top-center). While the jump-to-page box is not
    // being edited it mirrors the current page number (`page_display`).
    let page_box_value = if state.viewer.page_input.is_empty() {
        &state.viewer.page_display
    } else {
        &state.viewer.page_input
    };
    layers.push(
        iced::widget::container(page_selector(page, total_pages, page_box_value.as_str()))
            .align_top(iced::Length::Fill)
            .center_x(iced::Length::Fill)
            .padding(8)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into(),
    );

    // Full-width load progress bar at the very top (only while loading).
    if let Some(frac) = state.viewer.progress {
        let bar = iced::widget::progress_bar(0.0..=1.0, frac).style(|theme| {
            iced::widget::progress_bar::Style {
                // Dark translucent track so it's visible over bright images.
                background: iced::Background::Color(iced::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.55,
                }),
                bar: iced::Background::Color(theme::accent(theme)),
                border: iced::Border {
                    radius: 0.0.into(),
                    ..Default::default()
                },
            }
        });
        layers.push(
            iced::widget::container(bar)
                .width(iced::Length::Fill)
                .height(6)
                .into(),
        );
    }

    // Combine / rotate tools (Twitter items).
    if let Some(toolbar) = crate::widgets::viewer_toolbar::toolbar(state) {
        layers.push(toolbar);
    }

    // Palette swatches (MMCQ color extraction) floating at the bottom.
    if let Some(palette) = state.images.palettes.get(&state.viewer.illust_id) {
        let mut swatches: Vec<iced::Element<'_, Message>> = Vec::new();
        for color in palette.iter().take(5) {
            let c = *color;
            swatches.push(
                iced::widget::container(iced::widget::text("").width(16).height(16))
                    .style(move |_| iced::widget::container::Style {
                        background: Some(iced::Background::Color(iced::Color::from_rgb(
                            c[0] as f32 / 255.0,
                            c[1] as f32 / 255.0,
                            c[2] as f32 / 255.0,
                        ))),
                        border: iced::Border {
                            color: iced::Color::from_rgb(0.3, 0.3, 0.3),
                            width: 1.0,
                            radius: 2.0.into(),
                        },
                        ..Default::default()
                    })
                    .into(),
            );
        }
        layers.push(
            iced::widget::container(iced::widget::Row::with_children(swatches).spacing(2))
                .align_bottom(iced::Length::Fill)
                .center_x(iced::Length::Fill)
                .padding(8)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into(),
        );
    }

    iced::widget::Stack::with_children(layers)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into()
}

/// The floating page selector: ◀ [N] / N ▶ (hidden for single-page works).
/// The small box is a jump-to-page input (1-based, Enter to jump).
fn page_selector(page: usize, total_pages: usize, value: &str) -> iced::Element<'_, Message> {
    if total_pages <= 1 {
        return iced::widget::text("").into();
    }
    let digits = total_pages.to_string().len().max(2);
    iced::widget::container(
        iced::widget::row![
            icon_btn(
                "◀",
                Message::ViewerPageNavigate(crate::app::state::ViewerPageNav::Prev),
                page > 0
            ),
            crate::ui::text_input("–", value)
                .on_input(Message::ViewerPageInputChanged)
                .on_submit(Message::ViewerPageSubmitted)
                .size(12)
                .width(iced::Length::Fixed(20.0 + 7.0 * digits as f32))
                .align_x(iced::alignment::Horizontal::Center),
            iced::widget::text(format!("/ {total_pages}")).size(12),
            icon_btn(
                "▶",
                Message::ViewerPageNavigate(crate::app::state::ViewerPageNav::Next),
                page + 1 < total_pages
            ),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center),
    )
    .padding(iced::Padding::from([4, 10]))
    .style(crate::ui::glass_bar)
    .into()
}

/// An icon-only button (used by the page selector).
fn icon_btn(label: &str, msg: Message, enabled: bool) -> iced::Element<'_, Message> {
    let mut btn =
        crate::ui::button(iced::widget::text(label).size(13)).padding(iced::Padding::from([2, 8]));
    if enabled {
        btn = btn.on_press(msg);
    }
    // Disabled controls are visibly dimmed instead of looking clickable.
    btn.style(|theme, status| {
        let mut style = crate::ui::glass(false)(theme, status);
        if status == iced::widget::button::Status::Disabled {
            style.text_color = style.text_color.scale_alpha(0.35);
        }
        style
    })
    .into()
}

/// The download control as an icon-only button:
/// * idle: "⬇"
/// * downloading: "⏳ NN%" (disabled)
/// * done: "✓" (disabled)
///
/// All three size to their content: a fixed width clipped the progress
/// percentage text outside the button (the reported bug).
fn download_icon(state: &AppState, illust_id: u64) -> iced::Element<'_, Message> {
    if state.jobs.downloads.is_downloading(illust_id) {
        let frac = state
            .jobs
            .downloads
            .get(illust_id)
            .map(|s| s.progress_fraction())
            .unwrap_or(0.0);
        crate::ui::button(iced::widget::text(format!("⏳{:.0}%", frac * 100.0)).size(12))
            .width(iced::Length::Shrink)
            .height(26)
            .padding(iced::Padding::from([2, 8]))
            .into()
    } else if state.jobs.downloads.is_completed(illust_id) {
        crate::ui::button(
            iced::widget::text("✓")
                .size(14)
                .color(crate::theme::SUCCESS),
        )
        .width(iced::Length::Shrink)
        .height(26)
        .padding(iced::Padding::from([2, 10]))
        .into()
    } else {
        crate::ui::button(iced::widget::text("⬇").size(14))
            .on_press(Message::DownloadStarted(illust_id))
            .width(iced::Length::Shrink)
            .height(26)
            .padding(iced::Padding::from([2, 10]))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_viewer_builds() {
        // Test that viewer builds without panic for a simple state
        let state = AppState::default();
        let el = viewer(&state);
        drop(el);
    }
}
