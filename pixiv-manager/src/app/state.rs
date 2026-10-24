use crate::app::filter::{FilterState, Tristate};
use crate::app::sort::SortState;
use crate::image::cache::ImageCache;
use pixivj_rs::api::PixivApiClient;
use pixivj_rs::model::{Author, Comment, Illustration};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

// ── Tab ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Tab {
    #[default]
    Recommend,
    Feed,
    /// Twitter home timeline (photo tweets only).
    Twitter,
    /// Bilibili dynamic feed (image posts only).
    Bilibili,
    Bookmarks,
    Search,
    /// Pixiv daily/weekly/monthly leaderboard.
    Ranking,
    Extra,
}

// ── Tab Data ───────────────────────────────────────────────────────────

/// Pixiv-API-side search preferences (server-side sort/match options), as
/// opposed to the client-side [`FilterState`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchApiPrefs {
    /// Server-side result order (`None` = Pixiv's default, newest first).
    pub sort: Option<pixivj_rs::model::SortOption>,
    /// Server-side match mode (`None` = Pixiv's default, partial tag match).
    pub target: Option<pixivj_rs::model::SearchTarget>,
    /// Also match translated tag names.
    pub translated: bool,
}

/// The per-tab gallery state. One struct for every tab — the five tabs used
/// to have four parallel structs that forced hand-written 5-way dispatches
/// and drifted apart; a single type keeps every tab's behavior identical.
#[derive(Debug, Clone, Default)]
pub struct GalleryTab {
    pub illustrations: Vec<Illustration>,
    pub filtered: Vec<Illustration>,
    pub next_url: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub filters: FilterState,
    pub sort: SortState,
    /// Remaining auto-loads allowed after a filter/search shrank the grid.
    /// Reset to 0 whenever a fetch stops producing new data.
    pub auto_load_remaining: u32,
    /// Whether this tab's continuous loading ("load all pages") is on.
    /// Toggled per tab from the control pane button.
    pub auto_load: bool,
    /// This tab's saved gallery scroll offset (content coords). Tab
    /// switches must not lose it: switching back restores the position.
    pub scroll: f32,
    /// Cursor for the mixed Bookmarks tab's Twitter likes pagination
    /// (`None` = exhausted or never started). Only used by
    /// [`Tab::Bookmarks`]; the Pixiv cursor lives in `next_url`.
    pub next_twitter_cursor: Option<String>,
    /// Whether a Twitter likes page is in flight (Bookmarks tab).
    pub loading_twitter: bool,
    /// The search query for [`Tab::Search`]. `None` for tabs that don't
    /// search, and for [`Tab::Extra`] until a source feeds it data.
    pub query: Option<String>,
    /// Leaderboard (ranking) mode for [`Tab::Ranking`]; `None` = daily.
    pub ranking_mode: Option<pixivj_rs::model::FilterMode>,
    /// Leaderboard date (YYYY-MM-DD) for [`Tab::Ranking`] (past rankings).
    pub ranking_date: String,
    /// Illust ids found on disk in the gallery directory (drives the "Missing"
    /// filter and per-tile downloaded checkmarks). Empty = unknown.
    pub downloaded: std::collections::HashSet<u64>,
    /// Server-side search parameters (used by [`Tab::Search`] fetches).
    pub search_api: SearchApiPrefs,
}

impl GalleryTab {
    /// Re-apply this tab's filters + sort to its illustrations. This is what
    /// makes sort/filter changes take effect on the list. `palettes` feeds
    /// the sRGB/Hue color comparators.
    pub fn reapply(&mut self, palettes: &std::collections::HashMap<u64, Vec<[u8; 3]>>) {
        self.filtered = self.filters.apply(&self.illustrations, &self.downloaded);
        if self.filters.mode == crate::app::filter::FilterMode::Query {
            // Query mode sorts by its own `order:` tokens.
            crate::app::query::Query::parse(&self.filters.query_text)
                .sort_state()
                .sort(&mut self.filtered, palettes);
        } else {
            self.sort.sort(&mut self.filtered, palettes);
        }
    }
}

// ── Tabs State ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct TabsState {
    pub recommend: GalleryTab,
    pub feed: GalleryTab,
    pub twitter: GalleryTab,
    pub bilibili: GalleryTab,
    pub bookmarks: GalleryTab,
    pub search: GalleryTab,
    pub ranking: GalleryTab,
    pub extra: GalleryTab,
}

impl TabsState {
    /// The gallery tab for `tab`.
    pub fn tab(&self, tab: Tab) -> &GalleryTab {
        match tab {
            Tab::Recommend => &self.recommend,
            Tab::Feed => &self.feed,
            Tab::Twitter => &self.twitter,
            Tab::Bilibili => &self.bilibili,
            Tab::Bookmarks => &self.bookmarks,
            Tab::Search => &self.search,
            Tab::Ranking => &self.ranking,
            Tab::Extra => &self.extra,
        }
    }

    /// The gallery tab for `tab` (mutable).
    pub fn tab_mut(&mut self, tab: Tab) -> &mut GalleryTab {
        match tab {
            Tab::Recommend => &mut self.recommend,
            Tab::Feed => &mut self.feed,
            Tab::Twitter => &mut self.twitter,
            Tab::Bilibili => &mut self.bilibili,
            Tab::Bookmarks => &mut self.bookmarks,
            Tab::Search => &mut self.search,
            Tab::Ranking => &mut self.ranking,
            Tab::Extra => &mut self.extra,
        }
    }

