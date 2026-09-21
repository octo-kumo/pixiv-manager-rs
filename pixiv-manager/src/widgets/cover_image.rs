//! A small static image widget that draws its image **cover-fit** inside a
//! fixed-size cell (the image is scaled to fill the cell and the overflow is
//! cropped). Used for avatars/profile pictures, where a plain `image` widget
//! would squish or overflow the box.

use iced::advanced::image::Image;
use iced::advanced::layout;
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::widget::Widget;
use iced::advanced::{image, Layout};
use iced::{Border, Length, Rectangle, Size};

/// Marker state (nothing to store).
#[derive(Default)]
struct CoverState;

/// A crop-to-fill static image.
pub struct CoverImage {
    handle: iced::widget::image::Handle,
    size: f32,
}

impl CoverImage {
    pub fn new(handle: iced::widget::image::Handle, size: f32) -> Self {
        Self { handle, size }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for CoverImage
where
    Renderer: iced::advanced::Renderer + image::Renderer<Handle = iced::widget::image::Handle>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<CoverState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(CoverState)
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fixed(self.size),
            height: Length::Fixed(self.size),
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
                .width(Length::Fixed(self.size))
                .height(Length::Fixed(self.size))
                .resolve(
                    Length::Fixed(self.size),
                    Length::Fixed(self.size),
                    Size::new(self.size, self.size),
                ),
        )
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: iced::mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let Some(image_size) = renderer.measure_image(&self.handle) else {
            return;
        };
        let (iw, ih) = (image_size.width as f32, image_size.height as f32);
        if iw == 0.0 || ih == 0.0 {
            return;
        }

        // Cover-fit: scale so the image fills the cell, then crop the rest.
        let scale = (bounds.width / iw).max(bounds.height / ih);
        let draw_size = Size::new(iw * scale, ih * scale);
        let drawing_bounds = Rectangle::new(
            iced::Point::new(
                bounds.x + (bounds.width - draw_size.width) / 2.0,
                bounds.y + (bounds.height - draw_size.height) / 2.0,
            ),
            draw_size,
        );

        renderer.with_layer(bounds, |renderer| {
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
                *viewport,
            );
        });
    }
}

impl<'a, Message, Theme, Renderer> From<CoverImage> for iced::Element<'a, Message, Theme, Renderer>
where
    Renderer: 'a + iced::advanced::Renderer + image::Renderer<Handle = iced::widget::image::Handle>,
{
    fn from(widget: CoverImage) -> Self {
        Self::new(widget)
    }
}

/// Build a cover-fit avatar of the given size.
pub fn cover_image(
    handle: iced::widget::image::Handle,
    size: f32,
) -> iced::Element<'static, crate::app::message::Message> {
    iced::Element::new(CoverImage::new(handle, size))
}
