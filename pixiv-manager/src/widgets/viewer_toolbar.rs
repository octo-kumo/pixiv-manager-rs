//! The viewer's floating transform toolbar (Twitter items): combine split
//! images and rotate. Icon-only buttons with hover tooltips; icons are drawn
//! on a canvas so they match the function and the theme.

use crate::app::message::Message;
use crate::app::state::AppState;
use crate::theme;
use crate::viewer_tools::MergeAxis;
use iced::widget::canvas::{self, Path, Stroke};
use iced::{mouse, Color, Point, Rectangle, Size};

const ICON: f32 = 18.0;

#[derive(Debug, Clone, Copy)]
enum Glyph {
    /// Two pieces side by side joined at a seam.
    MergeHorizontal,
    /// Two pieces stacked and joined at a seam.
    MergeVertical,
    RotateCw,
    RotateCcw,
    /// Drop all transforms.
    Reset,
}

struct Icon {
    glyph: Glyph,
    color: Color,
}

impl<Message> canvas::Program<Message> for Icon {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let s = bounds.width.min(bounds.height) / 20.0;
        let p = |x: f32, y: f32| Point::new(x * s, y * s);
        let line = |w: f32| {
            Stroke::default()
                .with_width(w * s)
                .with_color(self.color)
                .with_line_cap(canvas::LineCap::Round)
                .with_line_join(canvas::LineJoin::Round)
        };
        match self.glyph {
            Glyph::MergeHorizontal | Glyph::MergeVertical => {
                let horizontal = matches!(self.glyph, Glyph::MergeHorizontal);
                // Two pieces that touch at a dashed seam.
                let (a, b, seam) = if horizontal {
                    (
                        (p(2.0, 4.5), Size::new(8.0 * s, 11.0 * s)),
                        (p(10.0, 4.5), Size::new(8.0 * s, 11.0 * s)),
                        (p(10.0, 3.0), p(10.0, 17.0)),
                    )
                } else {
                    (
                        (p(4.5, 2.0), Size::new(11.0 * s, 8.0 * s)),
                        (p(4.5, 10.0), Size::new(11.0 * s, 8.0 * s)),
                        (p(3.0, 10.0), p(17.0, 10.0)),
                    )
                };
                frame.stroke(&Path::rectangle(a.0, a.1), line(1.4));
                frame.stroke(&Path::rectangle(b.0, b.1), line(1.4));
                frame.stroke(&Path::line(seam.0, seam.1), line(1.0));
            }
            Glyph::RotateCw | Glyph::RotateCcw => {
                let cw = matches!(self.glyph, Glyph::RotateCw);
                let flip = |pt: Point| {
                    if cw {
                        pt
                    } else {
                        Point::new(bounds.width - pt.x, pt.y)
                    }
                };
                let center = p(10.0, 10.0);
                let radius = 6.5 * s;
                let start = -2.2f32;
                let end = start + 4.4;
                let arc = Path::new(|b| {
                    let steps = 24;
                    for i in 0..=steps {
                        let t = start + (end - start) * i as f32 / steps as f32;
                        let pt = flip(Point::new(
                            center.x + radius * t.cos(),
                            center.y + radius * t.sin(),
                        ));
                        if i == 0 {
                            b.move_to(pt);
                        } else {
                            b.line_to(pt);
                        }
                    }
                });
                frame.stroke(&arc, line(1.5));
                // Arrowhead at the end of the sweep, along the tangent.
                let tip = Point::new(center.x + radius * end.cos(), center.y + radius * end.sin());
                let (tx, ty) = (-end.sin(), end.cos());
                let head = 4.0 * s;
                let wing = |angle: f32| {
                    let (c, sn) = (angle.cos(), angle.sin());
                    // Rotate the backwards tangent by `angle`.
                    let (bx, by) = (-tx, -ty);
                    flip(Point::new(
                        tip.x + head * (bx * c - by * sn),
                        tip.y + head * (bx * sn + by * c),
                    ))
                };
                let tip = flip(tip);
                frame.stroke(
                    &Path::new(|b| {
                        b.move_to(wing(0.6));
                        b.line_to(tip);
                        b.line_to(wing(-0.6));
                    }),
                    line(1.5),
                );
            }
            Glyph::Reset => {
                frame.stroke(&Path::line(p(5.0, 5.0), p(15.0, 15.0)), line(1.6));
                frame.stroke(&Path::line(p(15.0, 5.0), p(5.0, 15.0)), line(1.6));
            }
        }
        vec![frame.into_geometry()]
    }
}