    /// Every gallery tab, in a stable order.
    ///
    /// Cross-tab updates (bookmark/follow/refresh propagation) must use this
    /// instead of hand-listing tabs: the hand-written lists had already
    /// drifted, silently skipping the Bilibili/Ranking tabs in some paths.
    pub fn iter(&self) -> impl Iterator<Item = &GalleryTab> {
        [
            &self.recommend,
            &self.feed,
            &self.twitter,
            &self.bilibili,
            &self.bookmarks,
            &self.search,
            &self.ranking,
            &self.extra,
        ]
        .into_iter()
    }

    /// Every gallery tab (mutable), in the same order as [`Self::iter`].
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut GalleryTab> {
        [
            &mut self.recommend,
            &mut self.feed,
            &mut self.twitter,
            &mut self.bilibili,
            &mut self.bookmarks,
            &mut self.search,
            &mut self.ranking,
            &mut self.extra,
        ]
        .into_iter()
    }

    /// Every gallery tab paired with its [`Tab`] discriminant, in the same
    /// order as [`Self::iter`].
    pub fn enumerate_mut(&mut self) -> impl Iterator<Item = (Tab, &mut GalleryTab)> {
        self.iter_mut().enumerate().map(|(i, t)| (tab_at(i), t))
    }
}

/// The [`Tab`] discriminant at `index` in [`TabsState::iter`] order.
fn tab_at(index: usize) -> Tab {
    match index {
        0 => Tab::Recommend,
        1 => Tab::Feed,
        2 => Tab::Twitter,
        3 => Tab::Bilibili,
        4 => Tab::Bookmarks,
        5 => Tab::Search,
        6 => Tab::Ranking,
        _ => Tab::Extra,
    }
}

// ── Twitter ────────────────────────────────────────────────────────────

/// Connection state of the Twitter client, shown by the Twitter tab and the
/// settings screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TwitterStatus {
    /// No cookie file configured/found.
    #[default]
    Disabled,
    /// Cookies were found; the client is being verified.
    Loading,
    /// Connected; `handle` is the logged-in account's @name.
    Ready { handle: String },
    /// Cookies missing/expired or the file could not be read.
    Error(String),
}

impl TwitterStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, TwitterStatus::Ready { .. })
    }

    /// The connected account handle, if known.
    pub fn handle(&self) -> Option<&str> {
        match self {
            TwitterStatus::Ready { handle } => Some(handle),
            _ => None,
        }
    }
}

/// Twitter client + UI state. Lives beside (not inside) the tab state so it
/// survives tab switches and is shared by the gallery, viewer and jobs.
#[derive(Debug, Clone, Default)]
pub struct TwitterState {
    /// The cookie-authenticated GraphQL client; `None` until cookies load.
    pub client: Option<crate::twitter::TwitterApiClient>,
    pub status: TwitterStatus,
    /// Which home timeline the Twitter tab shows.
    pub feed_mode: crate::twitter::TwitterFeedMode,
    /// Whether the settings "Update query ids" fetch is in flight.
    pub query_ids_loading: bool,
}

// ── Bilibili ───────────────────────────────────────────────────────────

/// Connection state of the Bilibili client.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BilibiliStatus {
    /// No cookie file configured/found.
    #[default]
    Disabled,
    /// Cookies were found; the client is being verified.
    Loading,
    /// Connected; `name` is the logged-in account's display name.
    Ready { name: String },
    /// Cookies missing/expired or the file could not be read.
    Error(String),
}

impl BilibiliStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, BilibiliStatus::Ready { .. })
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            BilibiliStatus::Ready { name } => Some(name),
            _ => None,
        }
    }
}

/// Bilibili client + feed state. Bookmarks live in the shared
/// [`crate::io::library::BookmarkLibrary`] (Bilibili has no liked-posts feed,
/// so they are tracked locally).
#[derive(Debug, Clone, Default)]
pub struct BilibiliState {
    pub client: Option<crate::bilibili::BilibiliApiClient>,
    pub status: BilibiliStatus,
    /// Which dynamic feed the tab shows.
    pub feed_mode: crate::bilibili::BilibiliFeedMode,
    /// `Users` mode: `(uid, next_offset)` pairs still to fetch. Empty = every
    /// configured user has been walked to the end.
    pub users_pending: Vec<(u64, Option<String>)>,
    /// Number of per-user fetches currently in flight.
    pub users_loading: usize,
    /// Raw text of the fixed user-id input (kept separate from the parsed
    /// `Settings::bilibili_user_ids` so editing commas/spaces feels natural).
    pub user_ids_input: String,
}

// ── Settings ──────────────────────────────────────────────────────────

/// How the app chooses its dark/light theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemePreference {
    /// Follow the OS preference (dark/light).
    #[default]
    System,
    /// Always use the light theme.
    Light,
    /// Always use the dark theme.
    Dark,
}

/// How bookmarked items are shown outside the Bookmarks tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BookmarkedDisplay {
    /// Show normally (no treatment).
    #[serde(rename = "off")]
    Off,
    /// Dim the tiles of bookmarked items (the default).
    #[default]
    #[serde(rename = "dim")]
    Dim,
    /// Hide bookmarked items via the per-tab filter ("Bookmarked ✗").
    #[serde(rename = "hide")]
    Hide,
}

impl BookmarkedDisplay {
    /// Cycle for the settings screen button: Off → Dim → Hide → Off.
    pub fn next(self) -> Self {
        match self {
            BookmarkedDisplay::Off => BookmarkedDisplay::Dim,
            BookmarkedDisplay::Dim => BookmarkedDisplay::Hide,
            BookmarkedDisplay::Hide => BookmarkedDisplay::Off,
        }
    }
}

