#[tokio::main]
async fn main() {
    pixiv_manager::cli::dispatch::run().await;
}
