//! Profiling telemetry.
//!
//! With the `profiling` feature enabled this adds:
//!
//! * [`memory`] — a tagged `#[global_allocator]` attributing allocation churn
//!   to the component that was active when the allocation happened.
//! * [`cpu`] — a `pprof-rs` sampling profiler whose capture can be triggered
//!   over HTTP (agent-friendly) or directly.
//! * [`report`] — the markdown/JSON report writer that answers the seven
//!   profiling questions.
//! * [`http`] — a tiny control server for the running GUI app so an agent can
//!   start/stop a CPU capture and pull a flamegraph with `curl`.
//!
//! With the feature **off**, every function degrades to a transparent no-op
//! (e.g. [`timed_update`] just calls `update`), so call sites stay clean and
//! normal builds pay zero cost.

#[cfg(feature = "profiling")]
pub mod cpu;
#[cfg(feature = "profiling")]
pub mod http;
#[cfg(feature = "profiling")]
pub mod memory;
#[cfg(feature = "profiling")]
pub mod report;

#[cfg(feature = "profiling")]
pub use pixivj_rs::tracing;
#[cfg(feature = "profiling")]
pub use pixivj_rs::tracing::{FileOp, PhaseSample, Snapshot};

use crate::app::message::Message;
use crate::app::state::AppState;
#[cfg(feature = "profiling")]
use std::time::Instant;

