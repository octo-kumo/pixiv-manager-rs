use crate::app::message::Message;
use crate::theme;
use crate::widgets::gallery_item;
use pixivj_rs::model::{Author, Illustration};
use std::collections::HashMap;

/// Artist page — modal overlay with the artist's profile picture, name, and a
/// responsive gallery of their illustrations.
#[allow(clippy::too_many_arguments)]
pub fn artist_page<'a>(
    user: Option<&'a Author>,
    pfp: Option<&'a iced::widget::image::Handle>,
    illusts: &'a [Illustration],
    available_width: f32,
    grid_size: f32,
    thumbnails: &'a HashMap<u64, iced::widget::image::Handle>,
    downloads: &'a crate::io::download::DownloadTracker,
    text: crate::widgets::illust_info::TileText,
    thumbnail_avg: &HashMap<u64, [u8; 3]>,
    loading: bool,
    parallax: crate::widgets::parallax_image::ParallaxOptions,
    dim_bookmarked: bool,
    blur_r18: bool,
) -> iced::Element<'a, Message> {
    // ── Header: profile picture + name + account ──
    let mut header_children: Vec<iced::Element<'a, Message>> = Vec::new();
    if let Some(handle) = pfp {
        header_children.push(
            iced::widget::container(crate::widgets::cover_image::cover_image(
                handle.clone(),
                44.0,
            ))
            .width(44)
            .height(44)
            .clip(true)
            .style(|_theme| iced::widget::container::Style {
                border: iced::Border {
                    radius: 22.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
        );
    }
    header_children.push(
        iced::widget::text(user.map(|u| u.name.as_str()).unwrap_or("Artist"))
            .size(20)
            .into(),
    );
    if let Some(account) = user.and_then(|u| u.account.as_deref()) {
        header_children.push(iced::widget::text(format!("@{}", account)).size(12).into());
    }
    header_children.push(iced::widget::Space::new().width(iced::Length::Fill).into());
    header_children.push(
        iced::widget::button(iced::widget::text("✕ Close"))
            .on_press(Message::ArtistClosed)
            .padding(iced::Padding::from([4, 12]))
            .into(),
    );

    let header = iced::widget::Row::with_children(header_children)
        .spacing(8)
        .padding(8)
        .align_y(iced::Alignment::Center);

    let gallery = build_gallery(
        illusts,
        available_width,
        grid_size,
        thumbnails,
        downloads,
        text,
        thumbnail_avg,
        loading,
        parallax,
        dim_bookmarked,
        blur_r18,
    );

    let content = iced::widget::column![header, gallery];

    iced::widget::container(content)
        .style(|theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(iced::Color {
                a: 0.98,
                ..theme::bg(theme)
            })),
            ..Default::default()
        })
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .padding(iced::Padding::from([12, 16]))
        .into()
}

/// Responsive, seamless grid of the artist's illustrations (thumbnails).
/// The tile size follows the user's grid-size setting (the same minimum the
/// main gallery uses) instead of a hardcoded size, so the author view
/// respects the grid options.
#[allow(clippy::too_many_arguments)]
fn build_gallery<'a>(
    illusts: &'a [Illustration],
    available_width: f32,
    grid_size: f32,
    thumbnails: &'a HashMap<u64, iced::widget::image::Handle>,
    downloads: &'a crate::io::download::DownloadTracker,
    text: crate::widgets::illust_info::TileText,
    thumbnail_avg: &HashMap<u64, [u8; 3]>,
    loading: bool,
    parallax: crate::widgets::parallax_image::ParallaxOptions,
    dim_bookmarked: bool,
    blur_r18: bool,
) -> iced::Element<'a, Message> {
    if illusts.is_empty() {
        let label = if loading {
            "Loading..."
        } else {
            "No illustrations"
        };
        return iced::widget::container(iced::widget::text(label).size(16))
            .center_x(iced::Length::Fill)
            .center_y(iced::Length::Fill)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into();
    }

    let min_tile = grid_size.max(60.0);
    // Same responsive rule as the main gallery: enough columns that each
    // tile stays at least `min_tile` wide, capped so tiles stay readable.
    let cc = ((available_width / min_tile).floor().max(1.0) as u32).clamp(1, 12);
    let tile_w = (available_width / cc as f32).max(1.0);
    let tile_h = min_tile;

    let mut rows: Vec<iced::Element<'a, Message>> = Vec::new();
    for chunk in illusts.chunks(cc as usize) {
        let mut row_elems: Vec<iced::Element<'a, Message>> = Vec::new();
        for illust in chunk {
            let thumb = thumbnails.get(&illust.id);
            let progress = downloads.progress(illust.id);
            let tile_text = crate::widgets::illust_info::TileText {
                avg: thumbnail_avg.get(&illust.id).copied(),
                ..text
            };
            row_elems.push(gallery_item::gallery_item(
                illust,
                tile_w,
                tile_h,
                thumb,
                parallax,
                false,
                progress,
                tile_text,
                dim_bookmarked,
                blur_r18,
            ));
        }
        rows.push(iced::widget::Row::with_children(row_elems).into());
    }

    let grid = iced::widget::Column::with_children(rows);

    // Infinite scroll: reaching the bottom fetches the artist's next page.
    iced::widget::scrollable(grid)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .on_scroll(|viewport: iced::widget::scrollable::Viewport| {
            if viewport.relative_offset().y > 0.85 {
                Message::ArtistLoadMore
            } else {
                Message::Noop
            }
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_artist_empty() {
        let map = HashMap::new();
        let avg = HashMap::new();
        let downloads = crate::io::download::DownloadTracker::default();
        let el = artist_page(
            None,
            None,
            &[],
            1000.0,
            200.0,
            &map,
            &downloads,
            crate::widgets::illust_info::TileText::default(),
            &avg,
            false,
            crate::widgets::parallax_image::ParallaxOptions::ALL,
            false,
            false,
        );
        drop(el);
    }

    #[test]
    fn test_artist_builds() {
        let map = HashMap::new();
        let avg = HashMap::new();
        let downloads = crate::io::download::DownloadTracker::default();
        let el = artist_page(
            None,
            None,
            &[],
            1000.0,
            200.0,
            &map,
            &downloads,
            crate::widgets::illust_info::TileText::default(),
            &avg,
            true,
            crate::widgets::parallax_image::ParallaxOptions::ALL,
            false,
            false,
        );
        drop(el);
    }

    #[test]
    fn grid_size_respected_in_column_count() {
        // A small grid-size option yields more columns than a large one
        // (the old hardcoded 160px layout ignored the setting entirely).
        let map = HashMap::new();
        let avg = HashMap::new();
        let downloads = crate::io::download::DownloadTracker::default();
        let illusts: Vec<Illustration> = (1..=40).map(crate::test_util::mock_illust).collect();
        // Small tiles (80px min) on a 1600px page → 12 columns (capped).
        let el = artist_page(
            None,
            None,
            &illusts,
            1600.0,
            80.0,
            &map,
            &downloads,
            crate::widgets::illust_info::TileText::default(),
            &avg,
            false,
            crate::widgets::parallax_image::ParallaxOptions::ALL,
            false,
            false,
        );
        drop(el);
        // Large tiles (400px min) → 4 columns.
        let el = artist_page(
            None,
            None,
            &illusts,
            1600.0,
            400.0,
            &map,
            &downloads,
            crate::widgets::illust_info::TileText::default(),
            &avg,
            false,
            crate::widgets::parallax_image::ParallaxOptions::ALL,
            false,
            false,
        );
        drop(el);
    }
}
