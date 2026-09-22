use crate::app::message::Message;
use crate::theme;
use crate::widgets::{blur_image, illust_info, parallax_image};
use pixivj_rs::model::Illustration;

#[allow(clippy::too_many_arguments)]
pub fn gallery_item<'a>(
    illust: &'a Illustration,
    width: f32,
    height: f32,
    thumbnail_handle: Option<&'a iced::widget::image::Handle>,
    parallax: parallax_image::ParallaxOptions,
    downloaded: bool,
    download_progress: Option<f32>,
    text: illust_info::TileText,
    // Bookmark dimming: the item's bookmark state + whether non-Bookmarks
    // tabs dim bookmarked items.
    dim_bookmarked: bool,
    // R-18 blur: enabled by the setting (the shader animates un-blur on
    // hover itself).
    blur_r18: bool,
) -> iced::Element<'a, Message> {
    let id = illust.id;
    let bookmarked = illust.bookmarked();

    let thumbnail: iced::Element<'_, Message> = if let Some(handle) = thumbnail_handle {
        // Cached handle → parallax crop-to-fill tile (no per-frame decode).
        // The parallax fills the whole cell, so there's no visible seam.
        let base = parallax_image::parallax_image(handle.clone(), width, height, parallax);
        if blur_r18 && illust.is_r18() {
            // The R-18 censoring layer sits ON TOP of the parallax tile: a
            // GPU-blurred cover crop whose radius AND alpha animate to
            // sharp/transparent while hovered, so the parallax (and the
            // cursor/scroll motion it follows) stays alive underneath.
            match blur_image::blur_image(handle, width, height) {
                Some(blur) => iced::widget::stack![base, blur].into(),
                None => base,
            }
        } else {
            base
        }
    } else {
        // Placeholder while loading (theme-aware background, no border).
        iced::widget::container(
            iced::widget::text(format!("#{}", crate::media::native_id(id))).size(12),
        )
        .width(width)
        .height(height)
        .center_x(iced::Length::Fill)
        .center_y(iced::Length::Fill)
        .style(|theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::panel(theme))),
            ..Default::default()
        })
        .into()
    };

    // "Downloaded to disk" checkmark (top-left corner).
    let check: iced::Element<'_, Message> = if downloaded {
        iced::widget::container(
            iced::widget::text("✓")
                .size(text.author_size())
                .color(iced::Color::WHITE),
        )
        .padding(iced::Padding::from([1, 4]))
        .style(|_| iced::widget::container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgba(
                0.2, 0.7, 0.3, 0.85,
            ))),
            border: iced::Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    } else {
        iced::widget::text("").into()
    };

    // Provider badge (top-right): marks non-Pixiv tiles in the mixed tabs.
    let badge_label = crate::media::provider_of(id).badge();
    let badge: iced::Element<'_, Message> = match badge_label {
        Some(label) => iced::widget::container(
            iced::widget::text(label)
                .size(text.secondary_size())
                .color(iced::Color::WHITE),
        )
        .padding(iced::Padding::from([1, 5]))
        .style(|_| iced::widget::container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgba(
                0.0, 0.0, 0.0, 0.65,
            ))),
            border: iced::Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into(),
        None => iced::widget::text("").into(),
    };

    let info = illust_info::illust_info(illust, text);

    // Aggregated download progress (bottom edge): a thin bar tracking the
    // downloader's overall multi-page fraction while the item is in flight.
    // It is only built while the tracker holds the item, so it vanishes as
    // soon as the download completes (DownloadCompleted clears the entry).
    let progress: iced::Element<'_, Message> = match download_progress {
        Some(frac) => {
            let bar = iced::widget::progress_bar(0.0..=1.0, frac.clamp(0.0, 1.0))
                .length(iced::Length::Fill)
                .girth(4.0)
                .style(|theme| iced::widget::progress_bar::Style {
                    background: iced::Background::Color(iced::Color::from_rgba(
                        0.0, 0.0, 0.0, 0.55,
                    )),
                    bar: iced::Background::Color(theme::accent(theme)),
                    border: iced::Border::default(),
                });
            iced::widget::container(bar)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .align_y(iced::Alignment::End)
                .into()
        }
        None => iced::widget::text("").into(),
    };

    // The optional dim overlay only exists while it applies, so an
    // untouched tile keeps the exact original stack shape (widget-tree
    // identity and click handling unchanged). No centered icon: dimming is
    // purely a visual fade of the thumbnail (the info text stays readable).
    let dim = dim_bookmarked && bookmarked;
    let mut layers: Vec<iced::Element<'a, Message>> = vec![thumbnail];
    if dim {
        layers.push(
            iced::widget::container(iced::widget::text(""))
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .style(|_| iced::widget::container::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgba(
                        0.0, 0.0, 0.0, 0.55,
                    ))),
                    ..Default::default()
                })
                .into(),
        );
    }
    layers.push(
        iced::widget::container(check)
            .align_top(iced::Length::Fill)
            .align_left(iced::Length::Fill)
            .into(),
    );
    layers.push(
        iced::widget::container(badge)
            .align_top(iced::Length::Fill)
            .align_right(iced::Length::Fill)
            .into(),
    );
    layers.push(
        iced::widget::container(info)
            .align_bottom(iced::Length::Fill)
            .into(),
    );
    layers.push(progress);
    let stack = iced::widget::Stack::with_children(layers);

    // Seamless tile: no padding, no border, no radius — the image itself
    // fills the cell and adjacent tiles touch edge to edge. Height is fixed
    // (the grid size); width varies with the window.
    //
    // Clip the contents ourselves: iced's `button.clip(true)` forwards
    // `bounds ∩ viewport` to its child, which made a fully visible tile's
    // scroll-parallax reference viewport equal to the tile itself (zero
    // offset). `ClipViewport` clips the drawing but keeps the gallery's real
    // viewport for the parallax math.
    iced::widget::button(crate::widgets::clip_viewport::ClipViewport::new(stack))
        .on_press(Message::GalleryItemClicked(id))
        .clip(false)
        .padding(iced::Padding::ZERO)
        .style(|theme, _| iced::widget::button::Style {
            background: Some(iced::Background::Color(theme::panel(theme))),
            ..Default::default()
        })
        .width(width)
        .height(height)
        .into()
}
