use super::common;

pub async fn run(user_id: u64, json: bool) {
    let client = common::client_or_exit().await;

    let query = pixivj_rs::model::UserIllustQuery {
        user_id,
        illust_type: None,
        filter: None,
        offset: None,
    };

    let first_page = match client.get_user_illusts(&query).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to fetch artist illusts: {}", e);
            std::process::exit(1);
        }
    };

    let all = match crate::core::paginate(&client, first_page).await {
        Ok(illusts) => illusts,
        Err(e) => {
            eprintln!("Failed to paginate: {}", e);
            std::process::exit(1);
        }
    };

    common::print_illusts(&all, json);
}
