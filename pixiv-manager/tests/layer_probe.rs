//! Pixel probes for the gallery's clipping contract: render the app's real
//! widget stack (`view()`) with synthetic, uniquely-coloured thumbnails and
//! check — via rendered pixels — that no tile content is painted outside the
//! intersection of the gallery's own bounds and the parent viewport.
//!
//! Everything is derived from the layout at runtime (the `VirtualGallery`
//! draw probe reports its bounds and incoming viewport); no platform's assumed
//! pixel geometry is hard-coded. Palette pixels are matched exactly, so
//! ordinary UI colours (for example the R-18 badge's `(230, 51, 51)`) cannot
//! be mistaken for tile content.

use pixiv_manager::app::message::Message;
use pixiv_manager::app::state::{AppPhase, AppState, GalleryTab, Tab};
use pixiv_manager::app::view::view;
use pixiv_manager::widgets::virtual_gallery;
use pixivj_rs::model::*;

fn illust(id: u64, r18: bool) -> Illustration {
    Illustration {
        id,
        title: Some(format!("item {id}")),
        illust_type: None,
        image_urls: Some(IllustImageUrls {
            square_medium: Some("t.jpg".into()),
            medium: Some("t.jpg".into()),
            large: Some("l.jpg".into()),
        }),
        caption: None,
        restrict: if r18 { Some(1) } else { Some(0) },
        user: Some(Author {
            id: 42,
            profile_image_urls: None,
            name: "probeartist".into(),
            account: None,
            comment: None,
            is_followed: None,
        }),
        tags: vec![],
        tools: vec![],
        create_date: None,
        page_count: Some(1),
        width: Some(800),
        height: Some(600),
        sanity_level: None,
        x_restrict: if r18 { Some(1) } else { None },
        series: None,
        meta_single_page: None,
        meta_pages: vec![],
        total_view: Some(10),
        total_bookmarks: Some(5),
        is_bookmarked: Some(false),
        visible: Some(true),
        is_muted: None,
        illust_ai_type: None,
        provider_data: None,
    }
}

struct Probe {
    state: AppState,
}

/// The logical window size `iced_test::simulator` lays out at (its default).
/// The app state must agree with it: `view()` derives the grid from
/// `AppState::window_size`, not from the simulator's size.
const WINDOW_SIZE: (f32, f32) = (1024.0, 768.0);

