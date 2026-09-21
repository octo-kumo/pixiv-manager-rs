//! A virtualized, smoothly-scrolling gallery.
//!
//! Replaces iced's stock `scrollable` for the main gallery because:
//!
//! * **Lazy**: only the rows currently visible (plus a buffer) are built,
//!   laid out and drawn — a 6000-item bookmark library costs as much per
//!   frame as a 60-item one. (The stock scrollable lays out every child.)
//! * **Smooth**: wheel input sets a *target* offset that the view glides
//!   toward each frame (spring decay), instead of jumping line by line.
//! * **Reports the visible window**: every time the offset moves, it emits
//!   [`Message::GalleryWindowChanged`] (the visible content range) so the
//!   update loop can lazily fetch only the thumbnails that are on screen,
//!   plus [`Message::ScrollPositionChanged`] for bottom auto-load.
//!
//! Children are pre-built rows positioned at `y = (first_row + i) * row_height`
//! inside a virtual content space; the widget clips drawing to its bounds and
//! translates by the current scroll offset (mirroring how iced's scrollable
//! presents content coordinates to children, which the parallax tiles rely
//! on). A semi-transparent scrollbar floats over the content and fades out
//! after [`SCROLLBAR_HOLD`] of no scrolling; hovering its edge or dragging
//! keeps it visible.

use crate::app::message::Message;
use iced::advanced::layout;
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::widget::Widget;
use iced::advanced::{mouse, Clipboard, Layout, Shell};
use iced::{Event, Length, Point, Rectangle, Size, Vector};
// Bring `with_layer`/`with_translation`/`fill_quad` into scope for the
// concrete renderer.
use iced::advanced::Renderer as _;

/// How fast the offset glides toward its target per frame (lower = smoother).
const SCROLL_DECAY: f32 = 0.12;
/// Stop animating when this close to the target (px).
const SCROLL_EPSILON: f32 = 0.05;
/// How close (in fractions of a row) the offset must be to a row edge before
/// it snaps to it, so the row at the top of the viewport aligns cleanly.
const SNAP_FRACTION: f32 = 0.35;
/// Floating scrollbar rail width in pixels.
const SCROLLBAR_WIDTH: f32 = 8.0;
/// Gap between the floating scrollbar and the gallery edges.
const SCROLLBAR_MARGIN: f32 = 3.0;
/// Minimum scrollbar thumb height.
const THUMB_MIN: f32 = 28.0;
/// How long the scrollbar stays fully visible after the last scroll input.
const SCROLLBAR_HOLD: std::time::Duration = std::time::Duration::from_millis(900);
/// Duration of the fade-out once the hold expires.
const SCROLLBAR_FADE_SECS: f32 = 0.45;
/// Peak opacity of the thumb — the bar is always semi-transparent.
const THUMB_OPACITY: f32 = 0.55;
/// Peak opacity of the rail behind the thumb (kept very subtle).
const RAIL_OPACITY: f32 = 0.16;

/// Scroll state stored in the widget tree (survives view rebuilds).
#[derive(Debug, Default)]
struct GalleryState {
    /// Current (animated) offset in content coordinates.
    offset: f32,
    /// Target offset; the view springs toward it.
    target: f32,
    /// Scrollbar thumb drag: the cursor offset inside the thumb when grabbed.
    drag_grab: Option<f32>,
    /// Offset we last emitted a window report for.
    last_emitted: f32,
    /// Identity of the scroll content (the tab id). When the view passes a
    /// different key the widget resets to the top — otherwise the tree keeps
    /// the previous tab's offset while the build window was already reset,
    /// which rendered an empty gallery until the user scrolled.
    scroll_key: u64,
    /// When the scrollbar last had activity (wheel/drag/hover); it fades out
    /// after [`SCROLLBAR_HOLD`] of idleness. `None` = never shown.
    scrollbar_active: Option<std::time::Instant>,
    /// Current scrollbar opacity [0, 1] (the peak is a semi-transparent bar).
    scrollbar_alpha: f32,
}

