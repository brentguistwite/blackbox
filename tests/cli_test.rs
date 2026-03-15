use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

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