impl Probe {
    fn new() -> Self {
        let config_dir = std::env::temp_dir().join("pixiv_manager_layer_probe_config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::env::set_var("PIXIV_MANAGER_CONFIG_DIR", &config_dir);
        let mut state = AppState {
            phase: AppPhase::Main,
            window_size: WINDOW_SIZE,
            ..Default::default()
        };
        state.settings.grid_size = 350;
        state.settings.blur_r18 = false; // parallax-only variant
        state.active_tab = Tab::Recommend;

        // 6×3 tiles; even ids are R-18 (blurred, red/blue), odd ids normal
        // (green/blue). Id 1 has no thumbnail (grey placeholder).
        let mut illusts = Vec::new();
        for id in 1..=18u64 {
            illusts.push(illust(id, id % 2 == 0));
        }
        let red_blue = split_rgba([255, 0, 0], [0, 0, 255]);
        let green_blue = split_rgba([0, 255, 0], [0, 0, 255]);
        let mut handles = std::collections::HashMap::new();
        for id in 2..=18u64 {
            let pixels = if id % 2 == 0 {
                red_blue.clone()
            } else {
                green_blue.clone()
            };
            // RGBA handles are uploaded synchronously (unlike encoded `Bytes`,
            // which iced decodes on a worker thread): the first snapshot draw
            // below must already contain the synthetic tile pixels.
            handles.insert(id, iced::widget::image::Handle::from_rgba(64, 64, pixels));
        }
        state.images.thumbnail_handles = handles;
        state.tabs.recommend = GalleryTab {
            illustrations: illusts,
            ..Default::default()
        };
        state.tabs.recommend.filtered = state.tabs.recommend.illustrations.clone();
        state.gallery_window = Some((0.0, 900.0));

        Self { state }
    }

    /// The same UI with every synthetic thumbnail removed (the tile overlays
    /// stay): the control render used to classify the offending pixels.
    fn without_tiles(&self) -> Self {
        let mut state = self.state.clone();
        state.images.thumbnail_handles.clear();
        Self { state }
    }
}

fn split_rgba(left: [u8; 3], right: [u8; 3]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(64 * 64 * 4);
    for _y in 0..64 {
        for x in 0..64 {
            let c = if x < 32 { left } else { right };
            pixels.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    pixels
}

/// The synthetic tile palette. The left half of each thumbnail is one of these,
/// the right half is always [`BLUE`]. They are deliberately full-intensity
/// primaries that no theme colour reproduces, so a palette pixel can only come
/// from a gallery tile image.
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

/// The palette, with the tiles that use it (for tracing a pixel to a tile).
fn palette() -> [(&'static str, [u8; 3], &'static str); 3] {
    [
        ("red", RED, "even ids 2..=18 (left half)"),
        ("green", GREEN, "odd ids 3..=17 (left half)"),
        ("blue", BLUE, "every tile 2..=18 (right half)"),
    ]
}

/// Returns the palette name when `p` is a *full-opacity* synthetic tile pixel.
///
/// Tolerant only to a few LSBs (the thumbnails are upscaled, but interior
/// texels stay exact; bilinear filtering at the half-way seam blends RED/BLUE
/// and GREEN/BLUE into colours no palette entry matches). Deliberately strict
/// on saturation: the R-18 badge background is `[0.9, 0.2, 0.2]` ≈
/// `(230, 51, 51)`, ordinary UI painted by the tile overlay — NOT tile content.
fn tile_color(p: &[u8; 4]) -> Option<&'static str> {
    if p[3] < 250 {
        return None;
    }
    palette().iter().find_map(|(name, c, _)| {
        let near = p[0].abs_diff(c[0]) <= 8 && p[1].abs_diff(c[1]) <= 8 && p[2].abs_diff(c[2]) <= 8;
        near.then_some(*name)
    })
}

/// One rendered frame of the scrolled app plus the gallery draw probe recorded
/// by that same frame.
struct Render {
    image: image::RgbaImage,
    probe: virtual_gallery::DrawProbe,
    backend: String,
    png: std::path::PathBuf,
}

/// Renders the app scrolled to a mid-glide offset.
///
/// `parent_clip_bottom` places the whole app view inside a parent that only
/// exposes its top `h` logical pixels (a scrollable viewport) while the app
/// lays itself out in a taller box — the gallery then extends past the parent
/// viewport and must not paint below it.
fn scrolled_render(state: &Probe, directory: &str, parent_clip_bottom: Option<f32>) -> Render {
    let app_view = view(&state.state);
    let element: iced::Element<'_, Message> = match parent_clip_bottom {
        None => app_view,
        // A scrollable whose content is taller than its viewport forwards a
        // smaller viewport to the app view (and creates its own layer). The
        // gallery's layout still extends below that viewport, which is exactly
        // the "parent viewport is smaller than my bounds" case.
        Some(bottom) => iced::widget::scrollable(
            iced::widget::container(app_view)
                .width(iced::Length::Fill)
                .height(iced::Length::Fixed(900.0)),
        )
        .width(iced::Length::Fill)
        .height(iced::Length::Fixed(bottom))
        .into(),
    };
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(WINDOW_SIZE.0, WINDOW_SIZE.1),
        element,
    );
    ui.point_at(iced::Point::new(600.0, 300.0));

    let wheel = |y: f32| {
        iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y },
        })
    };
    let redraw = || {
        iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ))
    };
    for _ in 0..2 {
        ui.simulate([redraw()]);
        ui.simulate([wheel(-3.0)]);
        ui.simulate([redraw()]);
    }
    // Settle on a row boundary (offset 1050 = 3 rows), so the next notch
    // starts from a clean, aligned position.
    for _ in 0..80 {
        ui.simulate([redraw()]);
    }
    // One more notch, then snapshot MID-GLIDE: the animated offset (~1260,
    // not a multiple of the row height) leaves the third row straddling the
    // gallery's top edge — exactly the state where tile content must be
    // clipped away instead of painting over the toolbar.
    ui.simulate([wheel(-3.0)]);
    for _ in 0..4 {
        ui.simulate([redraw()]);
    }

    // Probe artifacts go to the OS temp directory: a test must never write
    // into the source tree (and never depend on a Unix-only path literal).
    let probe_dir = std::env::temp_dir()
        .join("pixiv-manager-layer-probe")
        .join(directory);
    std::fs::create_dir_all(&probe_dir).unwrap();
    for entry in std::fs::read_dir(&probe_dir).unwrap().flatten() {
        let _ = std::fs::remove_file(entry.path());
    }
    let snapshot = ui.snapshot(&iced::Theme::Dark).expect("snapshot");
    snapshot.matches_image(probe_dir.join("shot.png")).unwrap();
    let png = std::fs::read_dir(&probe_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .next()
        .expect("snapshot png");
    let backend = png
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let image = image::open(&png).unwrap().to_rgba8();
    let probe = virtual_gallery::draw_probe().expect("gallery recorded a draw probe");
    Render {
        image,
        probe,
        backend,
        png,
    }
}

