use assert_cmd::Command;
use predicates::prelude::*;

/// Tests that touch `XDG_CONFIG_HOME` must run one at a time: the env var is
/// process-global, so parallel tests would race each other's temp dirs.
static CONFIG_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn cli_bin() -> Command {
    Command::cargo_bin("pixiv-manager-cli").unwrap()
}

#[test]
fn test_help() {
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    cli_bin()
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
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    cli_bin()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_no_args_shows_error() {
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    cli_bin()
        .assert()
        .failure()
        .stderr(predicate::str::contains("No command specified"));
}

#[test]
fn test_config_show_no_config() {
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    cli_bin().arg("config").arg("--show").assert().success();
}

#[test]
fn test_config_set_invalid_format() {
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    cli_bin()
        .arg("config")
        .arg("--set")
        .arg("noequals")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid format"));
}

#[test]
fn test_config_set_unknown_key() {
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    cli_bin()
        .arg("config")
        .arg("--set")
        .arg("unknown_key=value")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown config key"));
}

#[test]
fn test_config_set_grid_size_invalid() {
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    cli_bin()
        .arg("config")
        .arg("--set")
        .arg("grid_size=abc")
        .assert()
        .failure()
        .stderr(predicate::str::contains("positive integer"));
}

#[test]
fn test_config_set_grid_size_zero() {
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    cli_bin()
        .arg("config")
        .arg("--set")
        .arg("grid_size=0")
        .assert()
        .failure()
        .stderr(predicate::str::contains("positive integer"));
}

#[test]
fn test_config_set_valid() {
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    cli_bin()
        .arg("config")
        .arg("--set")
        .arg("grid_size=300")
        .assert()
        .success()
        .stdout(predicate::str::contains("Config updated"));
}

#[test]
fn test_config_roundtrip() {
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    // Set a value
    cli_bin()
        .arg("config")
        .arg("--set")
        .arg("grid_size=250")
        .assert()
        .success();

    // Show and verify it's there
    cli_bin()
        .arg("config")
        .arg("--show")
        .assert()
        .success()
        .stdout(predicate::str::contains("grid_size"));
}

#[test]
fn test_search_no_token() {
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    cli_bin()
        .arg("search")
        .arg("cat")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Not configured"));
}

#[test]
fn test_config_set_token_never_echoes_value() {
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    cli_bin()
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
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    cli_bin()
        .arg("config")
        .arg("--set")
        .arg("refresh_token=super-secret-token-value")
        .assert()
        .success();

    cli_bin()
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
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    cli_bin()
        .arg("config")
        .arg("--set")
        .arg("grid_size=300")
        .assert()
        .success();

    let path = dir.path().join("pixiv-manager").join("config.toml");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0, "config must not be group/other readable");
}