/// The per-tab filter's "Bookmarked" tristate that matches the
/// [`BookmarkedDisplay::Hide`] option (or a reset to its neutral value).
pub(crate) fn bookmarked_filter_default(tab: Tab, display: BookmarkedDisplay) -> Tristate {
    if tab == Tab::Bookmarks || display != BookmarkedDisplay::Hide {
        Tristate::Indeterminate
    } else {
        Tristate::Deselected
    }
}

/// Which rows of a gallery tile's info overlay are visible. Every row can be
/// toggled independently in Settings. (Derived `Default` would give all
/// `false`; the manual impl keeps every row on by default, matching the
/// serde field defaults.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileRows {
    /// The author line (clickable artist name).
    #[serde(default = "default_true")]
    pub author: bool,
    /// The ★bookmarks / 👁views counts line.
    #[serde(default = "default_true")]
    pub counts: bool,
    /// The id · dimensions · date line.
    #[serde(default = "default_true")]
    pub meta: bool,
    /// The R-18 / AI / sanity / page-count badges row.
    #[serde(default = "default_true")]
    pub badges: bool,
}

impl Default for TileRows {
    fn default() -> Self {
        Self {
            author: true,
            counts: true,
            meta: true,
            badges: true,
        }
    }
}

/// One of the toggles inside [`TileRows`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileRowKey {
    Author,
    Counts,
    Meta,
    Badges,
}

impl TileRows {
    /// Whether any row is hidden (drives the settings reset button).
    pub fn is_default_value(&self) -> bool {
        *self == TileRows::default()
    }
}

/// The memory/quality preset: how aggressively the app caches things in RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MemoryPreset {
    /// Keep as little as possible in RAM (thumbnails/caches trimmed hard).
    #[default]
    #[serde(rename = "low")]
    Low,
    /// Keep more in memory for responsiveness: larger image caches and
    /// unbounded viewer pages, at the cost of RAM.
    #[serde(rename = "high")]
    High,
}

impl MemoryPreset {
    /// Multiplier applied to the image caches' RAM budgets.
    pub fn cache_budget_multiplier(self) -> usize {
        match self {
            MemoryPreset::Low => 1,
            MemoryPreset::High => 4,
        }
    }

    /// Whether the viewer keeps every loaded page of the open illust in RAM
    /// (instead of trimming to a window around the current page).
    pub fn keep_all_viewer_pages(self) -> bool {
        self == MemoryPreset::High
    }

    /// Cycle for the settings screen button: Low → High → Low.
    pub fn next(self) -> Self {
        match self {
            MemoryPreset::Low => MemoryPreset::High,
            MemoryPreset::High => MemoryPreset::Low,
        }
    }
}

/// The individual settings shown in the settings screen. Used for per-item
/// reset buttons ("reset this one setting to its default").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsKey {
    Theme,
    GridSize,
    AutoPreload,
    DownloadConcurrency,
    GalleryPath,
    Proxy,
    /// Master parallax on/off.
    Parallax,
    /// Cursor-following parallax on hover.
    ParallaxMouse,
    /// Scroll-position parallax.
    ParallaxScroll,
    /// Base font size of the gallery tile info text.
    GalleryFontSize,
    /// How the gallery tile info text is drawn.
    GalleryTextStyle,
    /// Which info rows show on gallery tiles.
    TileRows,
    /// How bookmarked items are treated outside the Bookmarks tab.
    BookmarkedDisplay,
    /// Blur R-18 tiles until hovered.
    BlurR18,
    /// The memory/quality preset (cache budgets).
    MemoryPreset,
    /// The animation frame-rate limit (0 = uncapped).
    FpsLimit,
    /// High-resolution thumbnails (uses the 1200px source).
    ThumbnailHighRes,
    /// The Twitter viewer's combine/rotate toolbar.
    TwitterViewerTools,
    /// Path to the Twitter cookies file.
    TwitterCookies,
    /// Path to the Bilibili cookies file.
    BilibiliCookies,
    /// The fixed Bilibili user list.
    BilibiliUsers,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub refresh_token: Option<String>,
    pub gallery_path: Option<PathBuf>,
    pub grid_size: u32,
    pub proxy: Option<String>,
    /// Dark/light preference. Defaults to [`ThemePreference::System`] for
    /// older configs that predate the field.
    #[serde(default)]
    pub theme: ThemePreference,
    /// How many pages ahead of the current one to preload in the viewer.
    #[serde(default = "default_auto_preload")]
    pub auto_preload: u32,
    /// Concurrent downloads allowed by the "download all" job.
    #[serde(default = "default_download_concurrency")]
    pub download_concurrency: usize,
    /// Master switch for the gallery parallax effect (both kinds).
    #[serde(default = "default_true")]
    pub parallax: bool,
    /// Cursor-following (mouse) parallax on hover.
    #[serde(default = "default_true")]
    pub parallax_mouse: bool,
    /// Scroll-position parallax: each tile's image shifts with the tile's
    /// position in the viewport as you scroll (a port of the original app's
    /// `setRestParallax`).
    #[serde(default = "default_true")]
    pub parallax_scroll: bool,
    /// Scroll-parallax curve: centered (zero at screen center, the old
    /// behavior) or linear (a monotonic ramp across the viewport).
    #[serde(default)]
    pub parallax_mode: crate::widgets::parallax_image::ParallaxMode,
    /// Base font size (px) for the gallery tile info text. Secondary lines
    /// and badges derive from it.
    #[serde(default = "default_gallery_font_size")]
    pub gallery_font_size: u32,
    /// How the gallery tile info text is drawn over the thumbnail.
    #[serde(default)]
    pub gallery_text_style: crate::widgets::illust_info::GalleryTextStyle,
    /// Which info rows show on gallery tiles (author/counts/meta/badges).
    #[serde(default)]
    pub tile_rows: TileRows,
    /// How bookmarked items are treated outside the Bookmarks tab.
    #[serde(default)]
    pub bookmarked_display: BookmarkedDisplay,
    /// Blur R-18 tiles until the cursor hovers them.
    #[serde(default)]
    pub blur_r18: bool,
    /// The memory/quality preset: cache budgets and retention behavior.
    #[serde(default)]
    pub memory_preset: MemoryPreset,
    /// The animation frame-rate limit in fps (0 = uncapped). Caps the
    /// continuous animation redraws (parallax, gallery glide, blur
    /// transitions); event-driven redraws are never throttled.
    #[serde(default)]
    pub fps_limit: u32,
    /// Generate thumbnails at a higher resolution (uses the 1200px `large`
    /// source and a larger target), so tiles stay sharp at bigger grids and
    /// hi-DPI displays at the cost of more memory/bandwidth.
    #[serde(default)]
    pub high_res_thumbnails: bool,
    /// Show the combine/rotate toolbar in the viewer for Twitter items.
    #[serde(default = "default_true")]
    pub twitter_viewer_tools: bool,
    /// Path to a browser-exported Netscape `cookies.txt` for Twitter/X.
    /// `None` means Twitter is disabled (unless a default path exists).
    #[serde(default)]
    pub twitter_cookie_path: Option<String>,
    /// Path to a browser-exported Netscape `cookies.txt` for Bilibili.
    #[serde(default)]
    pub bilibili_cookie_path: Option<String>,
    /// Fixed list of Bilibili uids whose dynamics the tab shows in `Users`
    /// mode.
    #[serde(default)]
    pub bilibili_user_ids: Vec<u64>,
}

