//! A gallery tile image that fills its cell (crop-to-fill) and shifts with
//! the cursor while hovered and with its position in the viewport while
//! scrolling — a port of the old Java app's `GalleryImage` parallax:
//!
//! * **Mouse parallax** (`setParallax`): while hovered, the image follows the
//!   cursor (1.5x gain, clamped to ±1).
//! * **Scroll parallax** (`setRestParallax`): while not hovered, the image
//!   drifts so its crop follows the tile's position relative to the viewport
//!   center (`2·(viewportCenter − tileCenter)/viewportSize`), so tiles near
//!   the top of the screen reveal the upper part of the image and vice versa.
//!
//! Both can be toggled independently via [`ParallaxOptions`]. Implemented as a
//! custom widget because iced 0.14 offers no per-item image offset: the widget
//! draws the image larger than its cell (centered on it, so there is spare
//! area on every side) and translates it inside a clipped layer, so the crop
//! follows the target without ever revealing the cell background.
//!
//! The two effects are deliberately decoupled:
//! * **Scroll parallax is instant.** The rest offset is derived from the
//!   tile's viewport position *every frame in `draw`*, with no spring and no
//!   animation state, so scrolling the gallery moves every tile exactly with
//!   the content (no lag, and no "flash" when tiles are (re)created or when
//!   the gallery is rebuilt behind an overlay).
//! * **Mouse parallax springs.** While hovered the offset eases toward the
//!   cursor and decays back to 0 on leave, so the motion stays smooth without
//!   ever animating the scroll component.

use iced::advanced::image::Image;
use iced::advanced::layout;
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::widget::Widget;
use iced::advanced::{image, mouse, Clipboard, Layout, Shell};
use iced::{Border, Event, Length, Rectangle, Size, Vector};

/// Extra scaling beyond cover-fit, giving the image room to shift. `0.2`
/// means 10% of the cell on each side, matching the old app's 20px on a
/// 200px tile. Thumbnail generation uses this to size its output so the
/// parallax crop is displayed at ~1:1 pixels (sharp).
pub(crate) const PARALLAX_EXTRA: f32 = 0.2;
/// Cursor-to-offset gain (the old app used `1.5x`, clamped to ±1).
const PARALLAX_GAIN: f32 = 1.5;
/// Per-frame spring decay: how much the offset moves toward its target.
const SPRING_DECAY: f32 = 0.14;
/// Stop animating when the offset is within this of its target.
const SPRING_EPSILON: f32 = 0.001;

/// Which parallax effects are enabled for a tile.
#[derive(Debug, Clone, Copy, Default)]
pub struct ParallaxOptions {
    /// Master switch: when off, the tile draws statically (cover-fit, no
    /// extra scaling, no shifting).
    pub enabled: bool,
    /// Cursor-following parallax while hovered.
    pub mouse: bool,
    /// Scroll-position parallax (shift with the tile's viewport position).
    pub scroll: bool,
    /// Which scroll-parallax curve to use.
    pub mode: ParallaxMode,
}

/// The scroll-parallax curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ParallaxMode {
    /// **Centered**: the original Java `setRestParallax` behavior —
    /// `2·(viewportCenter − tileCenter)/viewportSize`, clamped to ±1. A tile
    /// above the viewport center reveals its upper part, at the center it
    /// shows the middle, below it shows the lower part.
    #[default]
    #[serde(rename = "centered")]
    Centered,
    /// **Linear**: true scroll parallax. The crop interpolates linearly
    /// across the image's full margin as the tile traverses the viewport:
    /// at ENTRY (tile top touching the viewport's bottom edge) the crop
    /// shows the image's BOTTOM; at EXIT (tile bottom at the viewport's top
    /// edge) it shows the image's TOP. The image lags the scroll — classic
    /// parallax — and the offset never exceeds the cover margin, so the
    /// cell is always fully covered.
    #[serde(rename = "linear")]
    Linear,
}

impl ParallaxOptions {
    /// All effects on (the default behavior).
    pub const ALL: Self = Self {
        enabled: true,
        mouse: true,
        scroll: true,
        mode: ParallaxMode::Centered,
    };
}

