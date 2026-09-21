# AGENTS.md

## Workspace layout

Cargo workspace with two crates:

- `pixivj-rs` — Pixiv API client library (OAuth, models, rate limiting).
- `pixiv-manager` — app library plus two binaries: `pixiv-manager` (iced GUI, `src/main.rs`) and `pixiv-manager-cli` (`src/cli/`). The CLI entrypoint is a thin wrapper; commands live in `src/cli/dispatch.rs`.

Logic shared by GUI and CLI (config IO, download paths, pagination, filename rules) lives in `pixiv-manager/src/core.rs` and `src/io/` so the front-ends cannot drift. Put shared behavior there, not in the reducer or CLI modules.

## Commands

CI (`.github/workflows/ci.yml`) is the source of truth:

```bash
cargo fmt --check
cargo clippy --workspace -- -D warnings   # README's `--all-targets` variant is stale
cargo test --workspace
cargo build --release --workspace
```

- Focused test: `cargo test -p pixiv-manager <name>`; integration target: `cargo test -p pixiv-manager --test ui_tests`.
- `rust-toolchain` pins **nightly** locally, but CI builds on stable — do not introduce nightly-only features.
- `.cargo/config.toml` defines `cargo b` / `t` / `cl` aliases.
- `./install.sh` builds release and installs to `~/.local/bin` plus a KDE desktop entry.

## Tests

- `cargo test --workspace` is fully offline (no network, no credentials). Fixtures in `pixiv-manager/tests/fixtures/` were captured from the real API; tests drive the `update()` reducer directly and use `iced_test`'s simulator for rendering.
- Live smoke tests are `#[ignore]`d and need cookies in the repo root:
  ```bash
  cargo test -p pixiv-manager --test twitter_tests -- --ignored live --nocapture
  cargo test -p pixiv-manager --test bilibili_tests -- --ignored live --nocapture
  ```
- Config isolation matters: `PIXIV_MANAGER_CONFIG_DIR` redirects config/library/color files. The `test-util` feature (auto-enabled by the self dev-dependency) makes test builds default to per-process temp dirs, and CLI tests use `XDG_CONFIG_HOME` serialized by `CONFIG_ENV_LOCK` (env vars are process-global). Any new test that persists config must opt into one of these or it will clobber the real `~/.config/pixiv-manager/config.toml`.
- Shared fixtures: `test_util::mock_illust`. Bench: `cargo bench -p pixiv-manager --bench filter_sort`.

## Architecture

- Every displayed/stored item is a `pixivj_rs::model::Illustration`, including Twitter and Bilibili posts. Provider ids are namespaced in `pixiv-manager/src/media.rs` (bit 63 = Twitter, bit 62 = Bilibili); all `u64`-keyed maps/caches (`downloaded`, thumbnails, palettes, library, queue) rely on namespaced ids. Never store native ids — use `media::{twitter_key,bilibili_key,native_id,provider_of}`.
- Each source owns its app-level behavior. `src/source.rs` hangs the shared provider decisions off `Provider` (image-CDN headers, public web URL, gallery badge, download-dir prefix); the Twitter/Bilibili reducers, pagination, viewer/comments and bookmark-like semantics live in `app/update/twitter.rs` and `app/update/bilibili.rs` (Pixiv belongs in the generic reducer + `pixivj-rs`). Add new provider-specific logic there, not as `match Provider` blocks in shared code.
- Cross-tab mutations iterate `TabsState::iter()/iter_mut()`; don't hand-list tabs (the old lists silently skipped Bilibili/Ranking in some paths).
- State machine: one `AppState` (`app/state.rs`), one `Message` enum (`app/message.rs`), reducer split by domain in `app/update/*.rs`, rendering in `app/view.rs` + `widgets/`. `main.rs` wraps update/view with `profiling::timed_update/timed_view`, no-ops unless the `profiling` feature is on.
- On-disk download layout is defined only in `core.rs`: dirs `{id}` / `tw_{id}` / `bl_{id}`, files `{page}.{ext}`; legacy `{id}_{title}` dirs are migrated in place. Reuse the `core.rs` helpers instead of hand-rolling paths.
- Config TOML at `~/.config/pixiv-manager/config.toml` is written `0600` (holds the OAuth refresh token). Pixiv rotates the refresh token on every refresh — the `on_refresh` callback passed to `core::build_client` must persist the new token.
- `twitter_cookies.txt` / `bilibili_cookies.txt` in the repo root are gitignored user secrets; only the ignored live tests read them. Never log cookie contents.

## Provider quirks

- Twitter/X has no public API: the client calls x.com's internal GraphQL with browser cookies. The opaque `queryId`s rotate every few months, so a Twitter 404 usually means stale ids. Refreshing is a **runtime** data update: Settings → Twitter → "Update query ids" scrapes the current bundles (`src/twitter/query_ids.rs`), writes `twitter_graphql.json`, and reloads the client over the compiled defaults in `src/twitter/graphql.rs` (resolved from `PIXIV_MANAGER_TWITTER_GRAPHQL`, then the config dir, then the cwd). Only rebuild if you want the new defaults baked in.
- Bilibili requests require WBI signing (`src/bilibili/wbi.rs`). Bilibili has no liked feed, so bookmarks are tracked locally and feed the mixed Bookmarks tab.

## Profiling

- `./scripts/profile.sh` — deterministic headless profiler against a mock Pixiv (no credentials); `./scripts/profile-gui.sh` — drives the real GUI with `xdotool` using real config unless `--temp`.
- The `profiling` feature adds a tagged global allocator, pprof CPU sampling, and an HTTP control server (`PIXIV_MANAGER_PROFILE_PORT` / `PIXIV_MANAGER_PROFILE_DIR`); scripts write `report.md` / `flamegraph.svg` / `hotspots.md`.
