use crate::app::filter::OptionFilterKey;
use crate::app::sort::SortKey;
use crate::app::state::{Tab, ThemePreference};
use pixivj_rs::api::PixivApiClient;
use pixivj_rs::error::PixivError;
use pixivj_rs::model::{
    Autocomplete, Comments, IllustDetail, RankedIllusts, RecommendedIllusts, SearchedIllusts,
    UserDetail, UserIllusts,
};

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Message {
    // ── Onboarding ──────────────────────────────────────────────────
    /// Start the onboarding flow (server + instructions).
    StartOnboarding,
    /// Local server started, URL is ready.
    OnboardingServerReady(String),
    /// User submitted token via the web form.
    TokenSubmitted(String),
    /// User typed in the token input field.
    TokenInputChanged(String),
    /// User clicked the URL to open it in browser.
    OpenUrl(String),
    /// User clicked "Choose Directory" — open native file dialog.
    PickDirectory,
    /// Token validation result from Pixiv API: `Ok(refresh_token)` on success.
    TokenValidated(Result<String, PixivError>),
    /// PKCE auth-code exchange result: `Ok(refresh_token)` on success.
    CodeExchanged(Result<String, PixivError>),
    /// User selected gallery directory.
    DirectorySelected(Option<String>),
    /// Complete onboarding, enter main app.
    OnboardingComplete,

    // ── Client building ──────────────────────────────────────
    /// Client built successfully after onboarding or boot.
    ClientBuilt(Result<PixivApiClient, PixivError>),

    // ── Tab management ─────────────────────────────────────────────
    TabSelected(Tab),
    LoadMore,
    ResetTab,
    /// The gallery scroll reached/left the bottom (rising-edge triggers load).
    ScrollPositionChanged(bool),
    /// The visible content range `(start, end)` of the gallery changed —
    /// drives lazy (visibility-based) thumbnail loading.
    GalleryWindowChanged {
        start: f32,
        end: f32,
    },
    /// Rescan the gallery directory and update every tab's "downloaded on
    /// disk" set (drives the Missing filter and per-tile checkmarks).
    RefreshDownloaded,
    /// Pace the lazy thumbnail loader: a window had more pending items than
    /// the per-tick budget, so keep loading until it is fully requested.
    ThumbnailWindowPulse,
    /// Scan the gallery directory for local (non-bookmarked) illusts — the
    /// Extra tab's data source.
    ScanExtra,
    /// Result of a local gallery scan: the synthetic illusts + their dirs.
    ExtraScanned(
        Result<
            (
                Vec<pixivj_rs::model::Illustration>,
                std::collections::HashMap<u64, std::path::PathBuf>,
            ),
            String,
        >,
    ),
    /// A local image file was loaded as a thumbnail `(id, result)`.
    LocalThumbnailLoaded(u64, Result<Vec<u8>, String>),
    /// An illust's MMCQ color palette was extracted `(id, palette)`.
    PaletteExtracted(u64, Vec<[u8; 3]>),
    /// A "process all" worker finished one illust `(id, palette)`.
    ColorPassPalette(u64, Vec<[u8; 3]>),
    /// Fill in missing metadata (color palettes) for all bookmark illusts.
    ProcessAllColors,
    /// The user picked a color for the sRGB color sort.
    ColorSortChanged([u8; 3]),
    /// The user changed the color-sort level (1–5).
    ColorLevelChanged(u8),
    /// The user pressed Enter in the search bar (runs a Pixiv search).
    SearchSubmitted,
    /// Server-side Pixiv sort order changed (`None` = API default).
    ApiSortChanged(Option<pixivj_rs::model::SortOption>),
    /// Server-side Pixiv match mode changed (`None` = API default).
    ApiTargetChanged(Option<pixivj_rs::model::SearchTarget>),
    /// Include translated tag matches in Pixiv searches.
    ApiTranslatedChanged(bool),
    /// Toggle continuous search loading: keeps fetching more result pages
    /// until the query is exhausted (or the toggle is turned off).
    ToggleAutoLoad,
    /// Autocomplete suggestions for a search query `(query, result)`.
    AutocompleteLoaded(String, Result<Autocomplete, PixivError>),
    /// User picked an autocomplete tag for the search query.
    AutocompleteTagClicked(String),
    /// Move the autocomplete highlight (keyboard ↑/↓; wraps around).
    AutocompleteMove(i32),
    /// Enter in the search box: pick the highlighted suggestion, or run the
    /// query when none is highlighted.
    AutocompleteCommit,
    /// Focus the search input (Ctrl+F).
    FocusSearch,
    /// Toggle the keyboard-shortcuts overlay (F1).
    ToggleShortcuts,
    /// The viewer page index settled; preload nearby pages.
    ViewerPageSettled,
    /// Fetch the next page of the artist's illustrations (infinite scroll).
    ArtistLoadMore,
    /// Fetch older comments for the viewer illust (cursor pagination).
    CommentsLoadMore,
    /// A subsequent page of comments arrived (appended to the existing list).
    CommentsPageFetched(Result<Comments, PixivError>),

    // ── Twitter ────────────────────────────────────────────────────
    /// Twitter client built + cookies verified (`Ok` carries the client and
    /// the logged-in account).
    TwitterClientBuilt(
        Result<crate::app::update::twitter::TwitterBoot, crate::twitter::TwitterError>,
    ),
    /// A page of the Twitter home timeline (For You / Following). Carries the
    /// mode it was fetched for so a late response for the previous mode is
    /// dropped after a toggle.
    TwitterFeedFetched(
        crate::twitter::TwitterFeedMode,
        Result<crate::twitter::TwitterPage, crate::twitter::TwitterError>,
    ),
    /// A page of the account's liked tweets (mixed Bookmarks tab).
    TwitterLikesFetched(Result<crate::twitter::TwitterPage, crate::twitter::TwitterError>),
    /// A page of a Twitter user's photo tweets (the author page).
    TwitterArtistLoaded(
        u64,
        Result<crate::twitter::TwitterPage, crate::twitter::TwitterError>,
    ),
    /// The user switched the Twitter tab's timeline.
    TwitterFeedModeChanged(crate::twitter::TwitterFeedMode),
    /// Clear the Twitter tab and reload the current timeline.
    TwitterFeedReload,
    /// A single tweet fetched by id (viewer opened from the library).
    TwitterTweetFetched(
        u64,
        Result<pixivj_rs::model::Illustration, crate::twitter::TwitterError>,
    ),
    /// First page of a tweet's replies (mapped to comments).
    TwitterRepliesFetched(Result<pixivj_rs::model::Comments, crate::twitter::TwitterError>),
    /// A subsequent page of a tweet's replies (cursor pagination).
    TwitterRepliesPageFetched(Result<pixivj_rs::model::Comments, crate::twitter::TwitterError>),
    /// Result of the like/unlike request.
    TwitterLikeResult(u64, Result<(), crate::twitter::TwitterError>),
    /// The user typed a new cookies-file path (does not reload yet).
    TwitterCookiePathChanged(String),
    /// Re-read the cookies file and rebuild the Twitter client.
    ReloadTwitterCookies,
    /// Fetch the current GraphQL query ids from X's web bundles, persist them
    /// to `twitter_graphql.json` and reload the client.
    RefreshTwitterQueryIds,
    /// Result of a query-id refresh: the merged config, or the fetch error.
    TwitterQueryIdsFetched(
        Result<crate::twitter::graphql::GraphQlConfig, crate::twitter::TwitterError>,
    ),

    // ── Bilibili ───────────────────────────────────────────────────
    /// Bilibili client built + cookies verified.
    BilibiliClientBuilt(
        Result<crate::app::update::bilibili::BilibiliBoot, crate::bilibili::BilibiliError>,
    ),
    /// A page of the followed-users dynamic feed.
    BilibiliFeedFetched(
        crate::bilibili::BilibiliFeedMode,
        Result<crate::bilibili::BilibiliPage, crate::bilibili::BilibiliError>,
    ),
    /// A page of one configured user's dynamic feed.
    BilibiliUserFetched(
        u64,
        Result<crate::bilibili::BilibiliPage, crate::bilibili::BilibiliError>,
    ),
    /// A page of one user's dynamic feed (the author page).
    BilibiliArtistLoaded(
        u64,
        Result<crate::bilibili::BilibiliPage, crate::bilibili::BilibiliError>,
    ),
    /// The user switched the Bilibili tab's feed mode.
    BilibiliFeedModeChanged(crate::bilibili::BilibiliFeedMode),
    /// Clear the Bilibili tab and reload the current feed.
    BilibiliFeedReload,
    /// A single dynamic fetched by id (viewer opened from the library).
    BilibiliDynamicFetched(
        u64,
        Result<pixivj_rs::model::Illustration, crate::bilibili::BilibiliError>,
    ),
    /// First page of a dynamic's comments.
    BilibiliCommentsFetched(Result<pixivj_rs::model::Comments, crate::bilibili::BilibiliError>),
    /// A subsequent page of a dynamic's comments (page pagination).
    BilibiliCommentsPageFetched(Result<pixivj_rs::model::Comments, crate::bilibili::BilibiliError>),
    /// Result of the best-effort like/unlike request (the local bookmark was
    /// already toggled).
    BilibiliLikeResult(u64, Result<(), crate::bilibili::BilibiliError>),
    /// The user typed a new cookies-file path (does not reload yet).
    BilibiliCookiePathChanged(String),
    /// The user edited the fixed user-id list.
    BilibiliUserIdsChanged(String),
    /// Re-read the cookies file and rebuild the Bilibili client.
    ReloadBilibiliCookies,

    // ── Bookmark library / download queue ───────────────────────────
    /// Run a full bookmark library sync (manual; incremental auto-runs at boot).
    StartBookmarkSync,
    /// Cancel the running bookmark sync at the next page boundary (pages
    /// already saved stay saved).
    CancelBookmarkSync,
    /// Stop the download-all job: no new items are started; in-flight ones
    /// finish so no partial files are left behind.
    CancelDownloadAll,
    /// Progress of the bookmark sync job: bookmarks loaded so far / total
    /// (when known), plus the illusts newly merged in this page (so the
    /// gallery can expand live).
    BookmarkSyncProgress {
        loaded: usize,
        total: Option<usize>,
        new: Vec<pixivj_rs::model::Illustration>,
    },
    /// The bookmark sync job finished. `total` = pages processed; `error` =
    /// set when the sync ended in failure; `library` is the final in-memory
    /// library so the UI never re-reads the file.
    BookmarkSyncFinished {
        total: usize,
        error: Option<String>,
        library: crate::io::library::BookmarkLibrary,
    },
    /// Enqueue all local library bookmarks into the shared download queue.
    StartDownloadAll,
    /// Toggle the floating debug overlay.
    ToggleDebugOverlay,

    // ── API results ────────────────────────────────────────────────
    RecommendedFetched(Result<RecommendedIllusts, PixivError>),
    RankedFetched(Result<RankedIllusts, PixivError>),
    FollowFeedFetched(Result<SearchedIllusts, PixivError>),
    BookmarksFetched(Result<SearchedIllusts, PixivError>),
    SearchFetched(Result<SearchedIllusts, PixivError>),
    IllustDetailFetched(Result<IllustDetail, PixivError>),
    CommentsFetched(Result<Comments, PixivError>),
    UserDetailFetched(Result<UserDetail, PixivError>),
    UserIllustsFetched(Result<UserIllusts, PixivError>),

    // ── Filter changes ─────────────────────────────────────────────
    SearchTextChanged(String),
    OptionFilterToggled(OptionFilterKey),
    /// Toggle which fields the search box scans (tags/author/title/caption/id).
    SearchTargetChanged(crate::app::filter::SearchTargetKey, bool),
    /// Text typed in the tag-filter input (separate from the search box).
    TagInputChanged(String),
    TagAdded(String),
    TagRemoved(String),
    ToolFilterToggled(String),
    /// Toggle one side of the Bookmarks tab's source filter (Pixiv / Twitter
    /// / Bilibili chips). Applies wherever the tab's filter state is used.
    SourceFilterToggled(crate::app::filter::SourceKey),
    ResetFilters,
    /// Show/hide the advanced filter controls (default: collapsed).
    ToggleAdvancedFilters,

    // ── Sort changes ───────────────────────────────────────────────
    SortKeyClicked(SortKey),
    MultiSortToggled,

    // ── Leaderboard (ranking) ──────────────────────────────────────
    /// The user changed the leaderboard mode (daily/weekly/monthly/…).
    RankingModeChanged(pixivj_rs::model::FilterMode),
    /// The user typed a leaderboard date override (YYYY-MM-DD).
    RankingDateChanged(String),

    // ── Tag filter focus ───────────────────────────────────────────
    /// The tag-filter text input gained/lost focus (drives whether the
    /// tag-suggestion dropdown is visible).
    TagFocusChanged(bool),
    /// Fired a short while after the tag input loses focus — closes the
    /// suggestion dropdown, but only after a grace window so a click on a
    /// candidate (which blurs the input first) still registers.
    TagFocusGrace,

    // ── Gallery interactions ───────────────────────────────────────
    GalleryItemClicked(u64),
    GalleryItemDoubleClicked(u64),
    /// A gallery tile gained/lost the cursor (drives the R-18 blur's
    /// un-blur-on-hover).
    GalleryItemHover(u64, bool),

    // ── Image loading ─────────────────────────────────────────────
    /// Thumbnail bytes loaded for an illust `(id, url, target, result)`. The
    /// URL lets a failed fetch be re-queued; the `target` (the pixel size it
    /// was generated at) lets the handler drop stale results after a grid
    /// resize.
    ThumbnailLoaded(u64, String, u32, Result<Vec<u8>, PixivError>),
    /// Progress of the full-size image download for `(illust, page)`, 0..1.
    FullImageProgress(u64, usize, f32),
    /// A partially-downloaded image decoded successfully (progressive JPEGs
    /// can be displayed before the whole file arrives) `(illust, page,
    /// handle)`.
    FullImagePartial(u64, usize, iced::widget::image::Handle),
    /// Full image bytes loaded for the viewer `(illust, page)`.
    FullImageLoaded(u64, usize, Result<Vec<u8>, PixivError>),

    // ── Viewer ─────────────────────────────────────────────────────
    ViewerOpened(u64),
    ViewerClosed,
    ViewerPageChanged(usize),
    /// Step the viewer page by a delta (browser back/forward shortcuts).
    ViewerPageStep(i32),

    // ── Download manager ────────────────────────────────────────────
    /// Open/close the download-manager panel (queue, status, priorities).
    ToggleDownloadManager,
    /// Raise an item's download priority (move it earlier in the queue).
    DownloadPriorityUp(u64),
    /// Lower an item's download priority (move it later in the queue).
    DownloadPriorityDown(u64),
    /// Relative/clamped page navigation (keyboard ←/→/Home/End).
    ViewerPageNavigate(crate::app::state::ViewerPageNav),
    /// Text typed into the viewer's jump-to-page box.
    ViewerPageInputChanged(String),
    /// Enter pressed in the jump-to-page box.
    ViewerPageSubmitted,

    // ── Bookmark/follow/download ───────────────────────────────────
    BookmarkToggled(u64),
    BookmarkResult(u64, Result<(), PixivError>),
    /// Fresh illust detail after a bookmark toggle (to update the count).
    IllustDetailRefreshed(u64, IllustDetail),
    FollowToggled(u64),
    FollowResult(u64, Result<(), PixivError>),
    DownloadStarted(u64),
    /// Progress of a download `(illust_id, fraction, bytes_per_sec)`.
    DownloadProgress(u64, f32, f64),
    DownloadCompleted(u64, Result<(), PixivError>),

    // ── Window ─────────────────────────────────────────────────────
    /// Captured the runtime window id (for `iced::window::close`).
    WindowRegistered(iced::window::Id),
    /// The window was closed by the user (system close button).
    WindowClosed,
    /// OS close requested for the window `id`; update() decides whether to
    /// close the app or just the current overlay.
    WindowCloseRequested(iced::window::Id),
    /// The window's DPI scale factor changed (drives sharp thumbnail sizing).
    ScaleFactorChanged(f32),
    /// A modifier key changed state (only Shift is tracked: it turns a
    /// gallery click into a bookmark toggle).
    ModifiersChanged(bool),
    /// The Esc key was pressed; update() closes the topmost overlay.
    EscapePressed,

    // ── Window resize ──────────────────────────────────────────────
    WindowResized {
        width: f32,
        height: f32,
    },

    // ── Theme ──────────────────────────────────────────────────────
    /// Initial system theme query result.
    SystemThemeFetched(iced::theme::Mode),
    /// The OS theme changed (dark/light).
    SystemThemeChanged(iced::theme::Mode),
    /// User changed the dark/light preference.
    ThemePreferenceChanged(ThemePreference),

    // ── Artist ─────────────────────────────────────────────────────
    ArtistOpened(u64),
    ArtistClosed,
    /// The artist's profile picture bytes loaded.
    ArtistPfpLoaded(Result<Vec<u8>, PixivError>),
    /// A comment author's profile picture loaded `(user_id, bytes)`.
    CommentPfpLoaded(u64, Result<Vec<u8>, PixivError>),
    /// A comment's stamp image loaded `(comment_id, bytes)`.
    CommentStampLoaded(u64, Result<Vec<u8>, PixivError>),

    // ── Browser ────────────────────────────────────────────────────
    OpenInBrowser(u64),
    /// Copy the artwork's pixiv web URL to the system clipboard.
    CopyIllustUrl(u64),
    /// Open the illust's local download folder in the OS file manager.
    RevealIllustFiles(u64),
    /// A tag chip was clicked: jump to the Search tab and search that tag.
    TagSearchRequested(String),

    // ── Settings ───────────────────────────────────────────────────
    GridSizeChanged(u32),
    /// Step the grid size by a delta (positive/negative; `0` resets to
    /// default). Used by the Ctrl+/-/0 shortcuts.
    GridSizeStep(i32),
    /// Open/close the settings screen.
    OpenSettings,
    CloseSettings,
    /// Open/close the help screen (usage, hotkeys, author, license).
    ToggleHelp,
    /// How bookmarked items are treated outside the Bookmarks tab.
    BookmarkedDisplayChanged(crate::app::state::BookmarkedDisplay),
    /// Blur R-18 tiles until hovered.
    BlurR18Changed(bool),
    /// Toggle one info row of the gallery tiles.
    TileRowsChanged(crate::app::state::TileRowKey, bool),
    /// The memory/quality preset changed (cache budgets).
    MemoryPresetChanged(crate::app::state::MemoryPreset),
    /// Viewer preload-ahead page count changed.
    AutoPreloadChanged(u32),
    /// Download-all concurrency changed.
    DownloadConcurrencyChanged(usize),
    /// The user typed a new gallery path.
    GalleryPathInputChanged(String),
    /// The user typed a new proxy address.
    ProxyInputChanged(String),
    /// Master parallax toggle.
    ParallaxChanged(bool),
    /// Mouse (hover) parallax toggle.
    ParallaxMouseChanged(bool),
    /// Scroll-position parallax toggle.
    ParallaxScrollChanged(bool),
    /// High-resolution thumbnails toggle (invalidates the processed cache).
    ThumbnailHighResChanged(bool),
    /// The scroll-parallax curve changed (centered vs linear).
    ParallaxModeChanged(crate::widgets::parallax_image::ParallaxMode),
    /// The gallery tile text's base font size changed.
    GalleryFontSizeChanged(u32),
    /// The gallery tile text style changed (plain / invert / background).
    GalleryTextStyleChanged(crate::widgets::illust_info::GalleryTextStyle),
    /// Reset a single setting to its default (never touches the token).
    ResetSetting(crate::app::state::SettingsKey),
    /// Reset all settings to their defaults (never touches the token).
    ResetAllSettings,

    // ── Error ──────────────────────────────────────────────────────
    /// Report an error; shown as a dismissible banner above all overlays.
    Error(String),
    /// Dismiss the current error banner.
    DismissError,

    // ── Proxy hot-apply ────────────────────────────────────────────
    /// Debounced check: if `settings.proxy` differs from the proxy the
    /// clients were built with, rebuild them (no restart needed).
    ApplyProxy,
    /// Result of rebuilding the API client after a proxy change. On error
    /// the previous client keeps working and the failure is surfaced.
    ClientProxied(Result<PixivApiClient, PixivError>),

    // ── Misc ────────────────────────────────────────────────────────
    /// No-op message (e.g. scroll notifications that don't need handling).
    Noop,
    /// Debounced disk flush: persist settings + the download queue if they
    /// changed since the last write. Keeps per-keystroke input and per-item
    /// queue updates from writing disk on every event.
    FlushDisk,
}