/// State stored in the widget tree: current + target mouse parallax offsets.
///
/// The scroll (rest) parallax is *stateless*: it is recomputed from the
/// viewport in `draw` every frame, so it always matches the tile's position
/// instantly. Only the cursor-following effect animates.
#[derive(Debug, Default)]
struct ParallaxState {
    /// Whether the cursor is over this tile.
    hovered: bool,
    /// Target mouse offset in `[-1, 1]` (cursor left → `+1`).
    target_x: f32,
    /// Target mouse offset in `[-1, 1]` (cursor top → `+1`).
    target_y: f32,
    /// Current (decayed) mouse offset in `[-1, 1]`.
    x: f32,
    /// Current (decayed) mouse offset in `[-1, 1]`.
    y: f32,
}

/// A crop-to-fill image with cursor-following and scroll-position parallax.
pub struct ParallaxImage {
    handle: iced::widget::image::Handle,
    width: f32,
    height: f32,
    options: ParallaxOptions,
}

impl ParallaxImage {
    /// Creates a new [`ParallaxImage`] with the given handle, cell size and
    /// parallax options.
    pub fn new(
        handle: iced::widget::image::Handle,
        width: f32,
        height: f32,
        options: ParallaxOptions,
    ) -> Self {
        Self {
            handle,
            width,
            height,
            options,
        }
    }