/// The clip the invariant requires: the gallery's own bounds intersected with
/// the viewport iced handed it. Derived from the probe's independent `bounds`
/// and `viewport` fields so the test still catches a `draw()` that fails to
/// apply the intersection.
fn expected_clip(render: &Render) -> Option<iced::Rectangle> {
    render.probe.bounds.intersection(&render.probe.viewport)
}

/// The fully-outside physical pixel ranges for `clip`, as
/// `(left, top, right, bottom)` exclusive bounds. A pixel that straddles the
/// clip edge is *not* counted: only whole pixels outside the region prove a
/// leak, which keeps the assertion independent of clip-edge rounding.
fn outside_bounds(clip: iced::Rectangle, scale: f32, (iw, ih): (u32, u32)) -> (f32, f32, f32, f32) {
    (
        (clip.x * scale).floor().max(0.0),
        (clip.y * scale).floor().max(0.0),
        ((clip.x + clip.width) * scale).ceil().min(iw as f32),
        ((clip.y + clip.height) * scale).ceil().min(ih as f32),
    )
}

/// One palette pixel that lies fully outside the gallery's required clip.
struct Offender {
    x: u32,
    y: u32,
    rgba: [u8; 4],
    tile: &'static str,
    edge: &'static str,
}

/// Every full-opacity synthetic tile pixel outside `clip`, with its
/// coordinates/RGBA, palette match and the edge it escaped through.
fn offenders_outside(image: &image::RgbaImage, clip: iced::Rectangle, scale: f32) -> Vec<Offender> {
    let (iw, ih) = image.dimensions();
    let (left, top, right, bottom) = outside_bounds(clip, scale, (iw, ih));
    let mut out = Vec::new();
    for y in 0..ih {
        for x in 0..iw {
            let p = image.get_pixel(x, y).0;
            let (xf, yf) = (x as f32, y as f32);
            let edge = if yf + 1.0 <= top {
                "above"
            } else if yf >= bottom {
                "below"
            } else if xf + 1.0 <= left {
                "left"
            } else if xf >= right {
                "right"
            } else {
                continue;
            };
            if let Some(tile) = tile_color(&p) {
                out.push(Offender {
                    x,
                    y,
                    rgba: p,
                    tile,
                    edge,
                });
            }
        }
    }
    out
}