impl GalleryState {
    /// Reset to the content identity's saved offset when the identity
    /// (`key`) changed. Returns whether a reset happened.
    fn sync_scroll_key(&mut self, key: u64, restore: f32) -> bool {
        if self.scroll_key == key {
            return false;
        }
        self.scroll_key = key;
        self.offset = restore;
        self.target = restore;
        // Sentinel: forces the next settled RedrawRequested to emit a window
        // report for the restored position, so thumbnails spawn immediately.
        self.last_emitted = -1.0;
        // A new list starts with the scrollbar hidden; the first scroll shows
        // it again.
        self.scrollbar_active = None;
        self.scrollbar_alpha = 0.0;
        true
    }

    /// Mark the scrollbar as just used/hovered: fully visible again, fade
    /// clock restarted.
    fn touch_scrollbar(&mut self) {
        self.scrollbar_active = Some(std::time::Instant::now());
        self.scrollbar_alpha = 1.0;
    }

    /// Advance the fade from the wall-clock time since the last activity.
    /// Returns whether more animation frames are needed: `true` during the
    /// hold window (a fade is still pending) and while fading out.
    fn update_scrollbar_fade(&mut self) -> bool {
        let Some(last) = self.scrollbar_active else {
            self.scrollbar_alpha = 0.0;
            return false;
        };
        let idle = last.elapsed();
        if idle <= SCROLLBAR_HOLD {
            self.scrollbar_alpha = 1.0;
            return true;
        }
        self.scrollbar_alpha = scrollbar_fade(idle);
        self.scrollbar_alpha > 0.001
    }
}

/// The scrollbar opacity for `idle` time since the last activity: fully
/// visible during the hold window, then a linear fade to zero.
fn scrollbar_fade(idle: std::time::Duration) -> f32 {
    if idle <= SCROLLBAR_HOLD {
        return 1.0;
    }
    let faded = (idle - SCROLLBAR_HOLD).as_secs_f32() / SCROLLBAR_FADE_SECS;
    (1.0 - faded).clamp(0.0, 1.0)
}

/// A vertically scrolling, virtualized grid of pre-built rows.
pub struct VirtualGallery<'a> {
    /// The visible rows (pre-built by the caller).
    rows: Vec<iced::Element<'a, Message>>,
    /// Index (into the full row list) of the first row in `rows`.
    first_row: usize,
    /// Total number of rows in the full content.
    total_rows: usize,
    /// Height of a single row (= tile size; rows are square).
    row_height: f32,
    /// Identity of the scroll content (the active tab) — changing it resets
    /// the scroll to the saved per-tab offset.
    scroll_key: u64,
    /// The offset to restore when the scroll identity changes (the tab's
    /// saved position). Reaching a new tab for the first time passes 0.
    restore_offset: f32,
}

impl<'a> VirtualGallery<'a> {
    /// Creates the gallery. The caller builds only the visible rows (the
    /// window around the current scroll offset) and hands them in together
    /// with their position and the total content size. `scroll_key` identifies
    /// the scroll content (the active tab); switching tabs resets to
    /// `restore_offset` (the tab's saved position — 0 on first visit).
    pub fn new(
        rows: Vec<iced::Element<'a, Message>>,
        first_row: usize,
        total_rows: usize,
        row_height: f32,
        scroll_key: u64,
        restore_offset: f32,
    ) -> Self {
        Self {
            rows,
            first_row,
            total_rows,
            row_height,
            scroll_key,
            restore_offset,
        }
    }

    fn content_height(&self) -> f32 {
        self.total_rows as f32 * self.row_height
    }

    /// A wheel notch scrolls exactly half a row (fine-grained stepping), then
    /// snaps to the nearest row edge when the result is close enough to one,
    /// so the row at the top of the viewport aligns cleanly.
    fn wheel_target(&self, current: f32, delta_y: f32, max_offset: f32) -> f32 {
        let bump = self.row_height * 0.5;
        let t = (current - delta_y * bump).clamp(0.0, max_offset);
        Self::snap_to_row(t, self.row_height, SNAP_FRACTION, max_offset)
    }

    /// Snap `offset` to the nearest row boundary (a `row_height` multiple)
    /// when it is within `fraction * row_height` of one; otherwise leave it
    /// untouched. Clamped to `[0, max_offset]`.
    fn snap_to_row(offset: f32, row_height: f32, fraction: f32, max_offset: f32) -> f32 {
        if row_height <= 0.0 {
            return offset.clamp(0.0, max_offset);
        }
        let nearest = (offset / row_height).round() * row_height;
        let snapped = if (offset - nearest).abs() <= row_height * fraction {
            nearest
        } else {
            offset
        };
        snapped.clamp(0.0, max_offset)
    }