fn default_auto_preload() -> u32 {
    3
}

fn default_download_concurrency() -> usize {
    2
}

fn default_gallery_font_size() -> u32 {
    crate::widgets::illust_info::TileText::DEFAULT_FONT_SIZE
}

fn default_true() -> bool {
    true
}

impl Settings {
    /// Reset a single setting (per the settings screen's per-item reset
    /// buttons). Never touches `refresh_token`.
    pub fn reset(&mut self, key: SettingsKey) {
        match key {
            SettingsKey::Theme => self.theme = ThemePreference::default(),
            SettingsKey::GridSize => self.grid_size = 200,
            SettingsKey::AutoPreload => self.auto_preload = default_auto_preload(),
            SettingsKey::DownloadConcurrency => {
                self.download_concurrency = default_download_concurrency();
            }
            SettingsKey::GalleryPath => self.gallery_path = None,
            SettingsKey::Proxy => self.proxy = None,
            SettingsKey::Parallax => self.parallax = default_true(),
            SettingsKey::ParallaxMouse => self.parallax_mouse = default_true(),
            SettingsKey::ParallaxScroll => self.parallax_scroll = default_true(),
            SettingsKey::GalleryFontSize => {
                self.gallery_font_size = default_gallery_font_size();
            }
            SettingsKey::GalleryTextStyle => {
                self.gallery_text_style = crate::widgets::illust_info::GalleryTextStyle::default();
            }
            SettingsKey::TileRows => self.tile_rows = TileRows::default(),
            SettingsKey::BookmarkedDisplay => {
                self.bookmarked_display = BookmarkedDisplay::default();
            }
            SettingsKey::BlurR18 => self.blur_r18 = false,
            SettingsKey::MemoryPreset => self.memory_preset = MemoryPreset::default(),
            SettingsKey::FpsLimit => self.fps_limit = 0,
            SettingsKey::ThumbnailHighRes => self.high_res_thumbnails = false,
            SettingsKey::TwitterViewerTools => self.twitter_viewer_tools = default_true(),
            SettingsKey::TwitterCookies => self.twitter_cookie_path = None,
            SettingsKey::BilibiliCookies => self.bilibili_cookie_path = None,
            SettingsKey::BilibiliUsers => self.bilibili_user_ids.clear(),
        }
    }

    /// Reset every setting to its default, preserving the refresh token.
    pub fn reset_all(&mut self) {
        let token = self.refresh_token.clone();
        *self = Self::default();
        self.refresh_token = token;
    }

