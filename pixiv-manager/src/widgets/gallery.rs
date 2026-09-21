use crate::app::message::Message;
use crate::widgets::{gallery_item, illust_info, virtual_gallery};
use pixivj_rs::model::Illustration;
use std::collections::HashMap;

/// Responsive grid of seamless, touching gallery tiles, virtualized and
/// smoothly scrolled.
///
/// The tile size is derived from the available width and the column count, so
/// the grid reflows on window resize: tiles expand horizontally to fill the
/// row and a new column is added once the width crosses a per-tile threshold.
/// There is no spacing, padding, border, or background — images touch.
///
/// Only the rows around `scroll_offset` (plus a one-viewport buffer on each
/// side) are actually built: with thousands of bookmarks the per-frame cost
/// stays flat. `viewport_height` is the approximate gallery height, used for
/// the row window.
#[allow(clippy::too_many_arguments)]
pub fn gallery<'a>(
    illusts: &'a [Illustration],
    available_width: f32,
    column_count: u32,
    thumbnails: &'a HashMap<u64, iced::widget::image::Handle>,
    parallax: crate::widgets::parallax_image::ParallaxOptions,
    scroll_offset: f32,
    viewport_height: f32,
    downloaded: &'a std::collections::HashSet<u64>,
    downloads: &'a crate::io::download::DownloadTracker,
    text: illust_info::TileText,
    thumbnail_avg: &HashMap<u64, [u8; 3]>,
    tile_height: f32,
    scroll_key: u64,
    restore_offset: f32,
    dim_bookmarked: bool,
    blur_r18: bool,
    hovered: &'a std::collections::HashSet<u64>,
) -> iced::Element<'a, Message> {
    if illusts.is_empty() {
        return iced::widget::container(iced::widget::text("No illustrations").size(16))
            .center_x(iced::Length::Fill)
            .center_y(iced::Length::Fill)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into();
    }

    let cc = column_count.max(1) as usize;
    // Fixed-height tiles: the height is the grid size (from settings), and
    // the width fills the available width per column — so tiles are
    // rectangles whose width follows window resizes and whose height stays
    // constant.
    let tile_w = (available_width / cc as f32).max(1.0);
    let tile_h = tile_height.max(1.0);
    let total_rows = illusts.len().div_ceil(cc);

    // Virtualize: build only the rows around the current scroll offset, with
    // a one-viewport buffer on each side so a fast glide never reveals an
    // empty strip while the window catches up. Rows are `tile_h` tall.
    let (first_row, last_row) =
        visible_row_range(scroll_offset, viewport_height.max(1.0), tile_h, total_rows);

    let mut rows: Vec<iced::Element<'a, Message>> = Vec::new();
    for row_idx in first_row..=last_row {
        let chunk = &illusts[row_idx * cc..((row_idx + 1) * cc).min(illusts.len())];
        let mut row_elems: Vec<iced::Element<'a, Message>> = Vec::new();
        for illust in chunk {
            let thumb = thumbnails.get(&illust.id);
            let on_disk = downloaded.contains(&illust.id);
            let progress = downloads.progress(illust.id);
            let tile_text = illust_info::TileText {
                avg: thumbnail_avg.get(&illust.id).copied(),
                ..text
            };
            row_elems.push(gallery_item::gallery_item(
                illust,
                tile_w,
                tile_h,
                thumb,
                parallax,
                on_disk,
                progress,
                tile_text,
                dim_bookmarked,
                blur_r18,
                hovered.contains(&illust.id),
            ));
        }
        // No spacing between tiles — images touch, like the original app.
        rows.push(iced::widget::Row::with_children(row_elems).into());
    }

    virtual_gallery::VirtualGallery::new(
        rows,
        first_row,
        total_rows,
        tile_h,
        scroll_key,
        restore_offset,
    )
    .into()
}

/// Number of columns for a given width: enough that each tile is at least
/// `min_tile` wide, capped to keep tiles from becoming absurdly small.
#[allow(dead_code)]
pub fn col_count(container_width: u32, min_tile: u32) -> u32 {
    if min_tile == 0 {
        return 1;
    }
    (container_width / min_tile).clamp(1, 7)
}

/// The row window to build for a given scroll offset: the visible rows plus a
/// one-viewport buffer on each side (so a fast glide never shows an empty
/// strip), clamped to the content.
fn visible_row_range(
    scroll_offset: f32,
    viewport_height: f32,
    tile_size: f32,
    total_rows: usize,
) -> (usize, usize) {
    if total_rows == 0 {
        return (0, 0);
    }
    let buffer_rows = (viewport_height / tile_size).ceil() as usize;
    let first = ((scroll_offset / tile_size).floor() as usize)
        .saturating_sub(buffer_rows)
        .min(total_rows - 1);
    let last = (((scroll_offset + viewport_height) / tile_size).ceil() as usize)
        .saturating_add(buffer_rows)
        .min(total_rows - 1);
    (first, last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_count_standard() {
        assert_eq!(col_count(1000, 200), 5);
    }

    #[test]
    fn col_count_capped_at_7() {
        assert_eq!(col_count(2000, 200), 7);
    }

    #[test]
    fn col_count_min_1() {
        assert_eq!(col_count(199, 200), 1);
    }

    #[test]
    fn col_count_zero_grid() {
        assert_eq!(col_count(1000, 0), 1);
    }

    #[test]
    fn gallery_empty() {
        let map = HashMap::new();
        let avg = HashMap::new();
        let downloaded = std::collections::HashSet::new();
        let hovered = std::collections::HashSet::new();
        let downloads = crate::io::download::DownloadTracker::default();
        let el = gallery(
            &[],
            1000.0,
            5,
            &map,
            crate::widgets::parallax_image::ParallaxOptions::ALL,
            0.0,
            700.0,
            &downloaded,
            &downloads,
            illust_info::TileText::default(),
            &avg,
            200.0,
            1,
            0.0,
            false,
            false,
            &hovered,
        );
        drop(el);
    }

    #[test]
    fn virtual_window_at_top() {
        // 3 visible rows + 3 buffer rows each side (600/200).
        assert_eq!(visible_row_range(0.0, 600.0, 200.0, 200), (0, 6));
    }

    #[test]
    fn virtual_window_follows_scroll() {
        // Scrolled to row 50 (y=10000): visible rows 50..53 plus a 3-row
        // buffer on each side → 47..56.
        let (first, last) = visible_row_range(10000.0, 600.0, 200.0, 200);
        assert_eq!((first, last), (47, 56), "got {first}..{last}");
    }

    #[test]
    fn virtual_window_clamped_at_bottom() {
        let (first, last) = visible_row_range(39800.0, 600.0, 200.0, 200);
        assert_eq!(last, 199, "never beyond the last row");
        assert!(first >= 193, "got {first}");
    }

    #[test]
    fn virtual_window_empty_content() {
        assert_eq!(visible_row_range(0.0, 600.0, 200.0, 0), (0, 0));
    }
}
