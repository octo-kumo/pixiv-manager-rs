use super::common;

pub async fn run(id: Option<u64>) {
    let client = common::client_or_exit().await;

    if let Some(illust_id) = id {
        download_one(&client, illust_id).await;
    } else {
        download_all(&client).await;
    }
}

async fn download_one(client: &pixivj_rs::api::PixivApiClient, illust_id: u64) {
    let detail = match client.get_illust_detail(illust_id).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to fetch illust {}: {}", illust_id, e);
            std::process::exit(1);
        }
    };

    let illust = &detail.illust;
    let title = illust.title();
    let artist = illust
        .user
        .as_ref()
        .map(|u| u.name.as_str())
        .unwrap_or("unknown");

    println!("Downloading: {} (ID: {}) by {}", title, illust.id, artist);

    let urls: Vec<String> = crate::core::collect_download_urls(illust);

    if urls.is_empty() {
        eprintln!("No downloadable image URLs found for illust {}", illust_id);
        return;
    }

    let total = urls.len();
    let gallery_path = get_gallery_path();
    let dir = crate::core::download_dir(&gallery_path, illust.id, title);

    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("Failed to create directory {:?}: {}", dir, e);
        std::process::exit(1);
    }

    for (i, url) in urls.iter().enumerate() {
        print!("  Page {}/{} ... ", i + 1, total);
        match client.download(url).await {
            Ok(resp) => {
                let bytes = match resp.bytes().await {
                    Ok(b) => b,
                    Err(e) => {
                        println!("failed to read body: {}", e);
                        continue;
                    }
                };

                let ext = crate::core::file_extension(url);
                let filename = crate::core::image_filename(illust.id, i, total, ext);

                let path = dir.join(&filename);
                match std::fs::write(&path, &bytes) {
                    Ok(()) => println!("OK ({})", path.display()),
                    Err(e) => println!("write error: {}", e),
                }
            }
            Err(e) => {
                println!("download error: {}", e);
            }
        }
    }
}

async fn download_all(client: &pixivj_rs::api::PixivApiClient) {
    println!("Fetching bookmarks...");

    let filter = pixivj_rs::model::V2Filter::default();
    let first_page = match client.get_bookmarks(&filter).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to fetch bookmarks: {}", e);
            std::process::exit(1);
        }
    };

    let all = match crate::core::paginate(client, first_page).await {
        Ok(illusts) => illusts,
        Err(e) => {
            eprintln!("Failed to paginate: {}", e);
            std::process::exit(1);
        }
    };

    // Skip illusts already fully on disk (same rule as the GUI job), so
    // repeated runs only fetch what's missing.
    let gallery_path = get_gallery_path();
    let (todo, skipped): (Vec<_>, Vec<_>) = all
        .iter()
        .partition(|i| !crate::core::is_illust_downloaded(&gallery_path, i));
    if !skipped.is_empty() {
        println!(
            "Skipping {} already downloaded; downloading {}.",
            skipped.len(),
            todo.len()
        );
    } else {
        println!("Found {} bookmarked illusts to download.", todo.len());
    }

    for illust in &todo {
        download_one(client, illust.id).await;
    }

    println!("Done.");
}

fn get_gallery_path() -> std::path::PathBuf {
    crate::io::config::load_config()
        .and_then(|c| c.gallery_path)
        .unwrap_or_else(crate::core::default_gallery_dir)
}
