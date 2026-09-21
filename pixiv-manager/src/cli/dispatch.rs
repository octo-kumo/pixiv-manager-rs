use super::{artist, bookmark, config, download, follow, ranking, recommended, search};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pixiv-manager", version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Download bookmarked illusts (or a specific illust by id)
    Download {
        /// Optional illust ID to download a specific illust
        id: Option<u64>,
    },
    /// Search illusts by query
    Search {
        /// Search query
        query: String,
        /// Output as JSON instead of table
        #[arg(long)]
        json: bool,
    },
    /// List your bookmarked illusts
    Bookmarks {
        /// Output as JSON instead of table
        #[arg(long)]
        json: bool,
    },
    /// Add a bookmark for an illust
    Bookmark { id: u64 },
    /// Follow a user
    Follow { user_id: u64 },
    /// Unfollow a user
    Unfollow { user_id: u64 },
    /// List illusts by a specific artist
    Artist {
        user_id: u64,
        #[arg(long)]
        json: bool,
    },
    /// List recommended illusts for your account
    Recommended {
        #[arg(long)]
        json: bool,
    },
    /// List ranked illusts
    Ranking {
        mode: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Show or update configuration
    Config {
        #[arg(long)]
        show: bool,
        #[arg(long)]
        set: Option<String>,
    },
}

/// Build a PixivApiClient from the refresh token in config.
pub async fn build_client() -> Result<pixivj_rs::api::PixivApiClient, pixivj_rs::error::PixivError>
{
    let config = crate::io::config::load_config();
    let refresh_token = config
        .as_ref()
        .and_then(|c| c.refresh_token.clone())
        .ok_or_else(|| {
            pixivj_rs::error::PixivError::Auth(pixivj_rs::error::AuthError {
                has_error: true,
                errors: pixivj_rs::error::AuthErrorDetails {
                    system: pixivj_rs::error::AuthSystem {
                        code: -1,
                        message:
                            "No refresh_token in config. Run 'pixiv-manager config --set refresh_token=YOUR_TOKEN'"
                                .into(),
                    },
                    message: Some("Not configured".into()),
                },
            })
        })?;
    let proxy = config.as_ref().and_then(|c| c.proxy.clone());

    // Pixiv rotates the refresh token on every refresh. Persist the newest
    // one so the saved login doesn't silently rot (this is the persist hook
    // the GUI already had; the CLI was dropping rotated tokens before).
    crate::core::build_client(
        refresh_token,
        proxy,
        |new_token| {
            if let Some(mut settings) = crate::io::config::load_config() {
                if settings.refresh_token.as_deref() != Some(new_token.as_str()) {
                    settings.refresh_token = Some(new_token);
                    crate::io::config::save_config(&settings);
                }
            }
        },
        true,
    )
    .await
}

/// Run the CLI: parse args, build client, dispatch.
pub async fn run() {
    let cli = Cli::parse();

    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            eprintln!("No command specified. Use --help for available commands.");
            std::process::exit(1);
        }
    };

    match command {
        Command::Download { id } => download::run(id).await,
        Command::Search { query, json } => search::run(query, json).await,
        Command::Bookmarks { json } => bookmark::run_list(json).await,
        Command::Bookmark { id } => bookmark::run_toggle(id).await,
        Command::Follow { user_id } => follow::run_follow(user_id).await,
        Command::Unfollow { user_id } => follow::run_unfollow(user_id).await,
        Command::Artist { user_id, json } => artist::run(user_id, json).await,
        Command::Recommended { json } => recommended::run(json).await,
        Command::Ranking { mode, json } => ranking::run(mode, json).await,
        Command::Config { show, set } => config::run(show, set).await,
    }
}
