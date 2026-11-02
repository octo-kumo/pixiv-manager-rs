# AGENTS.md

## Workspace layout

Cargo workspace with two crates:

- `pixivj-rs` — Pixiv API client library (OAuth, models, rate limiting).
- `pixiv-manager` — app library plus two binaries: `pixiv-manager` (iced GUI, `src/main.rs`) and `pixiv-manager-cli` (`src/cli/`). The CLI entrypoint is a thin wrapper; commands live in `src/cli/dispatch.rs`.

Logic shared by GUI and CLI (config IO, download paths, pagination, filename rules) lives in `pixiv-manager/src/core.rs` and `src/io/` so the front-ends cannot drift. Put shared behavior there, not in the reducer or CLI modules.

## Cross-platform rules (mandatory)

This is a cross-platform desktop app: **Linux x86-64 is supported, Windows
10/11 x86-64 is supported, macOS is experimental**. Do not introduce
Linux-only assumptions into shared Rust.

- **One codebase.** Never add a per-OS front end, state machine, downloader or
  config format. OS differences are allowed only in `src/platform/`
  (`memory.rs`, `shortcuts.rs`, `fs.rs`, `mod.rs`) and `src/io/paths.rs`.
  Reducers, widgets, provider code, the downloader and `core.rs` must stay free
  of `cfg(target_os = …)`.
- **Paths:** never call `dirs::*` outside `io::paths`. Ask
  `io::paths::AppPaths::discover()` for config/data/cache files. Never hard-code
  `~/.config/...` in code, tests or UI strings — the settings screen derives its
  hints from `AppPaths` for exactly this reason.
- **Persistence:** every persistent write goes through `io::write_atomic` /
  `io::save_json_atomic` / `io::write_atomic_private` (config, which is `0600`
  on Unix). Never hand-roll temp-file-plus-rename: the Windows semantics live in
  `platform::fs::replace_file` (ReplaceFileW / MoveFileExW), and duplicated
  copies are how the last two Windows bugs happened.
- **Tests:** isolate with `PIXIV_MANAGER_CONFIG_DIR` (prefer
  `Command::env(...)` for subprocess tests over `std::env::set_var`). No literal
  `/tmp`, `$HOME` or `\` paths in portable tests — use `tempfile::tempdir()` or
  `std::env::temp_dir()`. A test may never touch real user config, cookies,
  bookmarks or the image cache unless it is `#[ignore]`d as a live test.
- **Security:** never add browser cookie-database extraction or credential-store
  scraping; cookies stay user-exported files. Never broaden the onboarding
  listener off `127.0.0.1`. Never log cookie/token contents.
- Any PR touching `src/io/`, `src/platform/`, paths or persistence must pass the
  Linux **and** Windows test jobs in CI.

## Commands

CI (`.github/workflows/ci.yml`) is the source of truth and runs a platform
matrix:

```bash
cargo fmt --check                          # Linux only
cargo clippy --workspace --locked -- -D warnings   # Linux only
cargo test --workspace --locked            # Linux, Windows, macOS
cargo build --release --workspace --locked # Linux, Windows, macOS
```

- Focused test: `cargo test -p pixiv-manager <name>`; integration target: `cargo test -p pixiv-manager --test ui_tests`.
- `rust-toolchain` pins **stable** (same toolchain locally and in CI, so a
  nightly-only construct cannot slip in).
- `--locked` everywhere: `Cargo.lock` is committed and every platform and
  released artifact resolves the same dependency graph.
- `.cargo/config.toml` defines `cargo b` / `t` / `cl` aliases.
- `./install.sh` is the **Linux** installer (builds release, installs to
  `~/.local/bin`, KDE desktop entry, committed icon from `assets/icons/`). Keep
  it Linux-specific; Windows/macOS packaging lives in `scripts/package-*.{ps1,sh}`
  and `.github/workflows/release.yml`.

## Tests

- `cargo test --workspace` is fully offline (no network, no credentials). Fixtures in `pixiv-manager/tests/fixtures/` were captured from the real API; tests drive the `update()` reducer directly and use `iced_test`'s simulator for rendering.
- Live smoke tests are `#[ignore]`d and need cookies in the repo root:
  ```bash
  cargo test -p pixiv-manager --test twitter_tests -- --ignored live --nocapture
  cargo test -p pixiv-manager --test bilibili_tests -- --ignored live --nocapture
  ```