    /// The rest parallax offset for a tile, depending on the configured mode.
    ///
    /// `bounds` and `viewport` are both in content coordinates: iced passes
    /// scrollable children a viewport already translated by the scroll offset,
    /// so no extra math is needed.
    ///
    /// * **Centered** (`2·(viewportCenter − tileCenter)/viewportSize`): zero at
    ///   the screen center, ±1 at the edges (the old app's behavior).
    /// * **Linear** (tile position across the viewport, monotonic): the image
    ///   drifts linearly as the tile scrolls through — parallax everywhere.
    fn rest_offset(mode: ParallaxMode, bounds: &Rectangle, viewport: &Rectangle) -> (f32, f32) {
        match mode {
            // Centered: the original Java `setRestParallax` formula
            // `2·(visibleCenter − itemCenter)/visibleSize`, clamped to ±1. A
            // tile above the viewport center reveals its upper part, at the
            // center it shows the middle, below it shows the lower part.
            ParallaxMode::Centered => {
                let x = 2.0 * (viewport.center_x() - bounds.center_x()) / viewport.width;
                let y = 2.0 * (viewport.center_y() - bounds.center_y()) / viewport.height;
                (x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0))
            }
            // Linear: true scroll parallax. As the tile traverses the
            // viewport the crop sweeps across the image's full margin,
            // anchored at the edges: when the tile ENTERS (its top edge at
            // the viewport's bottom edge) the crop shows the image's BOTTOM
            // (offset −1); when it EXITS (its bottom edge at the viewport's
            // top edge) it shows the image's TOP (offset +1); in between it
            // interpolates linearly, passing through a centered crop at
            // mid-traversal. The image thus lags the scroll — the classic
            // parallax look — while |offset| ≤ 1 keeps the cell fully
            // covered at all times.
            ParallaxMode::Linear => {
                let vh = viewport.height.max(1.0);
                let th = bounds.height.max(1.0);
                // Traversal progress: 0 at entry (tile top at viewport
                // bottom), 1 at exit (tile bottom at viewport top).
                let p = (((viewport.y + viewport.height) - bounds.y) / (vh + th)).clamp(0.0, 1.0);
                let x = 1.0 - (bounds.center_x() - viewport.x) / viewport.width;
                ((x.clamp(0.0, 1.0)) * 2.0 - 1.0, 2.0 * p - 1.0)
            }
        }
    }

    /// Recompute the mouse target from the current hover/cursor state (called
    /// on every cursor move and every frame). The scroll (rest) parallax is
    /// NOT stored here — it is derived from the viewport in `draw` so it is
    /// always instant.
    fn update_target(&self, state: &mut ParallaxState, bounds: &Rectangle, cursor: mouse::Cursor) {
        if !self.options.enabled {
            state.hovered = false;
            state.target_x = 0.0;
            state.target_y = 0.0;
            return;
        }
        let hovering = self.options.mouse
            && cursor
                .position()
                .is_some_and(|position| bounds.contains(position));
        state.hovered = hovering;
        if hovering {
            if let Some(position) = cursor.position() {
                // Normalized cursor in the cell: left/top = 0, right/bottom = 1.
                let nx = (position.x - bounds.x) / bounds.width;
                let ny = (position.y - bounds.y) / bounds.height;
                state.target_x = (PARALLAX_GAIN * (1.0 - 2.0 * nx)).clamp(-1.0, 1.0);
                state.target_y = (PARALLAX_GAIN * (1.0 - 2.0 * ny)).clamp(-1.0, 1.0);
            }
        } else {
            state.target_x = 0.0;
            state.target_y = 0.0;
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for ParallaxImage
where
    Renderer: iced::advanced::Renderer + image::Renderer<Handle = iced::widget::image::Handle>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<ParallaxState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(ParallaxState::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fixed(self.width),
            height: Length::Fixed(self.height),
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(
            limits
                .width(Length::Fixed(self.width))
                .height(Length::Fixed(self.height))
                .resolve(
                    Length::Fixed(self.width),
                    Length::Fixed(self.height),
                    Size::new(self.width, self.height),
                ),
        )
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<ParallaxState>();

        // IMPORTANT: use the `cursor` parameter, not the raw event position.
        // Inside a scrollable, the event position is in window coordinates,
        // while `bounds` here are in content coordinates; the scrollable
        // adjusts the `cursor` parameter by the scroll offset. Using the raw
        // event position makes the parallax hit-test fail after scrolling.
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                self.update_target(state, &bounds, cursor);
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                // Cursor left the window — drift back to the rest position.
                self.update_target(state, &bounds, mouse::Cursor::Unavailable);
                shell.request_redraw();
            }
            Event::Window(iced::window::Event::RedrawRequested(_)) => {
                // Only the mouse parallax animates; the scroll (rest) parallax
                // is computed in draw and therefore always instant.
                self.update_target(state, &bounds, cursor);
                let tx = state.target_x;
                let ty = state.target_y;
                state.x += (tx - state.x) * SPRING_DECAY;
                state.y += (ty - state.y) * SPRING_DECAY;
                if (state.x - tx).abs() > SPRING_EPSILON || (state.y - ty).abs() > SPRING_EPSILON {
                    shell.request_redraw();
                } else {
                    state.x = tx;
                    state.y = ty;
                }
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        // Resize/transient guards: a zero-area cell or viewport (a window
        // being resized to a degenerate size, or a layer clipping its child
        // out entirely) must not reach the renderer — a zero or NaN-sized
        // layer/quad can panic the wgpu backend (the reported "random crash
        // on resize with parallax disabled").
        if bounds.width <= 0.0
            || bounds.height <= 0.0
            || viewport.width <= 0.0
            || viewport.height <= 0.0
        {
            return;
        }
        let state = tree.state.downcast_ref::<ParallaxState>();

        let Some(image_size) = renderer.measure_image(&self.handle) else {
            return;
        };
        let (iw, ih) = (image_size.width as f32, image_size.height as f32);
        if iw == 0.0 || ih == 0.0 {
            return;
        }

        // Scroll (rest) parallax is computed HERE, from the live viewport, so
        // it tracks the tile's position instantly — never spring-animated.
        // This is what keeps tiles glued to the content while scrolling and
        // prevents any "flash" when the gallery is rebuilt behind an overlay.
        let (rest_x, rest_y) = if self.options.enabled && self.options.scroll {
            Self::rest_offset(self.options.mode, &bounds, viewport)
        } else {
            (0.0, 0.0)
        };

        // Base cover-fit scale. When parallax is enabled, enlarge to leave
        // room for the shift; otherwise draw the plain cover image.
        let cover = (bounds.width / iw).max(bounds.height / ih);
        let draw_scale = if self.options.enabled {
            cover * (1.0 + PARALLAX_EXTRA)
        } else {
            cover
        };
        let draw_size = Size::new(iw * draw_scale, ih * draw_scale);

        // Spare margin on each side of the cell.
        let margin_w = (draw_size.width - bounds.width) / 2.0;
        let margin_h = (draw_size.height - bounds.height) / 2.0;
        // Combined offset (rest + decaying mouse spring), clamped to the
        // margin so the image always covers the whole cell.
        let ox = (rest_x + state.x).clamp(-1.0, 1.0);
        let oy = (rest_y + state.y).clamp(-1.0, 1.0);
        let tx = (ox * margin_w).clamp(-margin_w, margin_w);
        let ty = (oy * margin_h).clamp(-margin_h, margin_h);

        // Anchor the image centered on the cell, then shift by the parallax.
        // The margin exists on all four sides, so translating within ±margin
        // can never expose the cell background (left/top included).
        let translation = Vector::new(tx - margin_w, ty - margin_h);
        let drawing_bounds = Rectangle::new(bounds.position(), draw_size);

        // A degenerate draw size (a fully collapsed tile mid-resize) is
        // skipped rather than recorded — wgpu cannot clip a zero-area layer.
        if draw_size.width <= 0.0 || draw_size.height <= 0.0 {
            return;
        }
        renderer.with_layer(bounds, |renderer| {
            renderer.with_translation(translation, |renderer| {
                renderer.draw_image(
                    Image {
                        handle: self.handle.clone(),
                        border_radius: Border::default().radius,
                        filter_method: image::FilterMethod::default(),
                        rotation: iced::Radians(0.0),
                        opacity: 1.0,
                        snap: true,
                    },
                    drawing_bounds,
                    *viewport - translation,
                );
            });
        });
    }
}

impl<'a, Message, Theme, Renderer> From<ParallaxImage>
    for iced::Element<'a, Message, Theme, Renderer>
where
    Renderer: 'a + iced::advanced::Renderer + image::Renderer<Handle = iced::widget::image::Handle>,
{
    fn from(widget: ParallaxImage) -> Self {
        Self::new(widget)
    }
}

/// Build a parallax gallery tile around the given handle.
pub fn parallax_image(
    handle: iced::widget::image::Handle,
    width: f32,
    height: f32,
    options: ParallaxOptions,
) -> iced::Element<'static, crate::app::message::Message> {
    iced::Element::new(ParallaxImage::new(handle, width, height, options))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rectangle {
        Rectangle::new(iced::Point::new(x, y), Size::new(w, h))
    }

    #[test]
    fn rest_offset_centered_tile_is_zero() {
        // Tile center == viewport center (400, 300) → no offset (centered mode).
        let (x, y) = ParallaxImage::rest_offset(
            ParallaxMode::Centered,
            &rect(300.0, 200.0, 200.0, 200.0),
            &rect(0.0, 0.0, 800.0, 600.0),
        );
        assert!((x - 0.0).abs() < 1e-3 && (y - 0.0).abs() < 1e-3);
    }

    #[test]
    fn rest_offset_above_viewport_center_is_positive() {
        // Tile center above the viewport center → positive y (reveals the
        // upper part of the image), mirroring the old app's formula.
        let (x, y) = ParallaxImage::rest_offset(
            ParallaxMode::Centered,
            &rect(300.0, -200.0, 200.0, 200.0),
            &rect(0.0, 0.0, 800.0, 600.0),
        );
        assert!(y > 0.0);
        assert!(x.abs() < 1e-3);
        // Fully outside the viewport top → clamped to +1.
        let (_, y) = ParallaxImage::rest_offset(
            ParallaxMode::Centered,
            &rect(300.0, -5000.0, 200.0, 200.0),
            &rect(0.0, 0.0, 800.0, 600.0),
        );
        assert!((y - 1.0).abs() < 1e-3);
    }

    #[test]
    fn rest_offset_clamped_to_unit() {
        // A tile far outside the viewport still clamps to ±1 (centered mode).
        let (_, y) = ParallaxImage::rest_offset(
            ParallaxMode::Centered,
            &rect(300.0, -5000.0, 200.0, 200.0),
            &rect(0.0, 0.0, 800.0, 600.0),
        );
        assert!((y - 1.0).abs() < 1e-3);
        let (_, y) = ParallaxImage::rest_offset(
            ParallaxMode::Centered,
            &rect(300.0, 5000.0, 200.0, 200.0),
            &rect(0.0, 0.0, 800.0, 600.0),
        );
        assert!((y + 1.0).abs() < 1e-3);
    }

    #[test]
    fn linear_entry_shows_bottom_crop() {
        // ENTRY: the tile's top edge is at the viewport's bottom edge →
        // offset −1 → the crop shows the image's BOTTOM.
        let vp = rect(0.0, 0.0, 800.0, 600.0);
        let tile = rect(300.0, 600.0, 200.0, 200.0); // top == viewport bottom
        let (_, y) = ParallaxImage::rest_offset(ParallaxMode::Linear, &tile, &vp);
        assert!((y - (-1.0)).abs() < 1e-3, "entry y {y} must be −1");
    }

    #[test]
    fn linear_exit_shows_top_crop() {
        // EXIT: the tile's bottom edge is at the viewport's top edge →
        // offset +1 → the crop shows the image's TOP.
        let vp = rect(0.0, 0.0, 800.0, 600.0);
        let tile = rect(300.0, -200.0, 200.0, 200.0); // bottom == viewport top
        let (_, y) = ParallaxImage::rest_offset(ParallaxMode::Linear, &tile, &vp);
        assert!((y - 1.0).abs() < 1e-3, "exit y {y} must be +1");
    }

    #[test]
    fn linear_mid_traversal_is_centered() {
        // Halfway between entry and exit the crop is exactly centered.
        let vp = rect(0.0, 0.0, 800.0, 600.0);
        // p = (600 − y)/(600+200) = 0.5 → y = 200.
        let tile = rect(300.0, 200.0, 200.0, 200.0);
        let (_, y) = ParallaxImage::rest_offset(ParallaxMode::Linear, &tile, &vp);
        assert!((y - 0.0).abs() < 1e-3, "mid-traversal y {y} must be 0");
    }

    #[test]
    fn linear_changes_continuously_with_vertical_position() {
        // As the tile moves down through the viewport the offset changes
        // continuously (strictly decreasing) — the mode's whole point.
        let vp = rect(0.0, 0.0, 800.0, 600.0);
        let mut prev = f32::MAX;
        for i in 0..=8 {
            let y = -200.0 + i as f32 * 100.0; // exit (+1) → entry (−1)
            let tile = rect(300.0, y, 200.0, 200.0);
            let (_, v) = ParallaxImage::rest_offset(ParallaxMode::Linear, &tile, &vp);
            assert!(v < prev, "{v} must be below {prev} at tile y={y}");
            prev = v;
        }
        // Beyond the anchors it clamps (buffer rows off-screen).
        let (_, above) = ParallaxImage::rest_offset(
            ParallaxMode::Linear,
            &rect(300.0, -900.0, 200.0, 200.0),
            &vp,
        );
        assert!((above - 1.0).abs() < 1e-3);
        let (_, below) = ParallaxImage::rest_offset(
            ParallaxMode::Linear,
            &rect(300.0, 1300.0, 200.0, 200.0),
            &vp,
        );
        assert!((below - (-1.0)).abs() < 1e-3);
    }

    #[test]
    fn linear_differs_from_centered_on_visible_tiles() {
        // Regression: linear used to compute the exact centered formula, so
        // switching modes changed nothing (the ±1 clamp never binds for
        // visible tiles). The traversal ramp must produce different offsets.
        let vp = rect(0.0, 0.0, 800.0, 600.0);
        let tile = rect(300.0, 0.0, 200.0, 200.0); // top row of tiles
        let (_, linear) = ParallaxImage::rest_offset(ParallaxMode::Linear, &tile, &vp);
        let (_, centered) = ParallaxImage::rest_offset(ParallaxMode::Centered, &tile, &vp);
        assert!(
            (linear - centered).abs() > 0.1,
            "linear {linear} vs centered {centered}"
        );
    }

    #[test]
    fn options_disabled_zeroes_targets() {
        let mut state = ParallaxState::default();
        let img = ParallaxImage::new(
            iced::widget::image::Handle::from_rgba(4, 4, vec![0u8; 64]),
            200.0,
            200.0,
            ParallaxOptions {
                enabled: false,
                mouse: true,
                scroll: true,
                mode: ParallaxMode::Centered,
            },
        );
        img.update_target(
            &mut state,
            &rect(100.0, 100.0, 200.0, 200.0),
            mouse::Cursor::Available(iced::Point::new(150.0, 150.0)),
        );
        assert_eq!(state.target_x, 0.0);
        assert_eq!(state.target_y, 0.0);
    }
}
