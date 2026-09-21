use super::common;

pub async fn run(json: bool) {
    let client = common::client_or_exit().await;

    let filter = pixivj_rs::model::RecommendedIllustsFilter::default();

    match client.get_recommended_illusts(&filter).await {
        Ok(result) => {
            super::common::print_illusts(&result.illusts, json);
        }
        Err(e) => {
            eprintln!("Failed to fetch recommended illusts: {}", e);
            std::process::exit(1);
        }
    }
}