/// Renders, checks the required clip rectangle against the one the widget
/// actually used, and asserts that no tile content escaped it. On failure it
/// classifies the pixels with a control render that has no tile images.
fn assert_gallery_clip(
    probe: &Probe,
    directory: &str,
    parent_clip_bottom: Option<f32>,
    scenario: &str,
) {
    // Headless wgpu renderers must not be created/used concurrently: the
    // Vulkan loader crashes (SIGSEGV) when two simulator tests initialise at
    // the same time. Serialise the whole scenario, including its control
    // render, so these integration tests can stay parallel with the rest.
    static RENDER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = RENDER_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let render = scrolled_render(probe, directory, parent_clip_bottom);

    // The probe targets the wgpu blur/clip pipeline. Without a GPU (CI) iced
    // falls back to tiny-skia, which draws images unclipped, so the pixel
    // check is meaningless there. `matches_image` suffixes the backend name.
    if !render.backend.contains("wgpu") {
        eprintln!("skipping: snapshot not rendered by wgpu ({:?})", render.png);
        return;
    }

    let (iw, ih) = render.image.dimensions();
    // `iced_test::Simulator::snapshot` renders at a fixed 2× scale factor;
    // derive it from the snapshot instead of hard-coding it.
    let scale = iw as f32 / WINDOW_SIZE.0;
    println!("=== {scenario} ===");
    println!(
        "screen: {}x{} logical, snapshot {iw}x{ih} (backend {}, {scale}× scale)",
        WINDOW_SIZE.0, WINDOW_SIZE.1, render.backend
    );
    println!("tile palette:");
    for (name, c, tiles) in palette() {
        println!("  {name:5} rgb{:?} — {tiles}", c);
    }
    println!(
        "gallery bounds: {:?}\nincoming viewport: {:?}\nwidget clip: {:?}\nscroll offset: {} (first_row {}, {} built rows)",
        render.probe.bounds,
        render.probe.viewport,
        render.probe.clip,
        render.probe.offset,
        render.probe.first_row,
        render.probe.rows,
    );

    // The boundary comes from the layout (bounds ∩ viewport), never from one
    // platform's assumed pixel geometry.
    let clip = expected_clip(&render).expect("the gallery must be visible");
    let (left, top, right, bottom) = outside_bounds(clip, scale, (iw, ih));
    println!(
        "required clip: {:?}\n  physical fully-outside ranges: left<{left}, top<{top}, right>{right}, bottom>{bottom}",
        clip,
    );

    let offenders = offenders_outside(&render.image, clip, scale);
    println!(
        "full-opacity tile-colour pixels outside the gallery clip: {}",
        offenders.len()
    );
    for o in &offenders {
        println!(
            "  ({}, {}) rgba{:?} = {} tile, escaped {}",
            o.x, o.y, o.rgba, o.tile, o.edge
        );
    }

    assert_eq!(
        render.probe.clip,
        Some(clip),
        "VirtualGallery::draw must clip to bounds ∩ viewport ({:?}), but used {:?} \
         (bounds {:?}, viewport {:?}, offset {}); snapshot: {}",
        clip,
        render.probe.clip,
        render.probe.bounds,
        render.probe.viewport,
        render.probe.offset,
        render.png.display(),
    );

    if !offenders.is_empty() {
        // Classify, do not guess: re-render with the synthetic thumbnails
        // removed. A palette pixel in the control render would mean the
        // classifier catches ordinary UI; zero means every palette pixel is
        // genuinely tile content.
        let control = scrolled_render(
            &probe.without_tiles(),
            &format!("{directory}-control"),
            parent_clip_bottom,
        );
        let control_clip = expected_clip(&control).expect("visible gallery");
        let control_offenders = offenders_outside(&control.image, control_clip, scale);
        println!(
            "control render (no tile images): {} palette pixels outside the clip; \
             bounds {:?} vs {:?}",
            control_offenders.len(),
            control.probe.bounds,
            render.probe.bounds
        );
        eprintln!(
            "failing snapshot kept at {} (control: {})",
            render.png.display(),
            control.png.display()
        );
    }

    assert!(
        offenders.is_empty(),
        "gallery tile content rendered outside the gallery clip: {} full-opacity palette \
         pixel(s) outside {:?} (bounds {:?}, viewport {:?}); snapshot: {}",
        offenders.len(),
        clip,
        render.probe.bounds,
        render.probe.viewport,
        render.png.display(),
    );
}

#[test]
fn probe_scrolled_gallery() {
    assert_gallery_clip(
        &Probe::new(),
        "scrolled",
        None,
        "scrolled gallery in the real app view",
    );
}

/// The stated invariant, exercised for real: the whole app view sits under a
/// scrollable that exposes only the top 400 logical pixels. The gallery still
/// lays itself out 828.8 logical px tall, so `draw()` must intersect its
/// bounds with the parent viewport (a layer is scissored to its OWN bounds
/// only in iced_wgpu). Before the fix this painted ~490k tile pixels below
/// the parent clip.
#[test]
fn probe_gallery_inside_clipped_parent() {
    assert_gallery_clip(
        &Probe::new(),
        "clipped-parent",
        Some(400.0),
        "gallery inside a clipped parent viewport",
    );
}

/// The classifier must never confuse ordinary UI with tile content. The R-18
/// badge background is exactly `(230, 51, 51)` (the app's `[0.9, 0.2, 0.2]`
/// danger red) and the old loose `is_tile_color` predicate counted it as a
/// "red tile" — that is what made the previous Windows run report 2 offending
/// pixels above its hard-coded clip line. Full-opacity tile pixels are the
/// palette primaries only.
#[test]
fn classifier_ignores_ordinary_ui_colours() {
    assert_eq!(tile_color(&[230, 51, 51, 255]), None, "R-18 badge red");
    assert_eq!(tile_color(&[88, 101, 242, 255]), None, "theme accent blue");
    assert_eq!(tile_color(&[244, 96, 122, 255]), None, "theme danger pink");
    assert_eq!(tile_color(&[54, 56, 60, 255]), None, "panel background");
    // Blended (partial-coverage) tile pixels are not full-opacity content.
    assert_eq!(tile_color(&[128, 0, 127, 255]), None, "red/blue AA seam");

    assert_eq!(tile_color(&[255, 0, 0, 255]), Some("red"));
    assert_eq!(tile_color(&[0, 255, 0, 255]), Some("green"));
    assert_eq!(tile_color(&[0, 0, 255, 255]), Some("blue"));
}
