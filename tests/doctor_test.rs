use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_doctor_appears_in_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("doctor"));
}

#[test]
fn test_doctor_no_config_triggers_first_run() {
    // US-004: doctor with no config should trigger first-run setup, not doctor checks
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("Welcome to blackbox"))
        .stdout(predicate::str::contains("blackbox setup"));
}

#[test]
fn test_doctor_invalid_config_fails() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    let bb_config = config_dir.join("blackbox");
    std::fs::create_dir_all(&bb_config).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();

    // Write invalid TOML
    std::fs::write(bb_config.join("config.toml"), "not valid { toml").unwrap();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("doctor")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Config file"));
}

#[test]
fn test_doctor_valid_config_checks_pass() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    let bb_config = config_dir.join("blackbox");
    std::fs::create_dir_all(&bb_config).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();

    // Create a valid watch dir
    let watch_dir = tmp.path().join("repos");
    std::fs::create_dir_all(&watch_dir).unwrap();

    std::fs::write(
        bb_config.join("config.toml"),
        format!(
            "watch_dirs = [\"{}\"]\npoll_interval_secs = 300\n",
            watch_dir.display()
        ),
    )
    .unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("doctor")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Config and watch dir should pass
    assert!(stdout.contains("Config file"), "should check config");
    assert!(stdout.contains("Watch dir"), "should check watch dirs");
    assert!(stdout.contains("Database"), "should check database");
    assert!(stdout.contains("Daemon"), "should check daemon");
    assert!(stdout.contains("GitHub CLI"), "should check gh");
    assert!(stdout.contains("Shell hook"), "should check shell hook");
}

#[test]
fn test_doctor_missing_watch_dir() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    let bb_config = config_dir.join("blackbox");
    std::fs::create_dir_all(&bb_config).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();

    // Point to nonexistent watch dir
    std::fs::write(
        bb_config.join("config.toml"),
        "watch_dirs = [\"/tmp/nonexistent_blackbox_test_dir_xyz\"]\npoll_interval_secs = 300\n",
    )
    .unwrap();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("doctor")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Not found"));
}

#[test]
fn test_doctor_db_tables_exist() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    let bb_config = config_dir.join("blackbox");
    let bb_data = data_dir.join("blackbox");
    std::fs::create_dir_all(&bb_config).unwrap();
    std::fs::create_dir_all(&bb_data).unwrap();

    std::fs::write(
        bb_config.join("config.toml"),
        "watch_dirs = []\npoll_interval_secs = 300\n",
    )
    .unwrap();

    // Pre-create DB with migrations
    let db_path = bb_data.join("blackbox.db");
    blackbox::db::open_db(&db_path).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("doctor")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // DB check should pass (OK)
    assert!(
        stdout.contains("OK at"),
        "DB check should pass when tables exist, got: {}",
        stdout
    );
}

#[test]
fn test_doctor_output_includes_new_checks() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    let bb_config = config_dir.join("blackbox");
    std::fs::create_dir_all(&bb_config).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();

    std::fs::write(
        bb_config.join("config.toml"),
        "watch_dirs = []\npoll_interval_secs = 300\n",
    )
    .unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("doctor")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("LLM API key"), "doctor should check LLM key, got:\n{}", stdout);
    assert!(stdout.contains("Notifications"), "doctor should check notifications, got:\n{}", stdout);
    assert!(stdout.contains("AI tool:"), "doctor should enumerate AI tools, got:\n{}", stdout);
}


#[test]
fn test_doctor_check_count() {
    // With valid config + no watch dirs, we should see at least 5 checks:
    // config, database, daemon, gh, shell hook
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    let bb_config = config_dir.join("blackbox");
    std::fs::create_dir_all(&bb_config).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();

    std::fs::write(
        bb_config.join("config.toml"),
        "watch_dirs = []\npoll_interval_secs = 300\n",
    )
    .unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("doctor")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Count check lines (✓ or ✗)
    let check_count = stdout.lines().filter(|l| l.contains('✓') || l.contains('✗')).count();
    assert!(
        check_count >= 5,
        "should have at least 5 checks, got {}: {}",
        check_count,
        stdout
    );
}