    /// Floating scrollbar geometry `(thumb_y, thumb_h)` for the current
    /// (animated) offset; `None` when the content fits in the viewport.
    fn thumb(&self, state: &GalleryState, bounds: &Rectangle) -> Option<(f32, f32)> {
        let content_h = self.content_height();
        if content_h <= bounds.height {
            return None;
        }
        let track_top = bounds.y + SCROLLBAR_MARGIN;
        let track_h = (bounds.height - SCROLLBAR_MARGIN * 2.0).max(1.0);
        let thumb_h = (track_h * bounds.height / content_h)
            .max(THUMB_MIN)
            .min(track_h);
        let max_offset = content_h - bounds.height;
        let y = track_top + (state.offset / max_offset).clamp(0.0, 1.0) * (track_h - thumb_h);
        Some((y, thumb_h))
    }

    /// The rail's x coordinate (left edge) for `bounds`.
    fn rail_x(bounds: &Rectangle) -> f32 {
        bounds.x + bounds.width - SCROLLBAR_WIDTH - SCROLLBAR_MARGIN
    }
}

impl Widget<Message, iced::Theme, iced::Renderer> for VirtualGallery<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<GalleryState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(GalleryState::default())
    }

    fn children(&self) -> Vec<Tree> {
        self.rows.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.rows);
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        // Content identity change (tab switch): the tree may have preserved
        // this widget with the previous tab's offset while the row-build
        // window was already reset — restore the saved offset so the two
        // agree (returning to a tab restores its position).
        tree.state
            .downcast_mut::<GalleryState>()
            .sync_scroll_key(self.scroll_key, self.restore_offset);

        let size = limits.width(Length::Fill).height(Length::Fill).resolve(
            Length::Fill,
            Length::Fill,
            Size::ZERO,
        );

        let mut children_nodes = Vec::with_capacity(self.rows.len());
        for (i, row) in self.rows.iter_mut().enumerate() {
            let child = &mut tree.children[i];
            let mut node = row.as_widget_mut().layout(child, renderer, limits);
            // Rows start at the content top (row_height per row).
            node.move_to_mut(Point::new(
                0.0,
                (self.first_row + i) as f32 * self.row_height,
            ));
            children_nodes.push(node);
        }

        layout::Node::with_children(size, children_nodes)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        operation.traverse(&mut |operation| {
            for (i, row) in self.rows.iter_mut().enumerate() {
                let child = &mut tree.children[i];
                let child_layout = layout
                    .children()
                    .nth(i)
                    .expect("virtual gallery child layout");
                row.as_widget_mut()
                    .operate(child, child_layout, renderer, operation);
            }
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<GalleryState>();
        let max_offset = (self.content_height() - bounds.height).max(0.0);
        let over = cursor.position_over(bounds);

        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if over.is_none() || max_offset == 0.0 {
                    return;
                }
                let delta_y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 60.0,
                };
                state.target = self.wheel_target(state.target, delta_y, max_offset);
                state.touch_scrollbar();
                // Report the projected window immediately (prefetch-ahead);
                // the rendered offset catches up smoothly.
                self.report(shell, state.target, bounds.height);
                state.last_emitted = state.target;
                shell.request_redraw();
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                // Only grab the bar while it is visible; otherwise a click on
                // the gallery edge would jump the list invisibly.
                if let (Some(cursor), Some((thumb_y, thumb_h))) = (over, self.thumb(state, &bounds))
                {
                    if state.scrollbar_alpha > 0.02
                        && cursor.x >= Self::rail_x(&bounds)
                        && cursor.x <= bounds.x + bounds.width
                    {
                        // Click on the rail: jump the thumb, then drag.
                        let track_top = bounds.y + SCROLLBAR_MARGIN;
                        let track = (bounds.height - SCROLLBAR_MARGIN * 2.0 - thumb_h).max(1.0);
                        let grab = (cursor.y - thumb_y).clamp(0.0, thumb_h);
                        state.drag_grab = Some(grab);
                        let max_offset = (self.content_height() - bounds.height).max(0.0);
                        state.target = ((cursor.y - track_top - grab) / track * max_offset)
                            .clamp(0.0, max_offset);
                        state.touch_scrollbar();
                        self.report(shell, state.target, bounds.height);
                        state.last_emitted = state.target;
                        shell.capture_event();
                        shell.request_redraw();
                    }
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(grab) = state.drag_grab {
                    if let Some(cursor) = cursor.position() {
                        let (_, thumb_h) = self.thumb(state, &bounds).unwrap_or((0.0, THUMB_MIN));
                        let track_top = bounds.y + SCROLLBAR_MARGIN;
                        let track = (bounds.height - SCROLLBAR_MARGIN * 2.0 - thumb_h).max(1.0);
                        state.target = ((cursor.y - track_top - grab) / track * max_offset)
                            .clamp(0.0, max_offset);
                        state.touch_scrollbar();
                        self.report(shell, state.target, bounds.height);
                        state.last_emitted = state.target;
                        shell.capture_event();
                        shell.request_redraw();
                    }
                } else if let Some(cursor) = cursor.position() {
                    // Moving along the bar's edge reveals it and keeps the
                    // fade clock reset while the cursor keeps moving.
                    let hover_rail = over.is_some()
                        && cursor.x >= Self::rail_x(&bounds)
                        && self.thumb(state, &bounds).is_some();
                    if hover_rail {
                        let was_visible = state.scrollbar_alpha > 0.001;
                        state.touch_scrollbar();
                        if !was_visible {
                            shell.request_redraw();
                        }
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag_grab = None;
            }
            Event::Window(iced::window::Event::RedrawRequested(_)) => {
                // Clamp (content may have shrunk) and glide toward the target.
                state.target = state.target.clamp(0.0, max_offset);
                state.offset += (state.target - state.offset) * SCROLL_DECAY;
                if (state.offset - state.target).abs() < SCROLL_EPSILON {
                    state.offset = state.target;
                }
                // Report ONLY when the view settles (offset == target) and the
                // settled position differs from the last reported one. Wheel
                // input already reports the projected target, so reporting the
                // animated offset every frame would spam a `GalleryWindowChanged`
                // message per frame → each message rebuilds the view → layout
                // invalidation on every RedrawRequested (iced_winit warns).
                if state.offset == state.target && state.last_emitted != state.target {
                    state.last_emitted = state.target;
                    self.report(shell, state.target, bounds.height);
                }
                // Fade the scrollbar out after the hold window; keep animating
                // until it is fully transparent.
                let fading = state.update_scrollbar_fade();
                if state.offset != state.target || fading {
                    shell.request_redraw();
                }
            }
            _ => {}
        }

        // Children (rows) receive events with the cursor in CONTENT
        // coordinates — the same translation iced's scrollable applies, and
        // the parallax tiles depend on it.
        let translated_cursor = match over {
            Some(position) => mouse::Cursor::Available(position + Vector::new(0.0, state.offset)),
            None => mouse::Cursor::Unavailable,
        };
        for (i, row) in self.rows.iter_mut().enumerate() {
            let child = &mut tree.children[i];
            let child_layout = layout
                .children()
                .nth(i)
                .expect("virtual gallery child layout");
            row.as_widget_mut().update(
                child,
                event,
                child_layout,
                translated_cursor,
                _renderer,
                _clipboard,
                shell,
                &Rectangle {
                    x: bounds.x,
                    y: bounds.y + state.offset,
                    width: bounds.width,
                    height: bounds.height,
                },
            );
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_ref::<GalleryState>();

        // Resize guard: a degenerate viewport (window collapsed mid-resize)
        // must not reach the renderer — zero-area layers can panic wgpu.
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return;
        }

        // Clip to the viewport and translate children by the scroll offset.
        renderer.with_layer(bounds, |renderer| {
            renderer.with_translation(Vector::new(0.0, -state.offset), |renderer| {
                let content_viewport = Rectangle {
                    x: bounds.x,
                    y: bounds.y + state.offset,
                    width: bounds.width,
                    height: bounds.height,
                };
                for (i, row) in self.rows.iter().enumerate() {
                    let child = &tree.children[i];
                    let child_layout = layout
                        .children()
                        .nth(i)
                        .expect("virtual gallery child layout");
                    row.as_widget().draw(
                        child,
                        renderer,
                        theme,
                        style,
                        child_layout,
                        cursor,
                        &content_viewport,
                    );
                }
            });
        });

        // Floating scrollbar: always semi-transparent and faded out while
        // idle (`GalleryState::update_scrollbar_fade`).
        //
        // It gets its OWN layer: iced_wgpu records the rows inside the
        // `with_layer` above, which creates a higher-index layer; anything
        // drawn into this widget's parent layer afterwards is composited
        // UNDER those layers. A fresh `with_layer` here is what puts the
        // scrollbar on top of the tiles.
        let alpha = state.scrollbar_alpha;
        if alpha > 0.001 {
            if let Some((thumb_y, thumb_h)) = self.thumb(state, &bounds) {
                let rail_x = Self::rail_x(&bounds);
                let track_top = bounds.y + SCROLLBAR_MARGIN;
                let track_h = (bounds.height - SCROLLBAR_MARGIN * 2.0).max(1.0);
                let radius = (SCROLLBAR_WIDTH / 2.0).into();
                let dragging = state.drag_grab.is_some();
                let thumb_color = if dragging {
                    crate::theme::accent(theme).scale_alpha((THUMB_OPACITY + 0.25) * alpha)
                } else {
                    crate::theme::fg(theme).scale_alpha(THUMB_OPACITY * alpha)
                };
                let rail_color = crate::theme::fg(theme).scale_alpha(RAIL_OPACITY * alpha);
                renderer.with_layer(bounds, |renderer| {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(
                                Point::new(rail_x, track_top),
                                Size::new(SCROLLBAR_WIDTH, track_h),
                            ),
                            border: iced::Border {
                                radius,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        rail_color,
                    );
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(
                                Point::new(rail_x, thumb_y),
                                Size::new(SCROLLBAR_WIDTH, thumb_h),
                            ),
                            border: iced::Border {
                                radius,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        thumb_color,
                    );
                });
            }
        }
        let _ = viewport;
        let _ = cursor;
    }
}