    /// Whether `key` currently holds its default value (for the UI).
    pub fn is_default(&self, key: SettingsKey) -> bool {
        let mut probe = Self::default();
        probe.reset(key);
        match key {
            SettingsKey::Theme => self.theme == probe.theme,
            SettingsKey::GridSize => self.grid_size == probe.grid_size,
            SettingsKey::AutoPreload => self.auto_preload == probe.auto_preload,
            SettingsKey::DownloadConcurrency => {
                self.download_concurrency == probe.download_concurrency
            }
            SettingsKey::GalleryPath => self.gallery_path == probe.gallery_path,
            SettingsKey::Proxy => self.proxy == probe.proxy,
            SettingsKey::Parallax => self.parallax == probe.parallax,
            SettingsKey::ParallaxMouse => self.parallax_mouse == probe.parallax_mouse,
            SettingsKey::ParallaxScroll => self.parallax_scroll == probe.parallax_scroll,
            SettingsKey::GalleryFontSize => self.gallery_font_size == probe.gallery_font_size,
            SettingsKey::GalleryTextStyle => self.gallery_text_style == probe.gallery_text_style,
            SettingsKey::TileRows => self.tile_rows == probe.tile_rows,
            SettingsKey::BookmarkedDisplay => self.bookmarked_display == probe.bookmarked_display,
            SettingsKey::BlurR18 => self.blur_r18 == probe.blur_r18,
            SettingsKey::MemoryPreset => self.memory_preset == probe.memory_preset,
            SettingsKey::FpsLimit => self.fps_limit == probe.fps_limit,
            SettingsKey::ThumbnailHighRes => self.high_res_thumbnails == probe.high_res_thumbnails,
            SettingsKey::TwitterViewerTools => {
                self.twitter_viewer_tools == probe.twitter_viewer_tools
            }
            SettingsKey::TwitterCookies => self.twitter_cookie_path == probe.twitter_cookie_path,
            SettingsKey::BilibiliCookies => self.bilibili_cookie_path == probe.bilibili_cookie_path,
            SettingsKey::BilibiliUsers => self.bilibili_user_ids == probe.bilibili_user_ids,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            refresh_token: None,
            gallery_path: None,
            grid_size: 200,
            proxy: None,
            theme: ThemePreference::System,
            auto_preload: default_auto_preload(),
            download_concurrency: default_download_concurrency(),
            parallax: default_true(),
            parallax_mouse: default_true(),
            parallax_scroll: default_true(),
            parallax_mode: crate::widgets::parallax_image::ParallaxMode::default(),
            gallery_font_size: default_gallery_font_size(),
            gallery_text_style: crate::widgets::illust_info::GalleryTextStyle::default(),
            tile_rows: TileRows::default(),
            bookmarked_display: BookmarkedDisplay::default(),
            blur_r18: false,
            memory_preset: MemoryPreset::default(),
            fps_limit: 0,
            high_res_thumbnails: false,
            twitter_viewer_tools: default_true(),
            twitter_cookie_path: None,
            bilibili_cookie_path: None,
            bilibili_user_ids: Vec::new(),
        }
    }
}

// ── Onboarding ─────────────────────────────────────────────────────────

/// Which step of onboarding we're on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum OnboardingStep {
    /// Waiting for the local server to start.
    #[default]
    Starting,
    /// Server is running, waiting for user to submit token via browser.
    WaitingForToken,
    /// Token received, validating against Pixiv API.
    ValidatingToken,
    /// Token is valid, ask user to select gallery directory.
    SelectDirectory,
    /// Onboarding complete, transitioning to main app.
    Complete,
}

/// Overall app phase.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AppPhase {
    /// First-launch onboarding (or re-onboarding after token rejection).
    #[default]
    Onboarding,
    /// Normal main app.
    Main,
}

// ── App State ─────────────────────────────────────────────────────────

/// All onboarding-flow state. Kept separate from the main app state so the
/// onboarding phase can't accidentally reach into viewer/artist state.
#[derive(Debug, Clone, Default)]
pub struct OnboardingState {
    /// Which step of onboarding we're on.
    pub step: OnboardingStep,
    /// The URL the local server is listening on (e.g. "http://127.0.0.1:8934").
    pub url: Option<String>,
    /// The Pixiv OAuth PKCE login URL (opened in the user's browser).
    pub login_url: Option<String>,
    /// PKCE code_verifier used to exchange the auth code for a refresh token.
    pub code_verifier: Option<String>,
    /// Token the user submitted (before validation).
    pub token: String,
    /// The most recent onboarding failure (shown inline on the setup screen).
    pub error: Option<String>,
    /// Whether we're in offline mode (no internet).
    pub offline: bool,
    /// The onboarding server's token channel, polled by a subscription so the
    /// UI thread never blocks on a `recv()` (which would hang app shutdown).
    pub rx: Option<Arc<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>>,
}

/// Relative page navigation for the viewer (resolved + clamped in update).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerPageNav {
    First,
    Prev,
    Next,
    Last,
}

