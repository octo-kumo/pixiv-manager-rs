//! Temporary probe: render the app's real widget stack (view()) with
//! blurred R-18 tiles and check — via rendered pixels — that tile content
//! never leaves the gallery's clip region (the "blur above everything"
//! bug), with and without a scrolled/animated VirtualGallery.

use image::{ImageBuffer, ImageEncoder, Rgba};
use pixiv_manager::app::message::Message;
use pixiv_manager::app::state::{AppPhase, AppState, GalleryTab, Tab};
use pixiv_manager::app::view::view;
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

impl Probe {
    fn new() -> Self {
        std::env::set_var(
            "PIXIV_MANAGER_CONFIG_DIR",
            "/tmp/opencode/layer_probe_config",
        );
        std::fs::create_dir_all("/tmp/opencode/layer_probe_config").unwrap();
        let mut state = AppState {
            phase: AppPhase::Main,
            window_size: (1920.0, 1080.0),
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
        let red_blue = split_png([255, 0, 0], [0, 0, 255]);
        let green_blue = split_png([0, 255, 0], [0, 0, 255]);
        let mut handles = std::collections::HashMap::new();
        for id in 2..=18u64 {
            let bytes = if id % 2 == 0 {
                red_blue.clone()
            } else {
                green_blue.clone()
            };
            handles.insert(id, iced::widget::image::Handle::from_bytes(bytes));
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

    fn ui(&self) -> iced_test::Simulator<'_, Message, iced::Theme, iced::Renderer> {
        let mut ui = iced_test::simulator(view(&self.state));
        ui.point_at(iced::Point::new(600.0, 400.0));
        ui
    }
}

fn split_png(left: [u8; 3], right: [u8; 3]) -> Vec<u8> {
    let mut img = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(64, 64);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let c = if x < 32 { left } else { right };
        *p = Rgba([c[0], c[1], c[2], 255]);
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(img.as_raw(), 64, 64, image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}

fn is_tile_color(p: &[u8; 4]) -> bool {
    let (r, g, b, _) = (p[0], p[1], p[2], p[3]);
    // Strict: the pure test-image halves and their blur mixes. (The theme's
    // accent blue (88, 101, 242) — used by the control-pane chips — must NOT
    // count as tile content.)
    (r > 200 && g < 60 && b < 60)          // red half
        || (b > 200 && r < 60 && g < 60)    // blue half
        || (g > 200 && r < 60 && b < 60)    // green half
        || (r > 110 && g < 80 && b > 80 && b < 210) // red/blue blur mix
        || (g > 110 && b > 80 && b < 200 && r < 80) // green/blue blur mix
}

/// The strip above the gallery (tab bar + controls): ~90 logical px.
const TOP_STRIP: u32 = 90;

#[test]
fn probe_scrolled_gallery() {
    let probe = Probe::new();
    let mut ui = probe.ui();
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
    for _ in 0..6 {
        ui.simulate([redraw()]);
        ui.simulate([wheel(-3.0)]);
        ui.simulate([redraw()]);
    }
    // Let the glide settle.
    for _ in 0..40 {
        ui.simulate([redraw()]);
    }

    for entry in std::fs::read_dir("target/layer_probe").unwrap().flatten() {
        let _ = std::fs::remove_file(entry.path());
    }
    let snapshot = ui.snapshot(&iced::Theme::Dark).expect("snapshot");
    snapshot
        .matches_image("target/layer_probe/shot.png")
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

    // Every tile-color pixel must sit BELOW the gallery's clip top. The
    // VirtualGallery's draw bounds start at y=61.2 logical (tab bar +
    // controls); the simulator renders at 2× scale.
    let clip_top = 122u32; // floor(61.2 * 2) = 122: rows < 122 are outside
    let mut above = 0;
    for y in 0..clip_top.min(ih) {
        for x in 0..iw {
            if is_tile_color(&img.get_pixel(x, y).0) {
                above += 1;
            }
        }
    }
    println!("tile-color pixels above the gallery clip: {above}");
    assert_eq!(
        above, 0,
        "tile content must never escape the gallery's clip region"
    );
}
