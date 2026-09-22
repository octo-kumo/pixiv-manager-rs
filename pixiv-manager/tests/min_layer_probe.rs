//! Minimal in-process probe: does iced clip a `with_layer` + `with_translation`
// subtree whose children were laid out for a DIVERGENT offset (like the
// VirtualGallery's animation between view rebuilds)?

use iced::advanced::image::Renderer as ImageRenderer;
use iced::advanced::Renderer as _;
use iced::Length;
use iced_test::simulator;
use std::collections::HashMap;

/// A widget that draws an image at CONTENT y=0 inside a clip layer whose
/// translation is -offset — the image's screen position is far ABOVE the
/// clip's top edge and must be scissored away entirely.
struct DivergentLayer {
    offset: f32,
    handle: iced::widget::image::Handle,
}

impl<Message> iced::advanced::Widget<Message, iced::Theme, iced::Renderer> for DivergentLayer
where
    Message: 'static,
{
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::stateless()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::None
    }

    fn size(&self) -> iced::Size<Length> {
        iced::Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut iced::advanced::widget::Tree,
        _renderer: &iced::Renderer,
        limits: &iced::advanced::layout::Limits,
    ) -> iced::advanced::layout::Node {
        iced::advanced::layout::Node::new(limits.width(Length::Fill).height(Length::Fill).resolve(
            Length::Fill,
            Length::Fill,
            iced::Size::ZERO,
        ))
    }

    fn draw(
        &self,
        _tree: &iced::advanced::widget::Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &iced::advanced::renderer::Style,
        layout: iced::advanced::Layout<'_>,
        _cursor: iced::mouse::Cursor,
        _viewport: &iced::Rectangle,
    ) {
        let bounds = layout.bounds();
        let image_bounds = iced::Rectangle {
            x: bounds.x + 10.0,
            y: bounds.y, // content y = 0
            width: 120.0,
            height: 120.0,
        };
        let content_viewport = iced::Rectangle {
            x: bounds.x,
            y: bounds.y + self.offset,
            width: bounds.width,
            height: bounds.height,
        };
        renderer.with_layer(bounds, |renderer| {
            renderer.with_translation(iced::Vector::new(0.0, -self.offset), |renderer| {
                renderer.draw_image(
                    iced::advanced::image::Image {
                        handle: self.handle.clone(),
                        border_radius: iced::border::Radius::default(),
                        filter_method: iced::advanced::image::FilterMethod::default(),
                        rotation: iced::Radians(0.0),
                        opacity: 1.0,
                        snap: true,
                    },
                    image_bounds,
                    content_viewport,
                );
            });
        });
    }
}

impl<Message> From<DivergentLayer> for iced::Element<'static, Message, iced::Theme, iced::Renderer>
where
    Message: 'static,
{
    fn from(widget: DivergentLayer) -> Self {
        iced::Element::new(widget)
    }
}

fn red_blue() -> iced::widget::image::Handle {
    let mut pixels = vec![0u8; 64 * 64 * 4];
    for y in 0..64usize {
        for x in 0..64 {
            let i = (y * 64 + x) * 4;
            if x < 32 {
                pixels[i] = 255;
            } else {
                pixels[i + 2] = 255;
            }
            pixels[i + 3] = 255;
        }
    }
    iced::widget::image::Handle::from_bytes(pixels)
}

#[test]
fn clip_layer_scissors_translated_children() {
    let _thumbs: HashMap<u64, iced::widget::image::Handle> = HashMap::new();

    // The view: a small header strip, then the divergent layer. The header
    // occupies the top 40 logical px; the layer's clip starts below it.
    let element: iced::Element<'static, (), iced::Theme, iced::Renderer> = iced::widget::column![
        iced::widget::container(iced::widget::text("HEADER — must stay clean").size(14)).padding(6),
        DivergentLayer {
            offset: 300.0,
            handle: red_blue(),
        },
    ]
    .height(Length::Fill)
    .into();

    let mut ui = simulator(element);
    let _thumbs = &ui;
    let snapshot = ui.snapshot(&iced::Theme::Dark).expect("snapshot");

    for entry in std::fs::read_dir("target/layer_probe").unwrap().flatten() {
        let _ = std::fs::remove_file(entry.path());
    }
    snapshot
        .matches_image("target/layer_probe/min.png")
        .unwrap();
    let png = std::fs::read_dir("target/layer_probe")
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .next()
        .expect("snapshot png");
    let img = image::open(png).unwrap().to_rgba8();
    let (iw, ih) = img.dimensions();
    println!("snapshot {iw}x{ih}");

    // Count red/blue pixels ABOVE the layer's top edge. The header is the
    // first ~28 logical px; the layer fills the rest. With offset=300 the
    // image (content y 0..120) is entirely above the layer's content
    // viewport → scissored → NOT VISIBLE AT ALL.
    let mut red_rows = Vec::new();
    for y in 0..ih {
        let mut c = 0;
        for x in 0..iw {
            let p = img.get_pixel(x, y).0;
            if (p[0] > 200 && p[1] < 60 && p[2] < 60) || (p[2] > 200 && p[0] < 60 && p[1] < 60) {
                c += 1;
            }
        }
        if c > 0 {
            red_rows.push((y, c));
        }
    }
    println!(
        "red/blue rows: {} (first {:?})",
        red_rows.len(),
        red_rows.first()
    );
}