/// A short, stable name for a message, used as the frametime/alloc bucket.
pub fn message_name(message: &Message) -> &'static str {
    use Message::*;
    match message {
        StartOnboarding => "StartOnboarding",
        OnboardingServerReady(_) => "OnboardingServerReady",
        TokenSubmitted(_) => "TokenSubmitted",
        TokenInputChanged(_) => "TokenInputChanged",
        OpenUrl(_) => "OpenUrl",
        PickDirectory => "PickDirectory",
        TokenValidated(_) => "TokenValidated",
        CodeExchanged(_) => "CodeExchanged",
        DirectorySelected(_) => "DirectorySelected",
        OnboardingComplete => "OnboardingComplete",
        ClientBuilt(_) => "ClientBuilt",
        TabSelected(_) => "TabSelected",
        LoadMore => "LoadMore",
        ResetTab => "ResetTab",
        ScrollPositionChanged(_) => "ScrollPositionChanged",
        GalleryWindowChanged { .. } => "GalleryWindowChanged",
        ThumbnailWindowPulse => "ThumbnailWindowPulse",
        RefreshDownloaded => "RefreshDownloaded",
        ScanExtra => "ScanExtra",
        ExtraScanned(_) => "ExtraScanned",
        LocalThumbnailLoaded(..) => "LocalThumbnailLoaded",
        SearchSubmitted => "SearchSubmitted",
        ApiSortChanged(_) => "ApiSortChanged",
        ApiTargetChanged(_) => "ApiTargetChanged",
        ApiTranslatedChanged(_) => "ApiTranslatedChanged",
        ToggleAutoLoad => "ToggleAutoLoad",
        AutocompleteLoaded(..) => "AutocompleteLoaded",
        AutocompleteTagClicked(_) => "AutocompleteTagClicked",
        AutocompleteMove(_) => "AutocompleteMove",
        AutocompleteCommit => "AutocompleteCommit",
        FocusSearch => "FocusSearch",
        ToggleShortcuts => "ToggleShortcuts",
        ViewerPageSettled => "ViewerPageSettled",
        ArtistLoadMore => "ArtistLoadMore",
        CommentsLoadMore => "CommentsLoadMore",
        CommentsPageFetched(_) => "CommentsPageFetched",
        StartBookmarkSync => "StartBookmarkSync",
        CancelBookmarkSync => "CancelBookmarkSync",
        CancelDownloadAll => "CancelDownloadAll",
        BookmarkSyncProgress { .. } => "BookmarkSyncProgress",
        BookmarkSyncFinished { .. } => "BookmarkSyncFinished",
        StartDownloadAll => "StartDownloadAll",
        ToggleDownloadManager => "ToggleDownloadManager",
        DownloadPriorityUp(_) => "DownloadPriorityUp",
        DownloadPriorityDown(_) => "DownloadPriorityDown",
        ToggleDebugOverlay => "ToggleDebugOverlay",
        RecommendedFetched(_) => "RecommendedFetched",
        RankedFetched(_) => "RankedFetched",
        FollowFeedFetched(_) => "FollowFeedFetched",
        BookmarksFetched(_) => "BookmarksFetched",
        SearchFetched(_) => "SearchFetched",
        IllustDetailFetched(_) => "IllustDetailFetched",
        CommentsFetched(_) => "CommentsFetched",
        UserDetailFetched(_) => "UserDetailFetched",
        UserIllustsFetched(_) => "UserIllustsFetched",
        TwitterClientBuilt(_) => "TwitterClientBuilt",
        TwitterFeedFetched(..) => "TwitterFeedFetched",
        TwitterLikesFetched(_) => "TwitterLikesFetched",
        TwitterFeedModeChanged(_) => "TwitterFeedModeChanged",
        TwitterFeedReload => "TwitterFeedReload",
        TwitterTweetFetched(..) => "TwitterTweetFetched",
        TwitterRepliesFetched(_) => "TwitterRepliesFetched",
        TwitterRepliesPageFetched(_) => "TwitterRepliesPageFetched",
        TwitterLikeResult(..) => "TwitterLikeResult",
        TwitterCookiePathChanged(_) => "TwitterCookiePathChanged",
        ReloadTwitterCookies => "ReloadTwitterCookies",
        RefreshTwitterQueryIds => "RefreshTwitterQueryIds",
        TwitterQueryIdsFetched(_) => "TwitterQueryIdsFetched",
        BilibiliClientBuilt(_) => "BilibiliClientBuilt",
        BilibiliFeedFetched(..) => "BilibiliFeedFetched",
        BilibiliUserFetched(..) => "BilibiliUserFetched",
        BilibiliFeedModeChanged(_) => "BilibiliFeedModeChanged",
        BilibiliFeedReload => "BilibiliFeedReload",
        BilibiliDynamicFetched(..) => "BilibiliDynamicFetched",
        BilibiliCommentsFetched(_) => "BilibiliCommentsFetched",
        BilibiliCommentsPageFetched(_) => "BilibiliCommentsPageFetched",
        BilibiliLikeResult(..) => "BilibiliLikeResult",
        BilibiliCookiePathChanged(_) => "BilibiliCookiePathChanged",
        BilibiliUserIdsChanged(_) => "BilibiliUserIdsChanged",
        ReloadBilibiliCookies => "ReloadBilibiliCookies",
        SearchTextChanged(_) => "SearchTextChanged",
        SearchTargetChanged(..) => "SearchTargetChanged",
        OptionFilterToggled(_) => "OptionFilterToggled",
        TagInputChanged(_) => "TagInputChanged",
        TagAdded(_) => "TagAdded",
        TagRemoved(_) => "TagRemoved",
        ToolFilterToggled(_) => "ToolFilterToggled",
        ResetFilters => "ResetFilters",
        ToggleAdvancedFilters => "ToggleAdvancedFilters",
        SortKeyClicked(_) => "SortKeyClicked",
        MultiSortToggled => "MultiSortToggled",
        RankingModeChanged(_) => "RankingModeChanged",
        RankingDateChanged(_) => "RankingDateChanged",
        TagFocusChanged(_) => "TagFocusChanged",
        TagFocusGrace => "TagFocusGrace",
        PaletteExtracted(..) => "PaletteExtracted",
        ColorPassPalette(..) => "ColorPassPalette",
        ProcessAllColors => "ProcessAllColors",
        ColorSortChanged(_) => "ColorSortChanged",
        FilterModeChanged(_) => "FilterModeChanged",
        QueryTokenRemoved(_) => "QueryTokenRemoved",
        QuerySuggestionClicked(_) => "QuerySuggestionClicked",
        CustomColorSelected => "CustomColorSelected",
        CustomColorTextChanged(_) => "CustomColorTextChanged",
        ColorLevelChanged(_) => "ColorLevelChanged",
        GalleryItemClicked(_) => "GalleryItemClicked",
        GalleryItemDoubleClicked(_) => "GalleryItemDoubleClicked",
        ThumbnailLoaded(..) => "ThumbnailLoaded",
        FullImageProgress(..) => "FullImageProgress",
        FullImagePartial(..) => "FullImagePartial",
        FullImageLoaded(..) => "FullImageLoaded",
        ViewerOpened(_) => "ViewerOpened",
        ViewerClosed => "ViewerClosed",
        ViewerPageChanged(_) => "ViewerPageChanged",
        ViewerPageStep(_) => "ViewerPageStep",
        ViewerPageNavigate(_) => "ViewerPageNavigate",
        ViewerPageInputChanged(_) => "ViewerPageInputChanged",
        ViewerPageSubmitted => "ViewerPageSubmitted",
        BookmarkToggled(_) => "BookmarkToggled",
        BookmarkResult(..) => "BookmarkResult",
        IllustDetailRefreshed(..) => "IllustDetailRefreshed",
        FollowToggled(_) => "FollowToggled",
        FollowResult(..) => "FollowResult",
        DownloadStarted(_) => "DownloadStarted",
        DownloadProgress(..) => "DownloadProgress",
        DownloadCompleted(..) => "DownloadCompleted",
        WindowRegistered(_) => "WindowRegistered",
        WindowClosed => "WindowClosed",
        WindowCloseRequested(_) => "WindowCloseRequested",
        ScaleFactorChanged(_) => "ScaleFactorChanged",
        ModifiersChanged(_) => "ModifiersChanged",
        EscapePressed => "EscapePressed",
        WindowResized { .. } => "WindowResized",
        SystemThemeFetched(_) => "SystemThemeFetched",
        SystemThemeChanged(_) => "SystemThemeChanged",
        ThemePreferenceChanged(_) => "ThemePreferenceChanged",
        ArtistOpened(_) => "ArtistOpened",
        ArtistClosed => "ArtistClosed",
        ArtistPfpLoaded(_) => "ArtistPfpLoaded",
        CommentPfpLoaded(..) => "CommentPfpLoaded",
        CommentStampLoaded(..) => "CommentStampLoaded",
        OpenInBrowser(_) => "OpenInBrowser",
        CopyIllustUrl(_) => "CopyIllustUrl",
        RevealIllustFiles(_) => "RevealIllustFiles",
        TagSearchRequested(_) => "TagSearchRequested",
        GridSizeChanged(_) => "GridSizeChanged",
        GridSizeStep(_) => "GridSizeStep",
        OpenSettings => "OpenSettings",
        CloseSettings => "CloseSettings",
        AutoPreloadChanged(_) => "AutoPreloadChanged",
        DownloadConcurrencyChanged(_) => "DownloadConcurrencyChanged",
        GalleryPathInputChanged(_) => "GalleryPathInputChanged",
        ProxyInputChanged(_) => "ProxyInputChanged",
        ParallaxChanged(_) => "ParallaxChanged",
        ParallaxMouseChanged(_) => "ParallaxMouseChanged",
        ParallaxScrollChanged(_) => "ParallaxScrollChanged",
        ParallaxModeChanged(_) => "ParallaxModeChanged",
        GalleryFontSizeChanged(_) => "GalleryFontSizeChanged",
        GalleryTextStyleChanged(_) => "GalleryTextStyleChanged",
        TileRowsChanged(..) => "TileRowsChanged",
        BookmarkedDisplayChanged(_) => "BookmarkedDisplayChanged",
        BlurR18Changed(_) => "BlurR18Changed",
        ViewerMergeToggled(_) => "ViewerMergeToggled",
        ViewerRotate(_) => "ViewerRotate",
        ViewerTransformReset => "ViewerTransformReset",
        ViewerSplitDetected(..) => "ViewerSplitDetected",
        ViewerTransformReady(..) => "ViewerTransformReady",
        TwitterViewerToolsChanged(_) => "TwitterViewerToolsChanged",
        MemoryPresetChanged(_) => "MemoryPresetChanged",
        FpsLimitChanged(_) => "FpsLimitChanged",
        ToggleHelp => "ToggleHelp",
        ThumbnailHighResChanged(_) => "ThumbnailHighResChanged",
        ResetSetting(_) => "ResetSetting",
        ResetAllSettings => "ResetAllSettings",
        Error(_) => "Error",
        DismissError => "DismissError",
        ApplyProxy => "ApplyProxy",
        ClientProxied(_) => "ClientProxied",
        TwitterArtistLoaded(..) => "TwitterArtistLoaded",
        BilibiliArtistLoaded(..) => "BilibiliArtistLoaded",
        SourceFilterToggled(_) => "SourceFilterToggled",
        Noop => "Noop",
        FlushDisk => "FlushDisk",
    }
}

