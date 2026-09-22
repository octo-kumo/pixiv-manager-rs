//! Frame pacing and frame-time telemetry.
//!
//! * **Frame times** — [`record_frame`] is called once per *rendered frame*
//!   by the root [`FrameTick`] widget (a `RedrawRequested` listener that is
//!   always part of the widget tree — iced only rebuilds `view()` when
//!   messages arrive, so the update wrapper is NOT a frame hook). The debug
//!   overlay (F3) turns this into the FPS readout, the median frame time and
//!   a rolling time chart.
//! * **FPS limit** — the app is event-driven: it redraws when the runtime
//!   publishes messages, when widgets react to events, and when our
//!   continuous *animations* (parallax springs, gallery glide, R-18 blur
//!   transition) request frames. The animation requests go through
//!   [`request_redraw_gated`]: with a limit set, the next animation frame is
//!   scheduled one frame slot after the *actually rendered* last frame
//!   (tracked by [`FrameTick`]) instead of "as soon as possible" — so every
//!   loop the app itself animates is capped at the configured FPS while
//!   event-driven redraws (input, thumbnails, loads) stay instant.

use iced::advanced::Shell;
use iced::time;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

/// Process start (the frame-time clock's origin).
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// The configured frame limit in fps (0 = uncapped).
static LIMIT_FPS: AtomicU32 = AtomicU32::new(0);

/// Millis since [`START`] when the last frame was rendered.
static LAST_FRAME_MS: AtomicU64 = AtomicU64::new(0);

/// Rolling window of intervals between rendered frames (ms), oldest first.
static FRAME_TIMES: Mutex<VecDeque<f32>> = Mutex::new(VecDeque::new());

/// How many intervals the rolling window keeps (the debug chart's span).
const WINDOW: usize = 240;

/// Apply the FPS limit from a settings value (`0` = uncapped).
pub fn set_limit(fps: u32) {
    LIMIT_FPS.store(fps.min(1000), Ordering::Relaxed);
}

/// The configured limit (`0` = uncapped).
pub fn limit_fps() -> u32 {
    LIMIT_FPS.load(Ordering::Relaxed)
}

/// Record a rendered frame (called once per `view()`).
pub fn record_frame() {
    let start = START.get_or_init(std::time::Instant::now);
    let now = start.elapsed().as_millis() as u64;
    // `swap` stores the new timestamp and returns the previous one.
    let last = LAST_FRAME_MS.swap(now, Ordering::Relaxed);
    if last == 0 {
        return; // first frame: no interval yet
    }
    if let Ok(mut times) = FRAME_TIMES.lock() {
        let dt = now.saturating_sub(last).min(u16::MAX as u64) as f32;
        times.push_back(dt);
        while times.len() > WINDOW {
            times.pop_front();
        }
    }
}

/// A snapshot of the rolling frame intervals (ms), oldest first.
pub fn frame_times() -> Vec<f32> {
    FRAME_TIMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .copied()
        .collect()
}

