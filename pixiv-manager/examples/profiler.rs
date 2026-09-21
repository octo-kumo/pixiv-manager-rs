//! Headless AI-agent profiler harness (feature = `profiling`).
//!
//! Drives the real app (`update`/`view`/async tasks) against a local mock
//! Pixiv API + image CDN, simulates a scripted user session (tab switches,
//! scrolling, viewer, artist, filters, search, downloads), and writes a
//! profiling report.
//!
//! Usage:
//! ```text
//! cargo run -p pixiv-manager --example profiler --features profiling \
//!     [-- --out profiles/latest --cpu 100 --http 0]
//! ```
//!
//! Options:
//! * `--out <dir>`     report output dir (default `profiles/latest`)
//! * `--cpu <hz>`      CPU sampling frequency (0 = off, default 100)
//! * `--http <port>`   spawn the HTTP control server (0 = ephemeral, default off)
//!
//! Outputs: `report.md`, `report.json`, `flamegraph.svg`, `hotspots.md`.

use pixiv_manager::app::message::Message;
use pixiv_manager::app::state::{AppPhase, AppState, ImageState, JobsState, Tab};
use pixiv_manager::image::cache::{ImageCache, DEFAULT_MEMORY_BUDGET};
use pixiv_manager::profiling::{begin_session, timed_update, timed_view};
use pixivj_rs::api::PixivApiClient;
use pixivj_rs::auth::{LazyTokenRefresher, PixivOAuthClient};
use std::path::PathBuf;
use std::sync::Arc;

// ── Fixtures ────────────────────────────────────────────────────────────

/// Rewrite every URL in a JSON value to point at the mock server, and strip
/// `next_url` (its hostname security check would reject our mock host).
fn rewrite_urls(value: &mut serde_json::Value, base: &str) {
    match value {
        serde_json::Value::String(s) => {
            if s.starts_with("https://") || s.starts_with("http://") {
                // Keep the path after the host so distinct originals stay
                // distinct on the mock.
                let path = s.splitn(4, '/').nth(3).unwrap_or("").to_string();
                *s = format!("{base}/{path}");
            }
        }
        serde_json::Value::Array(a) => {
            for v in a {
                rewrite_urls(v, base);
            }
        }
        serde_json::Value::Object(o) => {
            for (k, v) in o.iter_mut() {
                if k == "next_url" {
                    *v = serde_json::Value::Null;
                } else {
                    rewrite_urls(v, base);
                }
            }
        }
        _ => {}
    }
}

