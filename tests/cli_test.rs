use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;
use blackbox::{config, db};

#[test]
fn test_cli_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
}

#[test]
fn test_init_creates_config_file() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
        .assert()
        .success();

    let config_path = config_dir.join("blackbox").join("config.toml");
    assert!(config_path.exists(), "config.toml should be created");
}

#[test]
fn test_init_config_contains_watch_dirs() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .args(["init", "--watch-dirs", "/tmp/repos,/tmp/work", "--poll-interval", "300"])
        .assert()
        .success();

    let content = fs::read_to_string(config_dir.join("blackbox/config.toml")).unwrap();
    assert!(content.contains("/tmp/repos"), "should contain first watch dir");
    assert!(content.contains("/tmp/work"), "should contain second watch dir");
}

#[test]
fn test_init_config_contains_poll_interval() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "120"])
        .assert()
        .success();

    let content = fs::read_to_string(config_dir.join("blackbox/config.toml")).unwrap();
    assert!(content.contains("120"), "should contain poll interval 120");
}

#[test]
fn test_init_existing_config_warns() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let bb_dir = config_dir.join("blackbox");
    fs::create_dir_all(&bb_dir).unwrap();
    let config_path = bb_dir.join("config.toml");
    fs::write(&config_path, "poll_interval_secs = 999\n").unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
        .assert()
        .success();

    // Should warn about existing config
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("already exists"),
        "should warn config already exists, got: {}",
        stdout
    );

    // Should NOT overwrite
    let content = fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("999"), "original config should be preserved");
}

#[test]
fn test_init_creates_parent_dirs() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("deeply/nested/config");
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
        .assert()
        .success();

    let config_path = config_dir.join("blackbox/config.toml");
    assert!(config_path.exists(), "should create deeply nested parent dirs");
}

#[test]
fn test_smoke_init_load_config_open_db() {
    let tmp = TempDir::new().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");

    // Step 1: Run blackbox init
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_DATA_HOME", &data_home)
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "120"])
        .assert()
        .success();

    // Step 2: Verify config.toml exists with correct content
    let config_path = config_home.join("blackbox/config.toml");
    assert!(config_path.exists(), "config.toml must exist after init");
    let content = fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("/tmp/repos"));
    assert!(content.contains("120"));

    // Step 3: Load config programmatically (with env override)
    // SAFETY: single-threaded test context, no other threads reading these vars
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        std::env::set_var("XDG_DATA_HOME", &data_home);
    }
    let cfg = config::load_config().expect("load_config should succeed after init");
    assert_eq!(cfg.poll_interval_secs, 120);
    assert_eq!(cfg.watch_dirs.len(), 1);

    // Step 4: Open DB in temp data dir
    let db_path = data_home.join("blackbox/blackbox.db");
    let conn = db::open_db(&db_path).expect("open_db should succeed");

    // Step 5: Verify WAL mode
    let journal: String = conn
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(journal.to_lowercase(), "wal");

    // Step 6: Verify git_activity table exists
    let table_exists: bool = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='git_activity'")
        .unwrap()
        .exists([])
        .unwrap();
    assert!(table_exists, "git_activity table must exist");

    // Step 7: blackbox --help returns 0
    Command::cargo_bin("blackbox")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
}

#[test]
fn test_completions_zsh() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("compdef"))
        .stdout(predicate::str::contains("blackbox"));
}

#[test]
fn test_completions_bash() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete"))
        .stdout(predicate::str::contains("blackbox"));
}

#[test]
fn test_completions_fish() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete"))
        .stdout(predicate::str::contains("blackbox"));
}

#[test]
fn test_completions_covers_subcommands() {
    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["completions", "zsh"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    for subcmd in ["today", "week", "month", "start", "stop", "status", "init", "completions"] {
        assert!(stdout.contains(subcmd), "completions should cover '{}' subcommand", subcmd);
    }
}

#[test]
fn test_completions_invalid_shell() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["completions", "nushell"])
        .assert()
        .failure();
}
