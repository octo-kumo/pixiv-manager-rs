//! A floating, half-transparent debug overlay for the gallery, inspired by
//! the old Java app's `Gallery.drawDebug`: a dark translucent block with
//! monospace text and solid memory/queue bars. Toggled with F3.
//!
//! iced has no XOR blend mode for text, so the XOR look is approximated with
//! bright text over a translucent black panel (readable over any image).

use iced::widget::canvas::{self, Cache, Canvas, Frame, Program, Text};
use iced::{Element, Font, Length, Point, Rectangle, Size};
use std::cell::Cell;

/// Data shown by the overlay (snapshotted by the caller each frame).
#[derive(Debug, Clone, Default)]
pub struct DebugData {
    pub os: String,
    pub arch: String,
    pub cores: usize,
    pub mem_used_mb: f64,
    pub mem_total_mb: f64,
    pub fps: f32,
    pub gallery_items: usize,
    pub thumbnails: usize,
    pub thumb_attempts: u64,
    pub thumb_cache_hits: u64,
    pub cache_mb: f64,
    pub downloads: usize,
    pub download_speed: f64,
    pub queue_done: usize,
    pub queue_total: usize,
    pub sync_loaded: usize,
    pub sync_total: Option<usize>,
    /// Seconds remaining in the rate-limit backoff window (0 = none).
    pub rate_limit_wait: f32,
    pub version: String,
}

/// Build the debug overlay element, owning the given data.
pub fn debug_overlay(data: DebugData) -> Element<'static, crate::app::message::Message> {
    Canvas::new(Overlay::new(data))
        .width(Length::Fixed(340.0))
        .height(Length::Fixed(230.0))
        .into()
}

/// A text line with an optional solid bar next to it (memory/queue style).
struct Line {
    label: String,
    value: String,
    /// Fraction [0,1] of the bar to fill (None = no bar).
    bar: Option<f32>,
}

impl Line {
    fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            bar: None,
        }
    }
    fn with_bar(mut self, frac: f32) -> Self {
        self.bar = Some(frac.clamp(0.0, 1.0));
        self
    }
}

struct Overlay {
    data: DebugData,
    cache: Cache,
    last_frame: Cell<std::time::Instant>,
    fps: Cell<f32>,
}

impl Overlay {
    fn new(data: DebugData) -> Self {
        Self {
            data,
            cache: Cache::default(),
            last_frame: Cell::new(std::time::Instant::now()),
            fps: Cell::new(0.0),
        }
    }

    fn lines(&self) -> Vec<Line> {
        let mut lines = vec![
            Line::new("OS", format!("{} ({})", self.data.os, self.data.arch)),
            Line::new("CPU", format!("{} cores", self.data.cores)),
        ];
        let mem_frac = if self.data.mem_total_mb > 0.0 {
            (self.data.mem_used_mb / self.data.mem_total_mb) as f32
        } else {
            0.0
        };
        lines.push(
            Line::new(
                "MEM",
                format!(
                    "{:.0}/{:.0} MB",
                    self.data.mem_used_mb, self.data.mem_total_mb
                ),
            )
            .with_bar(mem_frac),
        );
        lines.push(Line::new("FPS", format!("{:.1}", self.fps.get())));
        lines.push(Line::new(
            "GALLERY",
            format!("{} items", self.data.gallery_items),
        ));
        lines.push(Line::new(
            "CACHE",
            format!(
                "{} thumbs · {:.1} MB",
                self.data.thumbnails, self.data.cache_mb
            ),
        ));
        // Network-monitoring readout: cache hits vs actual CDN fetches. A
        // high attempts:hits ratio means images are being re-downloaded.
        lines.push(Line::new(
            "THUMBS",
            format!(
                "{} fetches · {} cache hits",
                self.data.thumb_attempts, self.data.thumb_cache_hits
            ),
        ));
        let queue_frac = if self.data.queue_total > 0 {
            self.data.queue_done as f32 / self.data.queue_total as f32
        } else {
            0.0
        };
        lines.push(
            Line::new(
                "DL",
                format!(
                    "{}/{} · {:.1} MB/s",
                    self.data.queue_done, self.data.queue_total, self.data.download_speed
                ),
            )
            .with_bar(queue_frac),
        );
        let sync = match self.data.sync_total {
            Some(t) if t > 0 => format!("{}/{}", self.data.sync_loaded, t),
            _ => format!("{} loaded", self.data.sync_loaded),
        };
        lines.push(Line::new("SYNC", sync));
        if self.data.rate_limit_wait > 0.0 {
            lines.push(Line::new(
                "RATE",
                format!("⏳ waiting {:.0}s", self.data.rate_limit_wait),
            ));
        }
        lines.push(Line::new("VER", self.data.version.clone()));
        lines
    }
}

impl Program<crate::app::message::Message> for Overlay {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry<iced::Renderer>> {
        // FPS: EMA of the frame cadence.
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.last_frame.get()).as_secs_f32();
        self.last_frame.set(now);
        let frame_fps = if dt > 0.0 { 1.0 / dt } else { 0.0 };
        let fps = if self.fps.get() == 0.0 {
            frame_fps
        } else {
            self.fps.get() * 0.9 + frame_fps * 0.1
        };
        self.fps.set(fps);

        let lines = self.lines();
        let frame = self.cache.draw(renderer, bounds.size(), |frame| {
            draw_panel(frame, bounds.size(), &lines);
        });
        // The canvas geometry is drawn at its own layout position already
        // (the Canvas widget translates it), so no manual offset is needed.
        vec![frame]
    }
}

/// The old app's drawDebug style: translucent black block, monospace text,
/// solid bars.
fn draw_panel(frame: &mut Frame, size: Size, lines: &[Line]) {
    // Half-transparent background block.
    frame.fill_rectangle(
        Point::new(0.0, 0.0),
        size,
        iced::Color {
            a: 0.65,
            ..iced::Color::BLACK
        },
    );

    let font = Font::MONOSPACE;
    let mut y = 8.0;
    let line_h = 19.0;
    let bar_x = 120.0;
    let bar_width = size.width - bar_x - 16.0;

    for line in lines {
        // Solid bar behind the value when requested.
        if let Some(frac) = line.bar {
            frame.fill_rectangle(
                Point::new(bar_x, y + 3.0),
                Size::new(bar_width, 8.0),
                iced::Color::from_rgba(1.0, 1.0, 1.0, 0.35),
            );
            if frac > 0.0 {
                frame.fill_rectangle(
                    Point::new(bar_x, y + 3.0),
                    Size::new(bar_width * frac, 8.0),
                    iced::Color::from_rgb(0.9, 0.3, 0.3),
                );
            }
        }

        frame.fill_text(Text {
            content: format!("{}:  {}", line.label, line.value),
            position: Point::new(8.0, y),
            color: iced::Color::WHITE,
            size: iced::Pixels(12.0),
            font,
            ..Text::default()
        });
        y += line_h;
    }
}
