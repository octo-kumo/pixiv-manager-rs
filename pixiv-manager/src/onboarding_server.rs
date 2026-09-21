//! Lightweight HTTP server for Pixiv token capture during onboarding.
//!
//! Uses only `std::net::TcpListener` — no external HTTP dependencies.
//! The server binds to 127.0.0.1 on a random port, serves a single HTML
//! page describing the Pixiv OAuth PKCE login flow, and accepts a POST
//! with the callback URL (or a refresh token) captured from the browser.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Placeholder in [`HTML_PAGE`] replaced with the Pixiv login URL.
const LOGIN_URL_PLACEHOLDER: &str = "__LOGIN_URL__";

/// Placeholder in [`HTML_PAGE`] replaced with a per-session nonce path segment,
/// so only a page served by *this* server run can submit a token.
const NONCE_PLACEHOLDER: &str = "__NONCE__";

const HTML_PAGE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Pixiv Manager — Login</title>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background: #1e1e2e; color: #cdd6f4;
    display: flex; justify-content: center; align-items: center;
    min-height: 100vh; padding: 20px;
  }
  .card {
    background: #313244; border-radius: 12px; padding: 32px;
    max-width: 560px; width: 100%; box-shadow: 0 4px 24px rgba(0,0,0,0.3);
  }
  h1 { font-size: 20px; margin-bottom: 8px; color: #89b4fa; }
  p { margin: 4px 0; font-size: 14px; }
  .step { margin: 16px 0; padding: 14px; background: #45475a; border-radius: 8px; }
  .step-num { display: inline-block; width: 24px; height: 24px; background: #89b4fa;
    color: #1e1e2e; border-radius: 50%; text-align: center; line-height: 24px;
    font-weight: bold; font-size: 13px; margin-right: 8px; }
  a { color: #89b4fa; }
  code { background: #1e1e2e; padding: 2px 6px; border-radius: 4px; font-size: 12px;
    word-break: break-all; }
  .login-btn {
    display: inline-block; background: #89b4fa; color: #1e1e2e; text-decoration: none;
    padding: 12px 24px; border-radius: 6px; font-size: 15px; font-weight: 600;
    margin: 8px 0;
  }
  .login-btn:hover { background: #74c7ec; }
  input[type="text"] {
    width: 100%; padding: 10px; border: 1px solid #585b70; border-radius: 6px;
    background: #1e1e2e; color: #cdd6f4; font-size: 13px; margin: 8px 0;
  }
  button {
    background: #89b4fa; color: #1e1e2e; border: none; padding: 10px 24px;
    border-radius: 6px; font-size: 14px; font-weight: 600; cursor: pointer;
  }
  button:hover { background: #74c7ec; }
  .success { display: none; color: #a6e3a1; margin-top: 16px; }
  .error { color: #f38ba8; margin-top: 8px; font-size: 13px; display: none; }
</style>
</head>
<body>
<div class="card">
  <h1>🔑 Pixiv Manager — Login</h1>
  <p>Authorize Pixiv Manager by logging in with your Pixiv account.</p>

  <div class="step">
    <span class="step-num">1</span>
    Click the button below to log in to Pixiv (opens in a new tab):
    <br>
    <a href="__LOGIN_URL__" target="_blank" rel="noopener" class="login-btn">Log in with Pixiv</a>
  </div>

  <div class="step">
    <span class="step-num">2</span>
    After logging in, the browser will try to open a
    <code>pixiv://account/login?code=...</code> link and show an
    <strong>"can't open this link"</strong> error page — that's normal.
    Copy the <strong>full URL</strong> from your browser's address bar.
    It looks like:
    <br><br>
    <code>pixiv://account/login?code=...&amp;via=login</code>
    <br><br>
    <em>Tip: if the address bar doesn't show it, press <code>F12</code> →
    Network tab → click the last <code>callback</code> / <code>pixiv://</code>
    entry and copy its URL.</em>
  </div>

  <div class="step">
    <span class="step-num">3</span>
    Paste the URL (or just the <code>code</code> value) below and click Continue:
    <input type="text" id="token" placeholder="Paste the URL from your address bar...">
    <button onclick="submitToken()">Continue</button>
    <div class="error" id="error">Please paste the callback URL.</div>
  </div>

  <div class="success" id="success">
    ✅ Token received! You can close this tab and return to the app.
  </div>
</div>
<script>
async function submitToken() {
  const token = document.getElementById('token').value.trim();
  if (!token) {
    document.getElementById('error').style.display = 'block';
    return;
  }
  document.getElementById('error').style.display = 'none';
  try {
    await fetch('/token/__NONCE__', {
      method: 'POST',
      headers: { 'Content-Type': 'text/plain' },
      body: token
    });
    document.getElementById('success').style.display = 'block';
    document.querySelector('.step:last-of-type').style.opacity = '0.5';
  } catch(e) {
    document.getElementById('error').textContent = 'Failed to submit. Is the app still running?';
    document.getElementById('error').style.display = 'block';
  }
}
</script>
</body>
</html>"#;

/// Start the token capture server, returns the URL and a channel receiver for the token.
///
/// The receiver is `mpsc::Receiver<String>` — IMPORTANT: do NOT call `.recv()` directly
/// inside an `iced::Task::perform` future, as it blocks the async runtime.
/// Use `tokio::task::spawn_blocking(move || rx.recv().ok())` instead.
pub fn start_server(login_url: &str) -> (String, mpsc::Receiver<String>) {
    // Per-session, unpredictable path segment: only a page served by THIS
    // server run (or someone who knows the secret URL) can submit a token, so
    // an unrelated local webpage can't inject/overwrite the pending one.
    use rand::RngExt;
    let mut nonce_bytes = [0u8; 16];
    rand::rng().fill(&mut nonce_bytes);
    let nonce: String = nonce_bytes.iter().map(|b| format!("{:02x}", b)).collect();

    let html = HTML_PAGE
        .replace(LOGIN_URL_PLACEHOLDER, login_url)
        .replace(NONCE_PLACEHOLDER, &nonce);

    log::info!("[onboarding] Starting TCP server on 127.0.0.1:0...");
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind onboarding server");
    let addr = listener.local_addr().unwrap();
    let url = format!("http://127.0.0.1:{}", addr.port());
    log::info!("[onboarding] Server listening on {}", url);

    let expected_host = format!("127.0.0.1:{}", addr.port());

    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        log::info!("[onboarding] Server thread started, waiting for connections...");
        // Non-blocking accept loop with a small sleep so the thread doesn't
        // spin. It is a detached thread — it dies with the process, so it
        // never prevents the app from exiting.
        let _ = listener.set_nonblocking(true);
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    log::info!(
                        "[onboarding] Connection from {}",
                        stream
                            .peer_addr()
                            .map(|a| a.to_string())
                            .unwrap_or_else(|_| "unknown".into())
                    );
                    if let Some(token) =
                        handle_connection(stream, &tx, &html, &nonce, &expected_host)
                    {
                        log::info!(
                            "[onboarding] Token received (len={}), sending to channel and stopping server",
                            token.len()
                        );
                        let _ = tx.send(token);
                        log::info!("[onboarding] Token sent to channel, server thread exiting");
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    log::warn!("[onboarding] Accept error, stopping: {}", e);
                    break;
                }
            }
        }
        log::info!("[onboarding] Server thread ended");
    });

    (url, rx)
}

fn handle_connection(
    mut stream: TcpStream,
    _token_tx: &mpsc::Sender<String>,
    html: &str,
    nonce: &str,
    expected_host: &str,
) -> Option<String> {
    log::debug!("[onboarding] handle_connection: reading request...");
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    log::debug!("[onboarding] Request line: {}", request_line.trim());

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        log::warn!("[onboarding] Malformed request: {:?}", parts);
        return None;
    }
    let method = parts[0];
    let path = parts[1];

    // Read headers to find content-length (and the Host header, used to
    // reject requests that don't target the bound address).
    let mut content_length = 0;
    let mut host = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let line = line.trim().to_lowercase();
        if line.is_empty() {
            break;
        }
        if let Some(stripped) = line.strip_prefix("content-length:") {
            content_length = stripped.trim().parse().unwrap_or(0);
            log::debug!("[onboarding] Content-Length: {}", content_length);
        }
        if let Some(stripped) = line.strip_prefix("host:") {
            host = stripped.trim().to_string();
        }
    }

    let token_path = format!("/token/{}", nonce);
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => {
            log::info!("[onboarding] Serving HTML page ({} bytes)", html.len());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = stream.write_all(response.as_bytes());
            log::debug!("[onboarding] HTML page sent");
            None
        }
        ("POST", p) if p == token_path => {
            log::info!(
                "[onboarding] POST {} received, content_length={}",
                p,
                content_length
            );
            // Defense-in-depth against requests that don't target the bound
            // address (the per-session nonce path is the primary gate).
            if host != expected_host {
                log::warn!(
                    "[onboarding] Rejected POST with Host {:?} (expected {})",
                    host,
                    expected_host
                );
                let response =
                    "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(response.as_bytes());
                return None;
            }
            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).ok()?;
            let token = String::from_utf8_lossy(&body).trim().to_string();
            // Log length only — never the token itself (or even a prefix).
            log::info!("[onboarding] Token parsed (len={})", token.len());

            let response = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
            let _ = stream.write_all(response.as_bytes());

            if !token.is_empty() {
                Some(token)
            } else {
                log::warn!("[onboarding] POST {} had empty body", p);
                None
            }
        }
        _ => {
            log::warn!("[onboarding] Unknown route: {} {}", method, path);
            let response =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(response.as_bytes());
            None
        }
    }
}
