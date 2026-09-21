use super::common;

pub async fn run_follow(user_id: u64) {
    let client = common::client_or_exit().await;

    let request = pixivj_rs::model::FollowUserRequest::new(user_id);

    match client.follow_user(&request).await {
        Ok(()) => println!("Followed user {}", user_id),
        Err(e) => {
            eprintln!("Failed to follow user {}: {}", user_id, e);
            std::process::exit(1);
        }
    }
}

pub async fn run_unfollow(user_id: u64) {
    let client = common::client_or_exit().await;

    let request = pixivj_rs::model::FollowUserRequest::new(user_id);

    match client.unfollow_user(&request).await {
        Ok(()) => println!("Unfollowed user {}", user_id),
        Err(e) => {
            eprintln!("Failed to unfollow user {}: {}", user_id, e);
            std::process::exit(1);
        }
    }
}
