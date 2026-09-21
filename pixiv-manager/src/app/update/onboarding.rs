//! Onboarding flow: local capture server, PKCE, token validation, client building.

use super::handle_api_error;
use super::jobs::{bookmark_sync_task, spawn_download_workers};
use crate::app::message::Message;
use crate::app::state::AppState;
use crate::app::state::{AppPhase, OnboardingStep, Tab};
use crate::io::config::{load_config, save_config};
use crate::io::library::{DownloadQueue, SyncJobState};
use crate::onboarding_server;
use pixivj_rs::api::PixivApiClient;
use pixivj_rs::auth::{Credential, PixivOAuthClient};
use pixivj_rs::error::PixivError;
use pixivj_rs::model::GrantType;
use std::sync::Arc;

pub(super) fn start_onboarding(state: &mut AppState) -> iced::Task<Message> {
    log::info!("[update] StartOnboarding — starting server");
    state.phase = AppPhase::Onboarding;
    state.onboarding.step = OnboardingStep::Starting;

    // Generate a PKCE (RFC 7636) verifier/challenge pair. The challenge goes
    // into the Pixiv login URL; the verifier is needed later to exchange the
    // auth code for a refresh token.
    let (code_verifier, code_challenge) = generate_pkce();
    let login_url = format!(
        "https://app-api.pixiv.net/web/v1/login?code_challenge={}&code_challenge_method=S256&client=pixiv-android",
        code_challenge
    );
    state.onboarding.code_verifier = Some(code_verifier);
    state.onboarding.login_url = Some(login_url.clone());

    let (url, rx) = onboarding_server::start_server(&login_url);
    state.onboarding.url = Some(url.clone());
    // Hand the receiver to the subscription so tokens are picked up without
    // ever blocking a thread (avoids hanging app shutdown on close).
    state.onboarding.rx = Some(Arc::new(std::sync::Mutex::new(rx)));
    log::info!("[update] Server started at {}", url);

    iced::Task::done(Message::OnboardingServerReady(url))
}

/// If the error is an auth failure (invalid/expired token), fall back to
/// onboarding so the user can submit a fresh one.
///
/// IMPORTANT: the saved token is NOT cleared from the config file here. A
/// `PixivError::Auth` can be caused by a transient problem (rate limiting, a
/// network blip whose body happens to parse as an auth error), and wiping the
/// config on those would log the user out for no reason. The token is only
const PIXIV_CALLBACK_URL: &str = "https://app-api.pixiv.net/web/v1/users/auth/pixiv/callback";

/// Generate a PKCE code_verifier and its S256 code_challenge (RFC 7636).
fn generate_pkce() -> (String, String) {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use rand::RngExt;
    use sha2::{Digest, Sha256};

    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// Extract the `code` query parameter from a pasted Pixiv OAuth callback URL.
/// Handles both the HTTPS callback (`.../users/auth/pixiv/callback?code=...`)
/// and the mobile-app deep link (`pixiv://account/login?code=...`).
/// Returns `None` if the input isn't a callback URL.
fn extract_oauth_code(input: &str) -> Option<String> {
    let input = input.trim().split('#').next()?;
    let (_, query) = input.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "code" && !value.is_empty()).then(|| value.to_string())
    })
}

/// Exchange a PKCE auth code for a refresh token (authorization_code grant).
async fn exchange_code(code: String, code_verifier: String) -> Result<String, PixivError> {
    let oauth = PixivOAuthClient::new()?;
    let credential = Credential {
        grant_type: GrantType::AuthorizationCode,
        code: Some(code),
        code_verifier: Some(code_verifier),
        redirect_uri: Some(PIXIV_CALLBACK_URL.to_string()),
        include_policy: Some(true),
        ..Default::default()
    };
    let result = oauth.authenticate(&credential).await?;
    Ok(result.refresh_token)
}

/// Try the pasted string as a refresh token; if Pixiv rejects it, retry as a
/// raw OAuth authorization code (for the `pixiv://` login flow where users
/// paste just the `code` value). Returns the refresh token on success.
async fn validate_or_exchange(input: String, code_verifier: String) -> Result<String, PixivError> {
    let oauth = PixivOAuthClient::new()?;
    match oauth
        .authenticate(&Credential::refresh(input.clone()))
        .await
    {
        Ok(result) => Ok(result.refresh_token),
        Err(_) => exchange_code(input, code_verifier).await,
    }
}

// ── Client building ────────────────────────────────────────────────────

pub(super) async fn build_client_async(
    token: String,
    proxy: Option<String>,
) -> Result<PixivApiClient, PixivError> {
    log::info!(
        "[build_client] Building PixivApiClient (token len={})...",
        token.len()
    );
    // Pixiv rotates the refresh token on every refresh. Persist the newest
    // one so the next launch can log in without repeating onboarding.
    let client = crate::core::build_client(token, proxy, persist_refresh_token, false).await?;
    log::info!("[build_client] Client built successfully");
    Ok(client)
}