/// All viewer-overlay state. `open == true` implies `illust_id` is set;
/// [`ViewerState::close`] is the only sanctioned way to reset it.
#[derive(Debug, Clone, Default)]
pub struct ViewerState {
    pub open: bool,
    pub illust_id: u64,
    pub page: usize,
    pub total_pages: usize,
    /// Text typed into the jump-to-page box (`1`-based; submitted → navigate).
    pub page_input: String,
    /// Mirror of the current page (`page + 1`) shown in the box while the
    /// user isn't typing (a borrowed display string for the view).
    pub page_display: String,
    pub illust: Option<Illustration>,
    pub comments: Vec<Comment>,
    /// Cursor for older comments (`next_url` from the last fetch); `None`
    /// when all comments are loaded.
    pub comments_next_url: Option<String>,
    pub loading: bool,
    /// Cached author profile-picture handle for the current viewer illust.
    pub author_pfp: Option<iced::widget::image::Handle>,
    /// Cached full-resolution image `Handle` for the current viewer page.
    /// Set to the best available cached image (gallery thumbnail, or a
    /// previously loaded full image) before the full-res download finishes.
    pub handle: Option<iced::widget::image::Handle>,
    /// Progress of the current full-image download as a fraction [0, 1].
    /// `None` when no full-res image is being downloaded.
    pub progress: Option<f32>,
    /// Cached full-image `Handle`s per viewer page (avoids re-decoding a page
    /// every time it's revisited within a session).
    pub page_handles: HashMap<usize, iced::widget::image::Handle>,
    /// Viewer pages currently being preloaded/loaded (dedup).
    pub pages_loading: std::collections::HashSet<usize>,
    /// Abort handles for in-flight page loads — lets navigation pause (abort)
    /// preloads of other pages so the current page always finishes first.
    pub page_aborts: std::collections::HashMap<usize, iced::task::Handle>,
    /// Cached comment-author profile pictures keyed by user id.
    pub comment_pfps: HashMap<u64, iced::widget::image::Handle>,
    /// Cached comment stamp images keyed by comment id.
    pub comment_stamps: HashMap<u64, iced::widget::image::Handle>,
    /// Whether an older-comments page is in flight (dedups scroll-triggered
    /// `CommentsLoadMore` requests for the same cursor).
    pub comments_loading: bool,
    /// The rendered (combined and/or rotated) image for `transformed.0`.
    /// Shown instead of `handle` while its key matches the wanted transform.
    pub transformed: Option<(
        crate::viewer_tools::TransformKey,
        iced::widget::image::Handle,
    )>,
    /// A transform render in flight.
    pub transform_pending: Option<crate::viewer_tools::TransformKey>,
    /// A transform render that failed (not retried automatically).
    pub transform_failed: Option<crate::viewer_tools::TransformKey>,
    /// The illust whose pages were (or are being) probed for a split image.
    pub split_probe: Option<u64>,
    /// Axis the detector found for the open illust (drives the hint).
    pub split_hint: Option<crate::viewer_tools::MergeAxis>,
}
impl ViewerState {
    /// Reset the viewer to its closed state. Every close path (Esc, close
    /// button, OS close) must go through here so no field can be forgotten.
    pub fn close(&mut self) {
        *self = Self::default();
    }
}

/// All artist-page state. The page works for every provider: a Pixiv user,
/// a Twitter/X user (their photo tweets) or a Bilibili user (their image
/// dynamics) — the provider rides on the namespaced id.
#[derive(Debug, Clone, Default)]
pub struct ArtistState {
    pub open: bool,
    pub id: u64,
    /// Which provider the author belongs to (drives fetching + pagination).
    pub provider: crate::media::Provider,
    pub illusts: Vec<Illustration>,
    pub user: Option<Author>,
    pub loading: bool,
    /// Pagination cursor: a Pixiv `next_url`, a Twitter cursor string or a
    /// Bilibili offset.
    pub next_url: Option<String>,
    /// Cached artist profile-picture handle.
    pub pfp: Option<iced::widget::image::Handle>,
}

impl ArtistState {
    /// Reset the artist page to its closed state (single close path).
    pub fn close(&mut self) {
        *self = Self::default();
    }
}
/// The shared image-CDN HTTP client + its rate limiter. Lives in state (not
/// global statics) so tests can substitute both, and so the configured proxy
/// can be applied at boot.
#[derive(Clone)]
pub struct ImageFetcher {
    pub http: Arc<reqwest::Client>,
    pub limiter: Arc<pixivj_rs::rate_limit::RateLimiter>,
    /// A dedicated limiter for thumbnail fetches, so a full-image download or
    /// pfp fetch backing off on a 429 never stalls the thumbnail pool (and
    /// vice versa). Pixiv's image CDN rate limit is per-host and lenient; the
    /// thumbnail pool bounds concurrency separately.
    pub thumbnail_limiter: Arc<pixivj_rs::rate_limit::RateLimiter>,
    /// Dedicated limiter for the viewer's CURRENT page. A preload or pfp
    /// burst that trips a 429 backoff window must never delay the page the
    /// user is actively looking at (that was the "detail view loads slower
    /// than downloads" bug).
    pub page_limiter: Arc<pixivj_rs::rate_limit::RateLimiter>,
    /// Dedicated limiter for viewer PRELOADS (background pages around the
    /// current one). Kept separate so a preload-induced backoff can't stall
    /// the current page, and so navigating (which aborts preloads) doesn't
    /// inherit their backoff windows.
    pub preload_limiter: Arc<pixivj_rs::rate_limit::RateLimiter>,
    /// Total thumbnail fetch attempts (incremented on every network miss).
    /// Surfaced in the debug overlay so repeated downloads are observable in
    /// normal usage.
    pub thumbnail_attempts: Arc<std::sync::atomic::AtomicU64>,
    /// Thumbnail cache hits (RAM + disk), also surfaced in the debug overlay.
    pub thumbnail_cache_hits: Arc<std::sync::atomic::AtomicU64>,
    /// Average RGB per illust id, computed as thumbnails are processed.
    /// Powers the gallery's "invert" tile-text style (tiles without an entry
    /// fall back to the plain theme color).
    pub thumbnail_avg: Arc<std::sync::Mutex<std::collections::HashMap<u64, [u8; 3]>>>,
}