/// Reset all telemetry at the start of a profiling session.
#[cfg(feature = "profiling")]
pub fn begin_session() {
    tracing::clear();
    memory::reset();
}

#[cfg(not(feature = "profiling"))]
pub fn begin_session() {}

/// Timed wrapper around the app's `update()`. Records the per-message
/// duration and tags allocations with the message. Drop-in for the
/// `iced::application` update parameter.
#[cfg(feature = "profiling")]
pub fn timed_update(state: &mut AppState, message: Message) -> iced::Task<Message> {
    let name = message_name(&message);
    memory::set_tag(name);
    let start = Instant::now();
    let task = crate::app::update::update(state, message);
    let duration = start.elapsed();
    tracing::record_update(name.to_string(), duration);
    task
}

#[cfg(not(feature = "profiling"))]
pub fn timed_update(state: &mut AppState, message: Message) -> iced::Task<Message> {
    crate::app::update::update(state, message)
}

/// Timed wrapper around the app's `view()`.
#[cfg(feature = "profiling")]
pub fn timed_view(state: &AppState) -> iced::Element<'_, Message> {
    memory::set_tag("view");
    let start = Instant::now();
    let element = crate::app::view::view(state);
    let duration = start.elapsed();
    tracing::record_view("view".to_string(), duration);
    element
}

#[cfg(not(feature = "profiling"))]
pub fn timed_view(state: &AppState) -> iced::Element<'_, Message> {
    crate::app::view::view(state)
}

/// Time an async task bucket (e.g. a fetch/resize job). Runs `f` to
/// completion, records its duration under `kind`, returns the result.
#[cfg(feature = "profiling")]
pub async fn timed_task<T>(kind: &'static str, f: impl std::future::Future<Output = T>) -> T {
    memory::set_tag(kind);
    let start = Instant::now();
    let out = f.await;
    tracing::record_task(kind.to_string(), start.elapsed());
    out
}

#[cfg(not(feature = "profiling"))]
pub async fn timed_task<T>(_kind: &'static str, f: impl std::future::Future<Output = T>) -> T {
    f.await
}

/// Duration thresholds (ms) used to classify frame spikes in the report.
pub const SPIKE_MS: &[f64] = &[16.7, 33.3, 100.0];
