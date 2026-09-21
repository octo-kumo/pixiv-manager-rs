pub async fn run(show: bool, set: Option<String>) {
    let mut settings = crate::io::config::load_config().unwrap_or_default();

    if let Some(ref kv) = set {
        let (key, value) = match kv.split_once('=') {
            Some(parts) => parts,
            None => {
                eprintln!("Invalid format. Use key=value (e.g. refresh_token=abc123)");
                std::process::exit(1);
            }
        };

        match key.trim() {
            "refresh_token" => {
                if value.is_empty() {
                    eprintln!("refresh_token cannot be empty");
                    std::process::exit(1);
                }
                settings.refresh_token = Some(value.to_string());
            }
            "gallery_path" => {
                settings.gallery_path = Some(std::path::PathBuf::from(value));
            }
            "grid_size" => match value.parse::<u32>() {
                Ok(n) if n > 0 => settings.grid_size = n,
                _ => {
                    eprintln!("grid_size must be a positive integer, got: {}", value);
                    std::process::exit(1);
                }
            },
            "proxy" => {
                if value.is_empty() {
                    settings.proxy = None;
                } else {
                    settings.proxy = Some(value.to_string());
                }
            }
            _ => {
                eprintln!(
                    "Unknown config key '{}'. Valid keys: refresh_token, gallery_path, grid_size, proxy",
                    key.trim()
                );
                std::process::exit(1);
            }
        }

        crate::io::config::save_config(&settings);
        // Never echo the refresh token itself — it lands in the terminal and
        // shell history. Scripts should set it via the PIXIV_REFRESH_TOKEN env
        // var, or read it from the config file directly.
        println!("Config updated: {}", key.trim());
    }

    if show || set.is_none() {
        let cfg_path = crate::io::config::config_path();
        println!(
            "Config file: {}",
            cfg_path
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        println!(
            "  refresh_token: {}",
            settings
                .refresh_token
                .as_deref()
                .map(mask_token)
                .unwrap_or_else(|| "<not set>".to_string())
        );
        println!(
            "  gallery_path:  {}",
            settings
                .gallery_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<not set>".to_string())
        );
        println!("  grid_size:     {}", settings.grid_size);
        println!(
            "  proxy:         {}",
            settings.proxy.as_deref().unwrap_or("<not set>")
        );
    }
}

/// Mask a secret, keeping only the last 4 characters, e.g. `••••••abcd`.
fn mask_token(token: &str) -> String {
    let last4: String = token
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("••••{}", last4)
}