impl Default for ImageFetcher {
    fn default() -> Self {
        Self {
            http: Arc::new(reqwest::Client::new()),
            limiter: Arc::new(pixivj_rs::rate_limit::RateLimiter::new()),
            thumbnail_limiter: Arc::new(pixivj_rs::rate_limit::RateLimiter::new()),
            page_limiter: Arc::new(pixivj_rs::rate_limit::RateLimiter::new()),
            preload_limiter: Arc::new(pixivj_rs::rate_limit::RateLimiter::new()),
            thumbnail_attempts: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            thumbnail_cache_hits: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            thumbnail_avg: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }
}

/// Image caches: the budgeted two-layer [`ImageCache`] plus the per-illust
/// handle/byte mirrors the view needs. Not `Debug` (the cache isn't).
#[derive(Clone, Default)]
pub struct ImageState {
    /// Cached `iced` image handles keyed by illust ID. Creating a `Handle`
    /// re-parses the image bytes and (on the first draw) re-uploads a
    /// texture — doing it in the `view()` every frame is the source of the
    /// UI freezes. Building it once here is the fix.
    pub thumbnail_handles: HashMap<u64, iced::widget::image::Handle>,
    /// Thumbnail cache (memory + disk LRU).
    pub image_cache: Arc<ImageCache>,
    /// The *processed* thumbnail cache (resized PNGs keyed by `url#target`).
    /// This is what grid-size / quality changes invalidate — the raw
    /// `image_cache` (URL → bytes) always survives, so re-processing reuses
    /// the downloaded bytes instead of hitting the network again.
    pub thumbnail_cache: Arc<ImageCache>,
    /// Illust ids enqueued into the thumbnail pool this session (or abandoned
    /// after exhausting retries). Guards the window scan from re-adding the
    /// same id on every scroll report.
    pub thumbnail_requested: std::collections::HashSet<u64>,
    /// The bounded thumbnail worker pool's pending queue: `(id, url, target)`.
    pub thumbnail_queue: Vec<(u64, String, u32)>,
    /// Illust ids currently being fetched by a pool worker (dedup + bounds
    /// the number of live fetches to [`THUMBNAIL_CONCURRENCY`]).
    pub thumbnail_in_flight: std::collections::HashSet<u64>,
    /// Abort handles for the pool's live fetches, keyed by illust ID — when
    /// a tile scrolls out of view before its fetch finishes, the task is
    /// aborted so it stops burning a concurrency slot and bandwidth.
    pub thumbnail_aborts: std::collections::HashMap<u64, iced::task::Handle>,
    /// The pixel target each id was last enqueued with — used to drop stale
    /// results after the grid size changes (a fetch started at the old
    /// resolution must not overwrite a fresh one).
    pub thumbnail_targets: std::collections::HashMap<u64, u32>,
    /// Per-illust fetch failure count; bounds automatic retries per session.
    pub thumbnail_retries: std::collections::HashMap<u64, u32>,
    /// Per-illust color palettes (MMCQ, ~5 colors) powering the sRGB/Hue
    /// sorts and the viewer swatches. Populated lazily from thumbnails and by
    /// "process all"; persisted centrally to `colors.json`.
    pub palettes: std::collections::HashMap<u64, Vec<[u8; 3]>>,
    /// Pending ids for the "process all colors" metadata pass `(id, url,
    /// target)`.
    pub color_queue: Vec<(u64, String, u32)>,
    /// Whether the "process all" metadata pass is running.
    pub color_processing: bool,
    /// Palettes processed so far in the running pass (progress bar).
    pub color_done: usize,
    /// Illusts enqueued by the running pass (progress bar denominator).
    pub color_total: usize,
    /// Color-pipeline workers currently fetching + extracting. Bounds the pool
    /// (they share the thumbnail client) instead of letting every completion
    /// spawn a fresh burst.
    pub color_in_flight: usize,
    /// The color map changed since the last write to disk.
    pub colors_dirty: bool,
    /// The shared image-CDN HTTP client + rate limiters + counters.
    pub fetcher: ImageFetcher,
}

/// A shared cancellation switch for long-running jobs. The UI sets it; the
/// job's stream checks it between steps. Wraps an [`std::sync::atomic::AtomicBool`]
/// so `JobsState` keeps its derived traits.
#[derive(Clone, Default)]
pub struct CancelFlag(Arc<std::sync::atomic::AtomicBool>);

impl CancelFlag {
    /// Signal cancellation.
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether cancellation was signalled.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl std::fmt::Debug for CancelFlag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CancelFlag({})", self.is_cancelled())
    }
}

/// Download/library/job state (persisted + in-memory mirrors).
#[derive(Debug, Clone, Default)]
pub struct JobsState {
    pub downloads: crate::io::download::DownloadTracker,
    /// Per-illust download speeds (bytes/sec) for the aggregate speed readout.
    pub download_speeds: HashMap<u64, f64>,
    /// The locally-mirrored bookmark library (persisted on disk).
    pub library: crate::io::library::BookmarkLibrary,
    /// Where the library file lives.
    pub library_path: std::path::PathBuf,
    /// Where the download-all job queue lives.
    pub download_queue_path: std::path::PathBuf,
    /// In-memory mirror of the persisted download queue.
    pub download_queue: crate::io::library::DownloadQueue,
    /// Bookmark sync job status (for the progress bar).
    pub bookmark_sync: crate::io::library::SyncJobState,
    /// Cancellation switch for the running bookmark sync (checked between
    /// pages; already-saved pages are kept).
    pub sync_cancel: CancelFlag,
    /// Download-all job status (for the progress bar).
    pub download_job: crate::io::library::DownloadJobState,
    /// The in-memory download queue changed since the last write to disk
    /// (flushed together with the config by the debounced writer).
    pub queue_dirty: bool,
    /// The bookmark library changed (locally toggled Bilibili bookmarks) and
    /// must be written on the next debounced flush.
    pub library_dirty: bool,
}

#[derive(Clone, Default)]
pub struct AppState {
    pub phase: AppPhase,
    pub active_tab: Tab,
    pub tabs: TabsState,
    pub settings: Settings,
    pub client: Option<PixivApiClient>,
    /// Twitter client + connection state (cookies-based).
    pub twitter: TwitterState,
    /// Bilibili client + feed state (cookies-based).
    pub bilibili: BilibiliState,