/// The median frame interval in the rolling window (ms).
pub fn median_frame_ms() -> f32 {
    let mut times = frame_times();
    if times.is_empty() {
        return 0.0;
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    times[times.len() / 2]
}

/// The current FPS from the rolling window (1 / mean of the recent
/// intervals; 0 before the first two frames).
pub fn fps() -> f32 {
    let times = frame_times();
    let recent: Vec<f32> = times.iter().rev().take(30).copied().collect();
    if recent.is_empty() {
        return 0.0;
    }
    let mean: f32 = recent.iter().sum::<f32>() / recent.len() as f32;
    if mean > 0.0 {
        1000.0 / mean
    } else {
        0.0
    }
}

/// The delay the next animation frame should wait for, to honor the limit:
/// the remaining time in the current frame slot, or "immediately" when the
/// frame is already late / the limit is off.
pub fn animation_gate() -> time::Duration {
    let fps = limit_fps();
    if fps == 0 {
        return time::Duration::ZERO;
    }
    let interval = (1000.0 / fps as f32).ceil() as u64;
    let start = START.get_or_init(std::time::Instant::now);
    let now_ms = start.elapsed().as_millis() as u64;
    let last = LAST_FRAME_MS.load(Ordering::Relaxed);
    let elapsed = now_ms.saturating_sub(last);
    if elapsed >= interval {
        time::Duration::from_millis(1)
    } else {
        time::Duration::from_millis(interval - elapsed)
    }
}

/// Request an *animation* redraw, scheduled through the configured fps
/// limit ("as soon as possible" when uncapped).
pub fn request_redraw_gated<Message>(shell: &mut Shell<'_, Message>) {
    if limit_fps() == 0 {
        shell.request_redraw();
    } else {
        shell.request_redraw_at(
            time::Instant::now() + animation_gate().max(time::Duration::from_millis(1)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_roundtrip() {
        set_limit(60);
        assert_eq!(limit_fps(), 60);
        set_limit(0);
        assert_eq!(limit_fps(), 0);
    }

    #[test]
    fn animation_gate_zero_when_uncapped() {
        set_limit(0);
        assert_eq!(animation_gate(), time::Duration::ZERO);
        set_limit(60);
        let gate = animation_gate();
        // The remaining time in a ~16.7ms slot is at most the interval.
        assert!(gate <= time::Duration::from_millis(17));
        set_limit(0);
    }

    #[test]
    fn median_of_recorded_frames() {
        set_limit(0);
        // Fresh window (test order may have recorded elsewhere; only assert
        // monotonic behavior, not exact values).
        let before = median_frame_ms();
        for _ in 0..5 {
            record_frame();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let after = median_frame_ms();
        assert!(after >= before && after > 0.0, "{before} → {after}");
    }
}

// ── The frame-tick widget ──────────────────────────────────────────────

/// A zero-size widget that records every rendered frame. It must sit in the
/// root widget tree at all times: iced only rebuilds `view()` when messages
/// arrive, so `update()`/`view()` wrappers are not frame hooks — but every
/// widget (including this one) receives an `Event::Window(RedrawRequested)`
/// each rendered frame. `record_frame` feeds both the debug overlay's FPS /
/// median / chart and the FPS limiter's pacing.
pub struct FrameTick;

impl<Message, Theme, Renderer> iced::advanced::Widget<Message, Theme, Renderer> for FrameTick
where
    Renderer: iced::advanced::Renderer,
{
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::stateless()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::None
    }

    fn size(&self) -> iced::Size<iced::Length> {
        iced::Size {
            width: iced::Length::Fixed(0.0),
            height: iced::Length::Fixed(0.0),
        }
    }

    fn layout(
        &mut self,
        _tree: &mut iced::advanced::widget::Tree,
        _renderer: &Renderer,
        _limits: &iced::advanced::layout::Limits,
    ) -> iced::advanced::layout::Node {
        iced::advanced::layout::Node::new(iced::Size::ZERO)
    }

    fn update(
        &mut self,
        _tree: &mut iced::advanced::widget::Tree,
        event: &iced::Event,
        _layout: iced::advanced::Layout<'_>,
        _cursor: iced::advanced::mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn iced::advanced::Clipboard,
        _shell: &mut Shell<'_, Message>,
        _viewport: &iced::Rectangle,
    ) {
        if let iced::Event::Window(iced::window::Event::RedrawRequested(_)) = event {
            record_frame();
        }
    }

    fn draw(
        &self,
        _tree: &iced::advanced::widget::Tree,
        _renderer: &mut Renderer,
        _theme: &Theme,
        _style: &iced::advanced::renderer::Style,
        _layout: iced::advanced::Layout<'_>,
        _cursor: iced::advanced::mouse::Cursor,
        _viewport: &iced::Rectangle,
    ) {
    }
}

impl<Message, Theme, Renderer> From<FrameTick> for iced::Element<'static, Message, Theme, Renderer>
where
    Renderer: iced::advanced::Renderer + 'static,
    Theme: 'static,
{
    fn from(widget: FrameTick) -> Self {
        iced::Element::new(widget)
    }
}
