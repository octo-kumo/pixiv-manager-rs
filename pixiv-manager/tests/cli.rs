//! CLI integration tests.
//!
//! Every test runs the real `pixiv-manager-cli` binary in an isolated config
//! directory. Isolation is passed to the *subprocess* through
//! `PIXIV_MANAGER_CONFIG_DIR`, so this test process never mutates its own
//! environment. That replaces the old scheme, which set `XDG_CONFIG_HOME`
//! (Linux-only) globally and therefore needed a process-wide mutex to stop
//! parallel tests from racing each other's temp directories.

use assert_cmd::Command;
use predicates::prelude::*;

/// One isolated CLI world: a private config directory plus a helper that
/// spawns the CLI pointed at it.
struct Isolated {
    dir: tempfile::TempDir,
}

impl Isolated {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temp config dir"),
        }
    }

    /// A CLI invocation whose config/library/cache all live in the temp dir.
    fn cli(&self) -> Command {
        let mut command = Command::cargo_bin("pixiv-manager-cli").expect("cli binary");
        command.env("PIXIV_MANAGER_CONFIG_DIR", self.dir.path());
        command
    }

    /// `<config dir>/config.toml` — where `PIXIV_MANAGER_CONFIG_DIR` puts it.
    fn config_file(&self) -> std::path::PathBuf {
        self.dir.path().join("config.toml")
    }
}

#[test]
fn test_help() {
    let cli = Isolated::new();
    cli.cli()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("download"))
        .stdout(predicate::str::contains("search"))
        .stdout(predicate::str::contains("bookmarks"))
        .stdout(predicate::str::contains("follow"))
        .stdout(predicate::str::contains("ranking"))
        .stdout(predicate::str::contains("recommended"))
        .stdout(predicate::str::contains("config"));
}

#[test]
fn test_version() {
    let cli = Isolated::new();
    cli.cli()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_no_args_shows_error() {
    let cli = Isolated::new();
    cli.cli()
        .assert()
        .failure()
        .stderr(predicate::str::contains("No command specified"));
}

#[test]
fn test_config_show_no_config() {
    let cli = Isolated::new();
    cli.cli().arg("config").arg("--show").assert().success();
}

#[test]
fn test_config_set_invalid_format() {
    let cli = Isolated::new();
    cli.cli()
        .arg("config")
        .arg("--set")
        .arg("noequals")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid format"));
}

#[test]
fn test_config_set_unknown_key() {
    let cli = Isolated::new();
    cli.cli()
        .arg("config")
        .arg("--set")
        .arg("unknown_key=value")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown config key"));
}

#[test]
fn test_config_set_grid_size_invalid() {
    let cli = Isolated::new();
    cli.cli()
        .arg("config")
        .arg("--set")
        .arg("grid_size=abc")
        .assert()
        .failure()
        .stderr(predicate::str::contains("positive integer"));
}

#[test]
fn test_config_set_grid_size_zero() {
    let cli = Isolated::new();
    cli.cli()
        .arg("config")
        .arg("--set")
        .arg("grid_size=0")
        .assert()
        .failure()
        .stderr(predicate::str::contains("positive integer"));
}

#[test]
fn test_config_set_valid() {
    let cli = Isolated::new();
    cli.cli()
        .arg("config")
        .arg("--set")
        .arg("grid_size=300")
        .assert()
        .success()
        .stdout(predicate::str::contains("Config updated"));
}

#[test]
fn test_config_roundtrip() {
    let cli = Isolated::new();
    cli.cli()
        .arg("config")
        .arg("--set")
        .arg("grid_size=250")
        .assert()
        .success();

    cli.cli()
        .arg("config")
        .arg("--show")
        .assert()
        .success()
        .stdout(predicate::str::contains("grid_size"));
}

/// The CLI must write exactly one file into the isolated directory and never
/// fall back to the real user configuration.
#[test]
fn test_config_is_written_inside_the_isolated_dir() {
    let cli = Isolated::new();
    cli.cli()
        .arg("config")
        .arg("--set")
        .arg("grid_size=260")
        .assert()
        .success();
    let path = cli.config_file();
    assert!(path.is_file(), "expected {} to exist", path.display());
    let content = std::fs::read_to_string(&path).expect("config readable");
    assert!(content.contains("260"), "config content: {content}");
}

#[test]
fn test_search_no_token() {
    let cli = Isolated::new();
    cli.cli()
        .arg("search")
        .arg("cat")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Not configured"));
}

#[test]
fn test_config_set_token_never_echoes_value() {
    let cli = Isolated::new();
    cli.cli()
        .arg("config")
        .arg("--set")
        .arg("refresh_token=super-secret-token-value")
        .assert()
        .success()
        .stdout(predicate::str::contains("Config updated: refresh_token"))
        .stdout(predicate::str::contains("super-secret-token-value").not());
}

#[test]
fn test_config_show_masks_token() {
    let cli = Isolated::new();
    cli.cli()
        .arg("config")
        .arg("--set")
        .arg("refresh_token=super-secret-token-value")
        .assert()
        .success();

    cli.cli()
        .arg("config")
        .arg("--show")
        .assert()
        .success()
        .stdout(predicate::str::contains("super-secret-token-value").not())
        .stdout(predicate::str::contains("••••"));
}

#[cfg(unix)]
#[test]
fn test_config_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let cli = Isolated::new();
    cli.cli()
        .arg("config")
        .arg("--set")
        .arg("grid_size=300")
        .assert()
        .success();

    let mode = std::fs::metadata(cli.config_file())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "config must not be group/other readable");
}