    /// Onboarding flow state (server, PKCE, submitted token).
    pub onboarding: OnboardingState,
    /// Viewer overlay state.
    pub viewer: ViewerState,
    /// Artist page state.
    pub artist: ArtistState,
    /// Saved display transforms (combine/rotate) per Twitter item, keyed by
    /// namespaced id; persisted to `viewer_transforms.json`.
    pub view_transforms: HashMap<u64, crate::viewer_tools::ViewTransform>,
    /// Image bytes/handles caches.
    pub images: ImageState,
    /// Downloads, library and job state.
    pub jobs: JobsState,

    /// The window id captured from the runtime, used to close the window.
    pub window_id: Option<iced::window::Id>,
    /// Last known inner window size in logical pixels.
    pub window_size: (f32, f32),
    /// The window's DPI scale factor (1.0 = 100%). Used to size thumbnails so
    /// they're sharp on hi-DPI displays (physical pixels = logical × scale).
    pub scale_factor: f32,
    /// Resolved system theme mode (dark/light) reported by the OS.
    pub theme_mode: iced::theme::Mode,
    /// Whether the advanced filter controls are expanded (default collapsed).
    pub show_advanced: bool,
    /// Whether the help screen is open (the "?" button / usage + hotkeys +
    /// author + license page).
    pub show_help: bool,
    /// Whether the tag-filter text input is focused (the tag-suggestion
    /// dropdown is shown only while it is).
    pub tag_input_focused: bool,
    /// Whether the tag-suggestion dropdown is currently visible. Decoupled
    /// from `tag_input_focused` so a candidate click (which briefly blurs the
    /// input) doesn't hide the dropdown before the click registers.
    pub tag_dropdown_open: bool,
    /// Whether the floating debug overlay is visible (F3).
    pub debug_overlay: bool,
    /// Whether the settings screen is open.
    pub settings_open: bool,
    /// Whether the download-manager panel is open.
    pub download_manager_open: bool,
    /// Settings changed since the last write to disk (debounced flush).
    pub config_dirty: bool,
    /// The proxy the running clients were built with (`None` = direct).
    /// Compared against `settings.proxy` to hot-apply changes without an
    /// app restart.
    pub applied_proxy: Option<String>,
    /// The most recent user-facing error (shown as a banner in the view).
    pub last_error: Option<String>,
    /// Whether the active gallery is scrolled to the bottom (rising-edge
    /// triggers a single LoadMore; reset when scrolled back up).
    pub scroll_at_bottom: bool,
    /// Mirror of the gallery's scroll offset (content coords), reported by
    /// the virtual gallery so the view can build only visible rows.
    pub gallery_scroll: f32,
    /// The visible content window `(start, end)` of the gallery, reported on
    /// every scroll — drives lazy (visibility-based) thumbnail loading.
    pub gallery_window: Option<(f32, f32)>,
    /// Autocomplete tag suggestions for the search tab.
    pub autocomplete_tags: Vec<String>,
    /// Keyboard-highlighted autocomplete entry (↑/↓), if any.
    pub autocomplete_selected: Option<usize>,
    /// Whether the keyboard-shortcuts overlay is visible (F1).
    pub show_shortcuts: bool,
    /// Whether a shift key is currently held (shift-click on a gallery tile
    /// toggles its bookmark instead of opening the viewer).
    pub shift_pressed: bool,
    /// Distinct tag names in the active tab's illustrations (tag-filter dropdown).
    pub gallery_tags: Vec<String>,
    /// Distinct tool names in the active tab's illustrations (tool filter UI).
    pub gallery_tools: Vec<String>,
    /// Combo-box state for the tag filter (dropdown of current gallery tags).
    pub tag_combo: iced::widget::combo_box::State<String>,
}

// Manual Debug impl to skip ImageCache (summarizes the image maps by size).
impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("phase", &self.phase)
            .field("active_tab", &self.active_tab)
            .field("settings", &self.settings)
            .field("client", &self.client.is_some())
            .field("onboarding", &self.onboarding)
            .field("viewer_open", &self.viewer.open)
            .field("viewer_illust_id", &self.viewer.illust_id)
            .field("artist_open", &self.artist.open)
            .field(
                "thumbnail_handles_count",
                &self.images.thumbnail_handles.len(),
            )
            .field("theme_mode", &self.theme_mode)
            .field("settings_open", &self.settings_open)
            .field("download_queue_done", &self.jobs.download_queue.done())
            .finish()
    }
}

impl AppState {
    /// The gallery tab for `tab` (immutable).
    pub fn tab(&self, tab: Tab) -> &GalleryTab {
        self.tabs.tab(tab)
    }

    /// The gallery tab for `tab` (mutable).
    pub fn tab_mut(&mut self, tab: Tab) -> &mut GalleryTab {
        self.tabs.tab_mut(tab)
    }

    /// The active gallery tab (immutable).
    pub fn active_tab_data(&self) -> &GalleryTab {
        self.tab(self.active_tab)
    }

    /// The active gallery tab (mutable).
    pub fn active_tab_data_mut(&mut self) -> &mut GalleryTab {
        let tab = self.active_tab;
        self.tab_mut(tab)
    }

    /// Re-apply the active tab's filters + sort to its illustrations.
    /// This is what makes sort/filter changes take effect on the list.
    pub fn reapply_active_tab(&mut self) {
        let tab = self.active_tab;
        let palettes = &self.images.palettes;
        self.tabs.tab_mut(tab).reapply(palettes);
    }
}
