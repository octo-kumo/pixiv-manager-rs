use pixiv_manager::app::state::AppState;
use pixiv_manager::app::subscription::subscription;
use pixiv_manager::app::update::boot;
use pixiv_manager::profiling::{timed_update, timed_view};

fn main() -> iced::Result {
    // Initialize logging — RUST_LOG=debug for verbose, info for normal
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .try_init();

    log::info!("=== Pixiv Manager starting ===");
    log::info!("Version: {}", env!("CARGO_PKG_VERSION"));
    log::info!(
        "Data dir: {:?}",
        dirs::config_dir().map(|d| d.join("pixiv-manager"))
    );

    #[cfg(feature = "profiling")]
    {
        // Profiling builds expose an HTTP control server so an AI agent can
        // start/stop CPU captures and pull flamegraphs/reports with curl.
        if let Ok(port) =
            std::env::var("PIXIV_MANAGER_PROFILE_PORT").map(|p| p.parse().unwrap_or(0))
        {
            let out = std::path::PathBuf::from(
                std::env::var("PIXIV_MANAGER_PROFILE_DIR")
                    .unwrap_or_else(|_| "profiles/live".into()),
            );
            pixiv_manager::profiling::http::init(out);
            match pixiv_manager::profiling::http::spawn(port) {
                Ok(bound) => log::info!(
                    "profiling control server on http://127.0.0.1:{bound} \
                     (/profile/start /profile/stop /profile/flamegraph /metrics /report)"
                ),
                Err(e) => log::warn!("profiling control server failed to bind: {e}"),
            }
        }
    }

    iced::application(boot, timed_update, timed_view)
        .theme(|state: &AppState| pixiv_manager::theme::theme(state))
        .title(window_title)
        .subscription(subscription)
        // Handle close requests ourselves: in the detail/author view the OS
        // close button exits back to the gallery; in the gallery it quits.
        .exit_on_close_request(false)
        .run()
}

/// The window title doubles as a taskbar busy indicator while jobs run.
fn window_title(state: &AppState) -> String {
    if state.phase != pixiv_manager::app::state::AppPhase::Main {
        return "Pixiv Manager — Setup".to_string();
    }
    let downloading = state.jobs.download_job.running;
    let syncing = state.jobs.bookmark_sync.running;
    if downloading && syncing {
        "⏳ Pixiv Manager — downloading + syncing".to_string()
    } else if downloading {
        format!(
            "⏳ Pixiv Manager — downloading ({}/{})",
            state.jobs.download_queue.done(),
            state.jobs.download_queue.total
        )
    } else if syncing {
        "⏳ Pixiv Manager — syncing bookmarks".to_string()
    } else {
        "Pixiv Manager".to_string()
    }
}