fn fixture_value(name: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(format!(
        "{}/tests/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    ))
    .expect("fixture missing");
    serde_json::from_str(&text).expect("fixture parse")
}

/// Generate a deterministic 256x256 gradient PNG for the mock CDN.
fn cdn_png() -> Vec<u8> {
    use image::ImageEncoder;
    let mut img = image::RgbImage::new(256, 256);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = image::Rgb([(x * 255 / 255) as u8, (y * 255 / 255) as u8, 128]);
    }
    let mut out = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    encoder
        .write_image(&img, 256, 256, image::ExtendedColorType::Rgb8)
        .expect("png encode");
    out
}

struct MockData {
    api: Vec<(String, String)>, // (prefix, json body)
    png: Vec<u8>,
}

impl MockData {
    fn new(base: &str) -> Self {
        let mut api = Vec::new();
        for (route, fixture) in [
            ("/v1/illust/recommended", "recommended.json"),
            ("/v1/illust/ranking", "recommended.json"),
            ("/v1/illust/detail", "detail_multi.json"),
            ("/v1/illust/comments", "comments.json"),
            ("/v3/illust/comments", "comments.json"),
            ("/v1/user/detail", "user_detail.json"),
            ("/v1/user/illusts", "user_illusts.json"),
            ("/v1/user/bookmarks/illust", "bookmarks.json"),
            ("/v2/illust/follow", "bookmarks.json"),
            ("/v1/search/illust", "bookmarks.json"),
        ] {
            let mut value = fixture_value(fixture);
            rewrite_urls(&mut value, base);
            api.push((route.to_string(), serde_json::to_string(&value).unwrap()));
        }
        // Autocomplete + write-endpoint success bodies.
        api.push(("/v2/search/autocomplete".into(), r#"{"tags":[]}"#.into()));
        api.push(("/v1/illust/bookmark/delete".into(), "{}".into()));
        api.push(("/v1/user/follow/add".into(), "{}".into()));
        api.push(("/v1/user/follow/delete".into(), "{}".into()));
        Self {
            api,
            png: cdn_png(),
        }
    }

    fn route(&self, path: &str) -> (&'static str, &'static str, Vec<u8>) {
        for (prefix, body) in &self.api {
            if path.starts_with(prefix.as_str()) {
                return ("200 OK", "application/json", body.as_bytes().to_vec());
            }
        }
        if path.starts_with('/') {
            return ("200 OK", "image/png", self.png.clone());
        }
        ("404 Not Found", "text/plain", b"nope".to_vec())
    }
}

// ── Mock HTTP server (tokio) ────────────────────────────────────────────

async fn serve_connection(
    mut socket: tokio::net::TcpStream,
    data: Arc<MockData>,
) -> std::io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 2048];
    loop {
        let n = socket.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 64 * 1024 {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
    let path = path.split('?').next().unwrap_or("").to_string();
    let (status, ctype, body) = data.route(&path);
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(header.as_bytes()).await?;
    socket.write_all(&body).await?;
    socket.flush().await?;
    Ok(())
}

async fn run_mock(listener: tokio::net::TcpListener, data: Arc<MockData>) {
    while let Ok((socket, _)) = listener.accept().await {
        let data = data.clone();
        tokio::spawn(async move {
            let _ = serve_connection(socket, data).await;
        });
    }
}

// ── Task executor (mini iced runtime) ───────────────────────────────────

/// Execute an `iced::Task`, replying to any side-effect actions with sane
/// defaults, and return every `Output` message it produced.
fn execute_task(task: iced::Task<Message>, rt: &tokio::runtime::Runtime) -> Vec<Message> {
    use iced::futures::StreamExt;
    use iced_test::runtime;

    let Some(stream) = runtime::task::into_stream(task) else {
        return Vec::new();
    };
    let mut outputs: Vec<Message> = Vec::new();
    rt.block_on(async {
        let mut stream = stream;
        while let Some(action) = stream.next().await {
            match action {
                runtime::Action::Output(msg) => outputs.push(msg),
                runtime::Action::LoadFont { channel, .. } => {
                    let _ = channel.send(Ok(()));
                }
                runtime::Action::System(act) => match act {
                    runtime::system::Action::GetInformation(tx) => {
                        let _ = tx.send(runtime::system::Information {
                            system_name: Some("Linux".into()),
                            system_kernel: Some("mock".into()),
                            system_version: None,
                            system_short_version: None,
                            cpu_brand: "MockCPU".into(),
                            cpu_cores: Some(8),
                            memory_total: 16 * 1024 * 1024 * 1024,
                            memory_used: None,
                            graphics_backend: "mock".into(),
                            graphics_adapter: "mock".into(),
                        });
                    }
                    runtime::system::Action::GetTheme(tx) => {
                        let _ = tx.send(runtime::core::theme::Mode::Dark);
                    }
                    _ => {}
                },
                runtime::Action::Window(act) => match act {
                    runtime::window::Action::Open(id, _settings, tx) => {
                        let _ = tx.send(id);
                    }
                    runtime::window::Action::GetOldest(tx) => {
                        let _ = tx.send(None);
                    }
                    runtime::window::Action::GetLatest(tx) => {
                        let _ = tx.send(None);
                    }
                    runtime::window::Action::GetSize(_id, tx) => {
                        let _ = tx.send(runtime::core::Size::new(1600.0, 900.0));
                    }
                    runtime::window::Action::GetMaximized(_id, tx) => {
                        let _ = tx.send(false);
                    }
                    runtime::window::Action::GetMinimized(_id, tx) => {
                        let _ = tx.send(None);
                    }
                    runtime::window::Action::GetPosition(_id, tx) => {
                        let _ = tx.send(None);
                    }
                    runtime::window::Action::GetScaleFactor(_id, tx) => {
                        let _ = tx.send(1.0);
                    }
                    runtime::window::Action::GetMode(_id, tx) => {
                        let _ = tx.send(iced::window::Mode::Windowed);
                    }
                    runtime::window::Action::GetRawId(_id, tx) => {
                        let _ = tx.send(0);
                    }
                    runtime::window::Action::GetMonitorSize(_id, tx) => {
                        let _ = tx.send(None);
                    }
                    _ => {}
                },
                runtime::Action::Clipboard(act) => {
                    if let runtime::clipboard::Action::Read { channel, .. } = act {
                        let _ = channel.send(None);
                    }
                }
                runtime::Action::Image(act) => match act {
                    runtime::image::Action::Allocate(_handle, tx) => {
                        let _ = tx.send(Err(runtime::image::Error::Unsupported));
                    }
                },
                runtime::Action::Widget(_) | runtime::Action::Reload | runtime::Action::Exit => {}
            }
        }
    });
    outputs
}

// ── Session driver ──────────────────────────────────────────────────────

struct Driver {
    state: AppState,
    rt: tokio::runtime::Runtime,
}

impl Driver {
    /// Dispatch a message and drain the resulting task cascade.
    fn drive(&mut self, msg: Message) {
        let task = timed_update(&mut self.state, msg);
        self.drain(task);
    }

    fn drain(&mut self, task: iced::Task<Message>) {
        let outputs = execute_task(task, &self.rt);
        for msg in outputs {
            self.drive(msg);
        }
    }

    /// Measure one view pass at the current state.
    fn view(&mut self) {
        let _ = timed_view(&self.state);
    }
}

// ── Scenario ────────────────────────────────────────────────────────────

/// A scripted "real user" session. Runs every update/view through the timed
/// wrappers so frametime, memory and CPU attribution are recorded.
fn run_scenario(d: &mut Driver) {
    // 1. Load the Recommend tab from the mock API (network fetch).
    d.drive(Message::TabSelected(Tab::Recommend));
    d.view();
    d.drive(Message::GalleryWindowChanged {
        start: 0.0,
        end: 900.0,
    });
    d.view();

    // 2. Grow the tab into a 2000-item grid (like a big bookmark library) to
    //    exercise lazy-thumbnail windowing + per-frame tree rebuild at scale.
    let base = d
        .state
        .tabs
        .recommend
        .illustrations
        .first()
        .cloned()
        .expect("recommended fixture must load");
    let mut big: Vec<pixivj_rs::model::Illustration> = Vec::with_capacity(2000);
    for i in 0..2000u64 {
        let mut il = base.clone();
        il.id = 9_000_000 + i;
        big.push(il);
    }
    d.state.tabs.recommend.illustrations = big;
    d.state.tabs.recommend.reapply(&d.state.images.palettes);

    // 3. Simulate scrolling through the grid: each window triggers thumbnail
    //    spawns (network+resize+disk) and a full view() rebuild.
    for step in 0..8 {
        let start = step as f32 * 800.0;
        d.drive(Message::GalleryWindowChanged {
            start,
            end: start + 900.0,
        });
        d.view();
    }
    // Scroll into the bottom → rising edge LoadMore.
    d.drive(Message::ScrollPositionChanged(false));
    d.drive(Message::ScrollPositionChanged(true));
    d.view();

    // 4. Other tabs (network fetches).
    d.drive(Message::TabSelected(Tab::Feed));
    d.view();
    d.drive(Message::TabSelected(Tab::Bookmarks));
    d.view();
    d.drive(Message::GalleryWindowChanged {
        start: 0.0,
        end: 900.0,
    });
    d.view();

    // 5. Search: type, autocomplete, submit.
    d.drive(Message::TabSelected(Tab::Search));
    d.drive(Message::SearchTextChanged("frieren".into()));
    d.drive(Message::SearchSubmitted);
    d.view();
    d.drive(Message::GalleryWindowChanged {
        start: 0.0,
        end: 900.0,
    });
    d.view();

    // 6. Filter + sort on the big Recommend grid (reapply over 2000 items).
    d.drive(Message::TabSelected(Tab::Recommend));
    d.drive(Message::SortKeyClicked(
        pixiv_manager::app::sort::SortKey::Views,
    ));
    d.drive(Message::SortKeyClicked(
        pixiv_manager::app::sort::SortKey::Bookmarks,
    ));
    d.drive(Message::OptionFilterToggled(
        pixiv_manager::app::filter::OptionFilterKey::R18,
    ));
    d.drive(Message::ToggleAdvancedFilters);
    d.view();

    // 7. Viewer: open the first (real, fetched) illust, page through, reopen
    //    to expose duplicate detail requests.
    let first = d
        .state
        .tabs
        .recommend
        .illustrations
        .iter()
        .find(|i| i.id < 9_000_000)
        .map(|i| i.id)
        .unwrap_or(0);
    if first != 0 {
        d.drive(Message::GalleryItemClicked(first));
        d.view();
        d.drive(Message::ViewerPageChanged(1));
        d.drive(Message::ViewerPageSettled);
        d.view();
        // Re-open the same illust: the detail/comments/full-image URLs get
        // requested again → duplicate-request detection.
        d.drive(Message::ViewerClosed);
        d.drive(Message::GalleryItemClicked(first));
        d.view();
        d.drive(Message::ViewerClosed);
    }

    // 8. Artist page.
    if let Some(author) = d
        .state
        .tabs
        .recommend
        .illustrations
        .iter()
        .find(|i| i.id < 9_000_000)
        .and_then(|i| i.user.as_ref())
    {
        d.drive(Message::ArtistOpened(author.id));
        d.view();
        d.drive(Message::ArtistClosed);
    }

    // 9. Debug overlay (per-frame /proc reads + canvas rasterization).
    d.drive(Message::ToggleDebugOverlay);
    d.view();
    d.drive(Message::ToggleDebugOverlay);

    // 10. Settings open/close + a debounced disk flush.
    d.drive(Message::OpenSettings);
    d.view();
    d.drive(Message::CloseSettings);
    d.drive(Message::FlushDisk);

    // 11. Download job + bookmark toggle (write endpoints + disk persistence).
    d.drive(Message::StartBookmarkSync);
    d.drive(Message::FlushDisk);
    if first != 0 {
        d.drive(Message::BookmarkToggled(first));
        d.drive(Message::FlushDisk);
    }
    d.drive(Message::StartDownloadAll);
    d.drive(Message::FlushDisk);
    d.view();

    // 12. A few resize/re-render passes (window resize changes layout).
    d.drive(Message::WindowResized {
        width: 1280.0,
        height: 800.0,
    });
    d.view();
    d.drive(Message::WindowResized {
        width: 1920.0,
        height: 1080.0,
    });
    d.view();

    // 13. CPU stress: concentrate work on the main thread so the sampling
    //     profiler catches line-level culprits (view tree rebuild + the
    //     filter/sort re-apply over the 2000-item grid).
    for _ in 0..400 {
        d.view();
    }
    for _ in 0..60 {
        d.drive(Message::SortKeyClicked(
            pixiv_manager::app::sort::SortKey::Id,
        ));
    }
}

// ── Entry point ─────────────────────────────────────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .try_init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out_dir = PathBuf::from("profiles/latest");
    let mut cpu_hz: i32 = 250;
    let mut http_port: u16 = 0;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out_dir = PathBuf::from(&args[i]);
            }
            "--cpu" => {
                i += 1;
                cpu_hz = args[i].parse()?;
            }
            "--http" => {
                i += 1;
                http_port = args[i].parse()?;
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    // Point config/library/cache at a private temp dir (never touch user data).
    let temp = tempfile::tempdir()?;
    std::env::set_var("PIXIV_MANAGER_CONFIG_DIR", temp.path());

    begin_session();

    // Start the mock Pixiv server.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    let (mock_port, _mock_handle) = rt.block_on(async {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind mock");
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let data = Arc::new(MockData::new(&base));
        let handle = tokio::spawn(run_mock(listener, data.clone()));
        (port, handle)
    });
    println!("mock pixiv server on 127.0.0.1:{mock_port}");

    // Optional HTTP control server (agent-driven captures on a long-running
    // session: `--http 0` binds an ephemeral port and keeps serving until
    // killed, so an agent can curl /profile/stop, /metrics, /report).
    let http_bound = if http_port > 0 {
        let dir = out_dir.clone();
        pixiv_manager::profiling::http::init(dir);
        Some(pixiv_manager::profiling::http::spawn(http_port)?)
    } else {
        None
    };
    if let Some(bound) = http_bound {
        println!("profiling control server on http://127.0.0.1:{bound}");
    }

    // Build the app state pointed at the mock.
    let mut driver = Driver {
        state: build_state(mock_port, temp.path(), &rt),
        rt,
    };

    // CPU sampling.
    if cpu_hz > 0 {
        pixiv_manager::profiling::cpu::set_frequency(cpu_hz);
        pixiv_manager::profiling::cpu::start()?;
        println!("cpu sampling at {cpu_hz}Hz");
    }

    let scenario_start = std::time::Instant::now();
    run_scenario(&mut driver);
    println!(
        "scenario ran in {:.2}s ({} update samples, {} view samples)",
        scenario_start.elapsed().as_secs_f64(),
        pixiv_manager::profiling::tracing::snapshot().update.len(),
        pixiv_manager::profiling::tracing::snapshot().view.len(),
    );
    // Diagnostics: how many thumbnails actually reached the disk cache?
    let cache_files = std::fs::read_dir(temp.path().join("cache"))
        .map(|d| d.flatten().count())
        .unwrap_or(0);
    println!("disk cache files: {cache_files}");

    // Collect and write the report.
    let snapshot = pixiv_manager::profiling::tracing::snapshot();
    let mem = pixiv_manager::profiling::memory::snapshot();
    let report = if cpu_hz > 0 {
        pixiv_manager::profiling::cpu::stop()
    } else {
        None
    };
    let (md, _json) = match report {
        Some(report) => {
            pixiv_manager::profiling::report::write_with_cpu(&snapshot, &mem, &report, &out_dir)
        }
        None => pixiv_manager::profiling::report::write(&snapshot, &mem, &out_dir),
    };
    println!("\n=== report written to {} ===\n", out_dir.display());
    println!("{md}");

    // In server mode (`--http`), keep serving so an agent can run extra
    // captures with curl; otherwise exit immediately after the report.
    if http_bound.is_some() {
        println!("\nserver mode: press Ctrl-C to stop");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    Ok(())
}

fn build_state(mock_port: u16, temp: &std::path::Path, rt: &tokio::runtime::Runtime) -> AppState {
    let base = format!("http://127.0.0.1:{mock_port}");
    let mut state = AppState {
        phase: AppPhase::Main,
        active_tab: Tab::Recommend,
        window_size: (1600.0, 900.0),
        images: ImageState {
            image_cache: Arc::new(ImageCache::new(temp.join("cache"), DEFAULT_MEMORY_BUDGET)),
            ..Default::default()
        },
        jobs: JobsState {
            library_path: temp.join("library").join("bookmarks.json"),
            download_queue_path: temp.join("library").join("download_queue.json"),
            ..Default::default()
        },
        ..Default::default()
    };

    // API client pointed at the mock; token provider pre-seeded so no real
    // OAuth network call happens.
    let refresher = Arc::new(LazyTokenRefresher::new(
        PixivOAuthClient::new().expect("oauth client"),
        "mock-refresh-token".into(),
    ));
    rt.block_on(refresher.update_tokens(
        "mock-access-token".into(),
        "mock-refresh-token".into(),
        3600,
    ));
    refresher.set_user_id(123456789);
    state.client = Some(
        PixivApiClient::new(refresher)
            .expect("api client")
            .with_base_url(base),
    );

    state
}