/// An icon button: `active` fills it with the accent, `hinted` outlines it.
fn tool_button<'a>(
    glyph: Glyph,
    tip: &'a str,
    msg: Option<Message>,
    active: bool,
    hinted: bool,
) -> iced::Element<'a, Message> {
    let btn = crate::ui::button(
        iced::widget::container(
            canvas::Canvas::new(Icon {
                glyph,
                color: if active {
                    Color::WHITE
                } else {
                    Color::from_rgb(0.9, 0.9, 0.93)
                },
            })
            .width(ICON)
            .height(ICON),
        )
        .center_x(26)
        .center_y(26),
    )
    .padding(0)
    .on_press_maybe(msg)
    .style(move |theme, status| {
        let mut style = crate::ui::glass(active)(theme, status);
        if hinted {
            style.border = iced::Border {
                color: theme::accent(theme),
                width: 2.0,
                radius: crate::ui::RADIUS_SM.into(),
            };
        }
        style
    });
    iced::widget::tooltip(
        btn,
        iced::widget::container(iced::widget::text(tip).size(11))
            .padding(iced::Padding::from([3, 8]))
            .style(crate::ui::tooltip),
        iced::widget::tooltip::Position::Top,
    )
    .into()
}

/// The toolbar, or `None` when the tools don't apply to the open item.
pub fn toolbar(state: &AppState) -> Option<iced::Element<'_, Message>> {
    if !crate::app::update::transform::enabled(state) || state.viewer.handle.is_none() {
        return None;
    }
    let id = state.viewer.illust_id;
    let tx = state.view_transforms.get(&id);
    let merge = tx.and_then(|t| t.merge);
    let multi = state.viewer.total_pages >= 2;
    let hint = state.viewer.split_hint.filter(|_| multi && merge.is_none());
    let has_transform = tx.is_some_and(|t| !t.is_identity());

    let mut tools: Vec<iced::Element<'_, Message>> = Vec::new();
    if multi {
        tools.push(tool_button(
            Glyph::MergeHorizontal,
            "Combine side by side (click again to undo)",
            Some(Message::ViewerMergeToggled(MergeAxis::Horizontal)),
            merge == Some(MergeAxis::Horizontal),
            hint == Some(MergeAxis::Horizontal),
        ));
        tools.push(tool_button(
            Glyph::MergeVertical,
            "Combine stacked (click again to undo)",
            Some(Message::ViewerMergeToggled(MergeAxis::Vertical)),
            merge == Some(MergeAxis::Vertical),
            hint == Some(MergeAxis::Vertical),
        ));
    }
    tools.push(tool_button(
        Glyph::RotateCcw,
        "Rotate counter-clockwise",
        Some(Message::ViewerRotate(-1)),
        false,
        false,
    ));
    tools.push(tool_button(
        Glyph::RotateCw,
        "Rotate clockwise",
        Some(Message::ViewerRotate(1)),
        false,
        false,
    ));
    if has_transform {
        tools.push(tool_button(
            Glyph::Reset,
            "Reset all changes to this post",
            Some(Message::ViewerTransformReset),
            false,
            false,
        ));
    }

    let bar = iced::widget::container(
        iced::widget::Row::with_children(tools)
            .spacing(2)
            .align_y(iced::Alignment::Center),
    )
    .padding(3)
    .style(crate::ui::glass_bar);

    let mut col = iced::widget::Column::new()
        .spacing(4)
        .align_x(iced::Alignment::End);
    if let Some(axis) = hint {
        col = col.push(
            crate::ui::button(iced::widget::text("Split image? Click to combine").size(11))
                .on_press(Message::ViewerMergeToggled(axis))
                .padding(iced::Padding::from([3, 9]))
                .style(crate::ui::primary),
        );
    }
    col = col.push(bar);
    Some(
        iced::widget::container(col)
            .align_right(iced::Length::Fill)
            .align_bottom(iced::Length::Fill)
            .padding(iced::Padding::from([34, 10]))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into(),
    )
}