impl<'a> From<VirtualGallery<'a>> for iced::Element<'a, Message, iced::Theme, iced::Renderer> {
    fn from(widget: VirtualGallery<'a>) -> Self {
        Self::new(widget)
    }
}

impl VirtualGallery<'_> {
    /// Emit the scroll reports for `offset` (projected target or current
    /// animated offset): the visible content window (for lazy thumbnail
    /// loading) and the relative offset (for bottom auto-load).
    fn report(&self, shell: &mut Shell<'_, Message>, offset: f32, viewport_h: f32) {
        let start = offset;
        let end = offset + viewport_h;
        shell.publish(Message::GalleryWindowChanged { start, end });
        let max = (self.content_height() - viewport_h).max(0.0);
        let relative = if max <= 0.0 {
            0.0
        } else {
            (offset / max).clamp(0.0, 1.0)
        };
        shell.publish(Message::ScrollPositionChanged(relative > 0.85));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::widget::tree::State as TreeState;

    fn state_mut<T: 'static>(state: &mut TreeState) -> Option<&mut T> {
        match state {
            TreeState::Some(any) => any.downcast_mut::<T>(),
            TreeState::None => None,
        }
    }

    fn state_ref<T: 'static>(state: &TreeState) -> Option<&T> {
        match state {
            TreeState::Some(any) => any.downcast_ref::<T>(),
            TreeState::None => None,
        }
    }

    /// Depth-first search for the `VirtualGallery`'s tree node (matches by its
    /// state type), so the test doesn't depend on the surrounding view layout.
    fn find_gallery_state_mut(tree: &mut Tree) -> Option<&mut GalleryState> {
        if let Some(state) = state_mut(&mut tree.state) {
            return Some(state);
        }
        for child in &mut tree.children {
            if let Some(state) = find_gallery_state_mut(child) {
                return Some(state);
            }
        }
        None
    }

    fn find_gallery_offset(tree: &Tree) -> Option<f32> {
        if let Some(state) = state_ref::<GalleryState>(&tree.state) {
            return Some(state.offset);
        }
        for child in &tree.children {
            if let Some(offset) = find_gallery_offset(child) {
                return Some(offset);
            }
        }
        None
    }

    /// Regression test for the parallax-flash / lost-scroll-position bugs.
    ///
    /// `view()` must ALWAYS return a Stack whose first child is the gallery, so
    /// iced's tree diff (which reconciles children by index and recreates the
    /// whole tree only when the root widget type changes) preserves the
    /// gallery's internal state — scroll offset, parallax springs, scrollbar
    /// drag — across opening/closing the viewer, artist, and settings overlays.
    /// Before the fix the root flipped between Column and Stack, which
    /// recreated the tree: the scroll reset to the top and every parallax tile
    /// sprang from zero.
    #[test]
    fn gallery_tree_state_survives_overlay_toggles() {
        use crate::app::state::{AppPhase, AppState};
        use crate::app::view::view;

        let mut state = AppState {
            phase: AppPhase::Main,
            window_size: (1600.0, 900.0),
            ..AppState::default()
        };
        // A non-empty gallery so `view()` builds a VirtualGallery (the empty
        // state renders a plain "No illustrations" container instead).
        state.tabs.recommend.illustrations = (1..=20).map(crate::test_util::mock_illust).collect();
        state.tabs.recommend.filtered = state.tabs.recommend.illustrations.clone();

        let el = view(&state);
        let mut tree = Tree::new(&el);
        drop(el);

        // Simulate a scrolled gallery (offset + target at 2500px).
        let gallery = find_gallery_state_mut(&mut tree).expect("gallery node in tree");
        gallery.offset = 2500.0;
        gallery.target = 2500.0;
        assert_eq!(find_gallery_offset(&tree), Some(2500.0));

        // Open the viewer overlay, then the settings screen on top.
        state.viewer.open = true;
        let el = view(&state);
        tree.diff(&el);
        drop(el);
        assert_eq!(
            find_gallery_offset(&tree),
            Some(2500.0),
            "offset survives viewer open"
        );

        state.settings_open = true;
        let el = view(&state);
        tree.diff(&el);
        drop(el);
        assert_eq!(
            find_gallery_offset(&tree),
            Some(2500.0),
            "offset survives settings open"
        );

        // Close everything again.
        state.viewer.open = false;
        state.settings_open = false;
        let el = view(&state);
        tree.diff(&el);
        drop(el);
        assert_eq!(
            find_gallery_offset(&tree),
            Some(2500.0),
            "offset survives closing all overlays"
        );
    }

    #[test]
    fn snap_to_row_boundary_when_close() {
        // 320px is 3.2 rows, within 0.35 of row 3 (300px) → snaps to 300.
        assert_eq!(
            VirtualGallery::snap_to_row(320.0, 100.0, 0.35, 10000.0),
            300.0
        );
        // 270px is 2.7 rows, within 0.35 of row 3 (300px) too.
        assert_eq!(
            VirtualGallery::snap_to_row(270.0, 100.0, 0.35, 10000.0),
            300.0
        );
    }

    #[test]
    fn snap_leaves_half_row_position_alone() {
        // 250px is exactly half a row away from both 200 and 300 → no snap.
        assert_eq!(
            VirtualGallery::snap_to_row(250.0, 100.0, 0.35, 10000.0),
            250.0
        );
    }

    #[test]
    fn snap_clamps_to_max_offset() {
        assert_eq!(
            VirtualGallery::snap_to_row(9999.0, 100.0, 0.35, 950.0),
            950.0
        );
    }

    #[test]
    fn wheel_notch_is_half_a_row() {
        // One wheel notch moves the target by half a row and stays put when
        // it is exactly between two boundaries.
        let vg = VirtualGallery::new(Vec::new(), 0, 100, 100.0, 1, 0.0);
        assert_eq!(vg.wheel_target(0.0, -1.0, 99999.0), 50.0);
        // Two notches land exactly on a row boundary.
        assert_eq!(vg.wheel_target(50.0, -1.0, 99999.0), 100.0);
    }

    #[test]
    fn wheel_clamps_to_max_offset() {
        let vg = VirtualGallery::new(Vec::new(), 0, 10, 100.0, 1, 0.0);
        // Scrolling down past the end clamps to max_offset (940px is 9.4
        // rows, far enough from the 900 boundary that no snap overrides it).
        assert_eq!(vg.wheel_target(915.0, -1.0, 940.0), 940.0);
        // Scrolling up past the top clamps to 0.
        assert_eq!(vg.wheel_target(30.0, 1.0, 920.0), 0.0);
    }

    #[test]
    fn scroll_key_change_restores_the_saved_offset() {
        // Regression: switching tabs used to reset the scroll to the top,
        // destroying the tab's position. Now the tab's saved offset is
        // restored (0 on a first visit — the old behavior).
        let mut state = GalleryState {
            scroll_key: 1,
            offset: 3000.0,
            target: 3000.0,
            last_emitted: 3000.0,
            ..GalleryState::default()
        };

        // Same key: no reset (scroll survives re-renders within a tab).
        assert!(!state.sync_scroll_key(1, 0.0));
        assert_eq!(state.offset, 3000.0);

        // New content identity (tab switch): restore the saved offset (0 =
        // a first visit resets to the top), and the sentinel forces a
        // window report so thumbnails spawn immediately.
        assert!(state.sync_scroll_key(2, 0.0));
        assert_eq!(state.offset, 0.0);
        assert_eq!(state.target, 0.0);
        assert_eq!(state.last_emitted, -1.0);

        // Returning to the first tab restores its saved position.
        assert!(state.sync_scroll_key(1, 2500.0));
        assert_eq!(state.offset, 2500.0);
        assert_eq!(state.target, 2500.0);
    }

    #[test]
    fn scrollbar_fades_out_after_the_hold_window() {
        use std::time::Duration;

        assert_eq!(scrollbar_fade(Duration::ZERO), 1.0);
        assert_eq!(scrollbar_fade(SCROLLBAR_HOLD), 1.0);
        // Midway through the fade the opacity is about half.
        let mid = SCROLLBAR_HOLD + Duration::from_secs_f32(SCROLLBAR_FADE_SECS / 2.0);
        let alpha = scrollbar_fade(mid);
        assert!((alpha - 0.5).abs() < 0.05, "got {alpha}");
        // Fully transparent after a full fade.
        let end = SCROLLBAR_HOLD + Duration::from_secs_f32(SCROLLBAR_FADE_SECS);
        assert_eq!(scrollbar_fade(end), 0.0);
    }

    #[test]
    fn scrollbar_touch_shows_it_and_restarts_the_fade() {
        let mut state = GalleryState::default();
        state.touch_scrollbar();
        assert_eq!(state.scrollbar_alpha, 1.0);
        assert!(state.scrollbar_active.is_some());
        // Fresh activity: visible, and frames are still needed because the
        // fade is pending.
        assert!(state.update_scrollbar_fade());
        assert_eq!(state.scrollbar_alpha, 1.0);

        // After the hold window elapses it starts fading (and asks for
        // redraws until fully faded).
        state.scrollbar_active = Some(
            std::time::Instant::now()
                - SCROLLBAR_HOLD
                - std::time::Duration::from_secs_f32(SCROLLBAR_FADE_SECS / 2.0),
        );
        assert!(state.update_scrollbar_fade(), "fade needs animation frames");
        assert!((state.scrollbar_alpha - 0.5).abs() < 0.1);

        // Fully faded: the animation stops.
        state.scrollbar_active = Some(
            std::time::Instant::now()
                - SCROLLBAR_HOLD
                - std::time::Duration::from_secs_f32(SCROLLBAR_FADE_SECS * 2.0),
        );
        assert!(!state.update_scrollbar_fade());
        assert_eq!(state.scrollbar_alpha, 0.0);
    }

    #[test]
    fn thumb_geometry_tracks_the_offset_inside_the_inset_track() {
        let vg = VirtualGallery::new(Vec::new(), 0, 100, 100.0, 1, 0.0);
        let bounds = Rectangle::new(Point::new(0.0, 0.0), Size::new(800.0, 400.0));
        let mut state = GalleryState::default();

        let (y_top, thumb_h) = vg.thumb(&state, &bounds).expect("content overflows");
        assert!(
            (y_top - SCROLLBAR_MARGIN).abs() < 0.01,
            "starts at the inset top, got {y_top}"
        );
        assert!(thumb_h >= THUMB_MIN);

        // At the bottom the thumb ends at the inset bottom.
        state.offset = vg.content_height() - bounds.height;
        let (y_bottom, thumb_h2) = vg.thumb(&state, &bounds).unwrap();
        assert_eq!(thumb_h, thumb_h2);
        let expected = bounds.height - SCROLLBAR_MARGIN - thumb_h;
        assert!(
            (y_bottom - expected).abs() < 0.01,
            "got {y_bottom}, want {expected}"
        );

        // Content that fits → no scrollbar.
        let short = VirtualGallery::new(Vec::new(), 0, 2, 100.0, 1, 0.0);
        assert!(short.thumb(&state, &bounds).is_none());
    }
}
