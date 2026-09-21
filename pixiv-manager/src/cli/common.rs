use super::format;

/// Format and print illustrations as table or JSON.
pub fn print_illusts(illusts: &[pixivj_rs::model::Illustration], json: bool) {
    if json {
        println!("{}", format::format_illust_json(illusts));
    } else {
        println!("{}", format::format_illust_table(illusts));
    }
}

/// Build the authenticated API client, exiting with an error on failure.
/// Every subcommand needs the same fail-fast client construction.
pub async fn client_or_exit() -> pixivj_rs::api::PixivApiClient {
    match super::dispatch::build_client().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to build API client: {}", e);
            std::process::exit(1);
        }
    }
}
