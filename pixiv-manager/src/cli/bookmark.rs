use super::common;

pub async fn run_list(json: bool) {
    let client = common::client_or_exit().await;

    let filter = pixivj_rs::model::V2Filter::default();

    let first_page = match client.get_bookmarks(&filter).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to fetch bookmarks: {}", e);
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

pub async fn run_toggle(id: u64) {
    let client = common::client_or_exit().await;

    match client.get_illust_detail(id).await {
        Ok(detail) => {
            if detail.illust.bookmarked() {
                match client.remove_bookmark(id).await {
                    Ok(()) => println!("Removed bookmark for illust {}", id),
                    Err(e) => {
                        eprintln!("Failed to remove bookmark: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                let bookmark = pixivj_rs::model::AddBookmark::new(id);
                match client.add_bookmark(&bookmark).await {
                    Ok(_) => println!("Bookmarked illust {}", id),
                    Err(e) => {
                        eprintln!("Failed to add bookmark: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to get illust detail: {}", e);
            std::process::exit(1);
        }
    }
}
