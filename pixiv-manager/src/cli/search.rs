use super::common;

pub async fn run(query: String, json: bool) {
    let client = common::client_or_exit().await;

    let filter = pixivj_rs::model::SearchedIllustsFilter::new(query);

    match client.search_illusts(&filter).await {
        Ok(result) => {
            common::print_illusts(&result.illusts, json);
        }
        Err(e) => {
            eprintln!("Search failed: {}", e);
            std::process::exit(1);
        }
    }
}