- Config isolation matters: `PIXIV_MANAGER_CONFIG_DIR` redirects config, library, metadata, colours and the image cache. The `test-util` feature (auto-enabled by the self dev-dependency) makes test builds default to a per-process temp dir, and the CLI tests pass the variable to the subprocess with `Command::env` (no process-global env mutation, no lock). Any new test that persists anything must opt into one of these or it will clobber real user data. `test_util::headless_iced(|| simulator(...))` serialises iced/wgpu initialization — building several headless renderers concurrently crashes inside the Vulkan loader, which is unrelated to the code under test.
- Shared fixtures: `test_util::mock_illust`. Bench: `cargo bench -p pixiv-manager --bench filter_sort`.

## Architecture

- Every displayed/stored item is a `pixivj_rs::model::Illustration`, including Twitter and Bilibili posts. Provider ids are namespaced in `pixiv-manager/src/media.rs` (bit 63 = Twitter, bit 62 = Bilibili); all `u64`-keyed maps/caches (`downloaded`, thumbnails, palettes, library, queue) rely on namespaced ids. Never store native ids — use `media::{twitter_key,bilibili_key,native_id,provider_of}`.
- Each source owns its app-level behavior. `src/source.rs` hangs the shared provider decisions off `Provider` (image-CDN headers, public web URL, gallery badge, download-dir prefix); the Twitter/Bilibili reducers, pagination, viewer/comments and bookmark-like semantics live in `app/update/twitter.rs` and `app/update/bilibili.rs` (Pixiv belongs in the generic reducer + `pixivj-rs`). Add new provider-specific logic there, not as `match Provider` blocks in shared code.
- Cross-tab mutations iterate `TabsState::iter()/iter_mut()`; don't hand-list tabs (the old lists silently skipped Bilibili/Ranking in some paths).
- State machine: one `AppState` (`app/state.rs`), one `Message` enum (`app/message.rs`), reducer split by domain in `app/update/*.rs`, rendering in `app/view.rs` + `widgets/`. `main.rs` wraps update/view with `profiling::timed_update/timed_view`, no-ops unless the `profiling` feature is on.
- On-disk download layout is defined only in `core.rs`: dirs `{id}` / `tw_{id}` / `bl_{id}`, files `{page}.{ext}`; legacy `{id}_{title}` dirs are migrated in place. Reuse the `core.rs` helpers instead of hand-rolling paths.
- The config TOML (`AppPaths::config_file()`) is written `0600` on Unix through `io::write_atomic_private` (it holds the OAuth refresh token); on Windows it relies on the user-profile ACLs. Pixiv rotates the refresh token on every refresh — the `on_refresh` callback passed to `core::build_client` must persist the new token.
- `twitter_cookies.txt` / `bilibili_cookies.txt` in the repo root are gitignored user secrets; only the ignored live tests read them. Never log cookie contents.

## Provider quirks

- Twitter/X has no public API: the client calls x.com's internal GraphQL with browser cookies. The opaque `queryId`s rotate every few months, so a Twitter 404 usually means stale ids. Refreshing is a **runtime** data update: Settings → Twitter → "Update query ids" scrapes the current bundles (`src/twitter/query_ids.rs`), writes `twitter_graphql.json`, and reloads the client over the compiled defaults in `src/twitter/graphql.rs` (resolved from `PIXIV_MANAGER_TWITTER_GRAPHQL`, then the config dir, then the cwd). Only rebuild if you want the new defaults baked in.
- Bilibili requests require WBI signing (`src/bilibili/wbi.rs`). Bilibili has no liked feed, so bookmarks are tracked locally and feed the mixed Bookmarks tab.

## Profiling

- `./scripts/profile.sh` — deterministic headless profiler against a mock Pixiv (no credentials); `./scripts/profile-gui.sh` — drives the real GUI with `xdotool` using real config unless `--temp`.
- The `profiling` feature adds a tagged global allocator, CPU sampling, and an HTTP control server (`PIXIV_MANAGER_PROFILE_PORT` / `PIXIV_MANAGER_PROFILE_DIR`); scripts write `report.md` / `flamegraph.svg` / `hotspots.md`.
- CPU sampling uses `pprof`, which is **Unix-only** (`pprof` is a `cfg(unix)` dependency). On Windows the feature still builds: `profiling::cpu` becomes a stub that reports CPU sampling as unsupported, while allocation profiling, timings and reports keep working. Keep it that way — a normal Windows build must never depend on POSIX profiling.
- Memory figures in the debug overlay come from `platform::memory_stats()` (sysinfo) and may be `None`, rendered as `N/A`. Never read `/proc` outside a Linux-specific implementation.