/// Merge the newest refresh token into the config on disk (only touches the
/// token field, preserving the rest of the saved settings).
fn persist_refresh_token(new_token: String) {
    if let Some(mut settings) = load_config() {
        if settings.refresh_token.as_deref() != Some(new_token.as_str()) {
            settings.refresh_token = Some(new_token);
            save_config(&settings);
            log::info!("[build_client] Persisted updated refresh token to config");
        }
    }
}

// ── Shared HTTP client ────────────────────────────────────────────────
// One pooled `reqwest::Client` reused for all image/thumbnail fetches.
// Building a fresh client per request throws away the TLS session and the
// connection pool, which makes downloads/opening images noticeably slower.

pub(super) fn handle(state: &mut AppState, message: Message) -> iced::Task<Message> {
    match message {
        Message::ClientBuilt(result) => {
            log::info!(
                "[update] ClientBuilt: {:?}",
                result
                    .as_ref()
                    .map(|_| "Ok(client)")
                    .map_err(|e| format!("Err({})", e))
            );
            match result {
                Ok(client) => {
                    state.client = Some(client);
                    let mut tasks: Vec<iced::Task<Message>> = vec![
                        iced::Task::done(Message::TabSelected(Tab::Recommend)),
                        iced::Task::done(Message::LoadMore),
                    ];

                    // Build the Twitter client (if cookies are configured)
                    // BEFORE creating the sync task so the boot sync also
                    // pulls liked tweets. The returned task only verifies the
                    // session in the background.
                    tasks.push(super::twitter::start_client(state));
                    tasks.push(super::bilibili::start_client(state));

                    // Resume a pending download job from a previous run.
                    state.jobs.download_queue =
                        DownloadQueue::load(&state.jobs.download_queue_path).unwrap_or_default();
                    if !state.jobs.download_queue.is_finished() {
                        state.jobs.download_job.running = true;
                        tasks.push(spawn_download_workers(state));
                    }

                    // Auto incremental bookmark library sync on every start
                    // (stops as soon as it reaches data we already have).
                    let client = state.client.clone().unwrap();
                    let library_path = state.jobs.library_path.clone();
                    state.jobs.bookmark_sync = SyncJobState {
                        running: true,
                        done: 0,
                        total: None,
                        current: "Checking for new bookmarks...".into(),
                        error: None,
                    };
                    tasks.push(bookmark_sync_task(
                        client,
                        state.twitter.client.clone(),
                        state.jobs.library.clone(),
                        library_path,
                        false,
                        state.jobs.sync_cancel.clone(),
                    ));

                    iced::Task::batch(tasks)
                }
                Err(e) => {
                    if let Some(task) = handle_api_error(state, &e) {
                        return task;
                    }
                    log::error!("[onboarding] Failed to build API client: {}", e);
                    // Non-auth build failures have no fallback flow — surface
                    // them so the user isn't stuck on a silent screen.
                    state.onboarding.error = Some(format!("Could not start: {e}"));
                    iced::Task::none()
                }
            }
        }

        // ── Onboarding: server ready ───────────────────────────────
        Message::OnboardingServerReady(url) => {
            log::info!("[update] OnboardingServerReady: {}", url);
            state.onboarding.url = Some(url);
            state.onboarding.step = OnboardingStep::WaitingForToken;
            log::info!("[update] Onboarding step → WaitingForToken");
            iced::Task::none()
        }

        // ── Onboarding: token input changed ─────────────────────────
        Message::TokenInputChanged(text) => {
            state.onboarding.token = text;
            iced::Task::none()
        }

        // ── Onboarding: open URL in browser ─────────────────────────
        Message::OpenUrl(url) => {
            log::info!("[update] OpenUrl: {}", url);
            let _ = open::that(&url);
            iced::Task::none()
        }

        // ── Onboarding: pick directory ───────────────────────────────
        Message::PickDirectory => {
            log::info!("[update] PickDirectory — opening native file dialog");
            // Use rfd in async mode to avoid blocking the UI
            let task = iced::Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Select gallery directory")
                        .pick_folder()
                        .await
                        .map(|h| h.path().to_string_lossy().to_string())
                },
                Message::DirectorySelected,
            );
            task
        }

        // ── Onboarding: token submitted ────────────────────────────
        Message::TokenSubmitted(token) => {
            log::info!("[update] TokenSubmitted (len={})", token.len());
            if token.is_empty() {
                log::warn!("[update] Empty token submitted, ignoring");
                return iced::Task::none();
            }
            state.onboarding.token = token.clone();
            state.onboarding.error = None;
            state.onboarding.step = OnboardingStep::ValidatingToken;

            // If the user pasted a callback URL (https or pixiv:// scheme),
            // extract the auth code and exchange it (PKCE).
            if let Some(code) = extract_oauth_code(&token) {
                log::info!("[update] Callback URL detected, exchanging auth code");
                let code_verifier = state.onboarding.code_verifier.clone().unwrap_or_default();
                return iced::Task::perform(
                    exchange_code(code, code_verifier),
                    Message::CodeExchanged,
                );
            }

            // Bare input: try it as a refresh token first; if Pixiv rejects
            // it, retry as a raw OAuth code (the pixiv:// flow).
            let code_verifier = state.onboarding.code_verifier.clone().unwrap_or_default();
            iced::Task::perform(
                validate_or_exchange(token, code_verifier),
                Message::TokenValidated,
            )
        }

        // ── Onboarding: auth code exchanged ─────────────────────────
        Message::CodeExchanged(result) => {
            match result {
                Ok(refresh_token) => {
                    log::info!(
                        "[update] Code exchanged, refresh token obtained (len={})",
                        refresh_token.len()
                    );
                    state.settings.refresh_token = Some(refresh_token);
                    save_config(&state.settings);
                    state.onboarding.error = None;
                    state.onboarding.step = OnboardingStep::SelectDirectory;
                }
                Err(e) => {
                    log::error!("[onboarding] Login failed: {}", e);
                    state.onboarding.step = OnboardingStep::WaitingForToken;
                    state.onboarding.token.clear();
                    state.onboarding.error = Some(format!(
                        "Login failed: {e}\nCheck the pasted URL/token and try again."
                    ));
                }
            }
            iced::Task::none()
        }

        // ── Onboarding: token validated ────────────────────────────
        Message::TokenValidated(result) => {
            match result {
                Ok(refresh_token) => {
                    // Token is valid — save it and move to directory selection
                    state.settings.refresh_token = Some(refresh_token);
                    save_config(&state.settings);
                    state.onboarding.error = None;
                    state.onboarding.step = OnboardingStep::SelectDirectory;
                }
                Err(e) => {
                    log::error!("[onboarding] Token rejected: {}", e);
                    state.onboarding.step = OnboardingStep::WaitingForToken;
                    state.onboarding.token.clear();
                    state.onboarding.error = Some(format!(
                        "Token rejected: {e}\nMake sure you pasted the FULL URL from the address bar."
                    ));
                }
            }
            iced::Task::none()
        }

        // ── Onboarding: directory selected ─────────────────────────
        Message::DirectorySelected(path) => {
            if let Some(p) = path {
                state.settings.gallery_path = Some(std::path::PathBuf::from(&p));
                save_config(&state.settings);
            }
            iced::Task::none()
        }

        // ── Onboarding: complete ───────────────────────────────────
        Message::OnboardingComplete => {
            state.onboarding.step = OnboardingStep::Complete;
            state.phase = AppPhase::Main;
            state.onboarding.rx = None;
            save_config(&state.settings);

            // Build the API client and fire initial data load
            if let Some(ref token) = state.settings.refresh_token {
                let token = token.clone();
                return iced::Task::perform(
                    build_client_async(token, state.settings.proxy.clone()),
                    Message::ClientBuilt,
                );
            }
            iced::Task::none()
        }

        // ── Tab management ─────────────────────────────────────────
        Message::StartOnboarding => start_onboarding(state),

        #[allow(unreachable_patterns)]
        // Any message routed here by the dispatcher must be one of the above.
        _ => unreachable!("message routed to the wrong update handler"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_pkce_produces_verifier_and_matching_challenge() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        use sha2::{Digest, Sha256};

        let (verifier, challenge) = generate_pkce();
        // Verifier: 32 random bytes, base64url, no padding.
        assert_eq!(verifier.len(), 43);
        assert!(verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        // Challenge = base64url(sha256(verifier)).
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
        // Two calls produce different verifiers (randomness).
        assert_ne!(verifier, generate_pkce().0);
    }

    #[test]
    fn extract_oauth_code_parses_callback_url() {
        let url =
            "https://app-api.pixiv.net/web/v1/users/auth/pixiv/callback?state=abc&code=secret123";
        assert_eq!(extract_oauth_code(url).as_deref(), Some("secret123"));

        // Fragments (e.g. trailing "#_") must not break parsing.
        let with_fragment = format!("{}#_", url);
        assert_eq!(
            extract_oauth_code(&with_fragment).as_deref(),
            Some("secret123")
        );

        // Parameter order can vary.
        let reversed =
            "https://app-api.pixiv.net/web/v1/users/auth/pixiv/callback?code=xyz&state=abc";
        assert_eq!(extract_oauth_code(reversed).as_deref(), Some("xyz"));

        // The current mobile-app flow uses a pixiv:// deep-link callback.
        let deep_link = "pixiv://account/login?code=deep_code_1&via=login";
        assert_eq!(
            extract_oauth_code(deep_link).as_deref(),
            Some("deep_code_1")
        );
        let deep_link_reversed = "pixiv://account/login?via=login&code=deep_code_2";
        assert_eq!(
            extract_oauth_code(deep_link_reversed).as_deref(),
            Some("deep_code_2")
        );
    }

    #[test]
    fn extract_oauth_code_rejects_non_urls_and_other_urls() {
        // A bare refresh token has no "code=" parameter.
        assert_eq!(extract_oauth_code("my_refresh_token_123"), None);
        // A URL without a query string.
        assert_eq!(
            extract_oauth_code("https://www.pixiv.net/en/artworks/12345"),
            None
        );
        // An empty code value is not accepted.
        assert_eq!(
            extract_oauth_code("https://app-api.pixiv.net/web/v1/users/auth/pixiv/callback?code="),
            None
        );
    }
}
