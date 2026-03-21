use assert_cmd::Command;
use blackbox::{config, db, query::QueryRange};
use chrono::{TimeZone, Utc};
use predicates::prelude::*;
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
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
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
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos,/tmp/work",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let content = fs::read_to_string(config_dir.join("blackbox/config.toml")).unwrap();
    assert!(
        content.contains("/tmp/repos"),
        "should contain first watch dir"
    );
    assert!(
        content.contains("/tmp/work"),
        "should contain second watch dir"
    );
}

#[test]
fn test_init_config_contains_poll_interval() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "120",
        ])
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
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
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
    assert!(
        content.contains("999"),
        "original config should be preserved"
    );
}

#[test]
fn test_init_creates_parent_dirs() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("deeply/nested/config");
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let config_path = config_dir.join("blackbox/config.toml");
    assert!(
        config_path.exists(),
        "should create deeply nested parent dirs"
    );
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
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "120",
        ])
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
    for subcmd in [
        "today",
        "week",
        "month",
        "start",
        "stop",
        "status",
        "init",
        "completions",
    ] {
        assert!(
            stdout.contains(subcmd),
            "completions should cover '{}' subcommand",
            subcmd
        );
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

// --- US-004: First-run detection ---

#[test]
fn test_first_run_today_shows_welcome() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("today")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Welcome to blackbox"),
        "should show welcome message on first run, got: {}",
        stdout
    );
}

#[test]
fn test_first_run_week_shows_welcome() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("week")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Welcome to blackbox"),
        "should show welcome for week command too, got: {}",
        stdout
    );
}

#[test]
fn test_first_run_doctor_shows_welcome() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("doctor")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Welcome to blackbox"),
        "doctor should trigger first-run too, got: {}",
        stdout
    );
}

#[test]
fn test_first_run_completions_exempt() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .args(["completions", "zsh"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Welcome to blackbox"),
        "completions should NOT trigger first-run, got: {}",
        stdout
    );
    assert!(output.status.success());
}

#[test]
fn test_first_run_init_exempt() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Welcome to blackbox"),
        "init should NOT trigger first-run, got: {}",
        stdout
    );
    assert!(output.status.success());
}

#[test]
fn test_first_run_shows_manual_setup_hint() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("status")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // In non-interactive (test) context, dialoguer fails, so we get the fallback message
    assert!(
        stdout.contains("blackbox setup"),
        "should mention manual setup option, got: {}",
        stdout
    );
}

#[test]
fn test_first_run_setup_exempt() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .arg("setup")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Welcome to blackbox"),
        "setup should NOT trigger first-run, got: {}",
        stdout
    );
}

#[test]
fn test_setup_shows_wizard_header() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("setup")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("blackbox setup"),
        "setup should show wizard header, got: {}",
        stdout
    );
}

#[test]
fn test_first_run_exits_cleanly() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    // Even though there's no config, first-run should exit 0 (not error)
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("today")
        .assert()
        .success();
}

// --- US-010: Standup command ---

#[test]
fn test_standup_first_run_shows_welcome() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("standup")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Welcome to blackbox"),
        "standup should trigger first-run when no config, got: {}",
        stdout
    );
}

#[test]
fn test_standup_with_config_runs() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    // Create config and DB first
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    // Create the DB
    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("standup")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "standup should succeed with config"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No activity") || stdout.contains("**Today"),
        "should show standup output, got: {}",
        stdout
    );
}

#[test]
fn test_standup_week_flag() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["standup", "--week"])
        .output()
        .unwrap();

    assert!(output.status.success(), "standup --week should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No activity") || stdout.contains("**This Week"),
        "should show week output, got: {}",
        stdout
    );
}

// --- US-002: Yesterday and Query commands ---

#[test]
fn test_yesterday_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["yesterday", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("yesterday"));
}

#[test]
fn test_yesterday_runs_with_config() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("yesterday")
        .assert()
        .success();
}

#[test]
fn test_query_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["query", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--from"))
        .stdout(predicate::str::contains("--to"));
}

#[test]
fn test_query_runs_with_config() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["query", "--from", "2025-03-01", "--to", "2025-03-15"])
        .assert()
        .success();
}

#[test]
fn test_query_missing_to_fails() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["query", "--from", "2025-03-01"])
        .assert()
        .failure();
}

#[test]
fn test_query_missing_from_fails() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["query", "--to", "2025-03-15"])
        .assert()
        .failure();
}

#[test]
fn test_yesterday_format_flag() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["yesterday", "--format", "json"])
        .assert()
        .success();
}

#[test]
fn test_query_format_flag() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "query",
            "--from",
            "2025-03-01",
            "--to",
            "2025-03-15",
            "--format",
            "json",
        ])
        .assert()
        .success();
}

// --- US-005: QueryRange enum ---

#[test]
fn query_range_all_starts_at_epoch() {
    let (from, to) = QueryRange::All.to_range();
    let epoch = Utc.timestamp_opt(0, 0).single().expect("epoch");
    assert_eq!(from, epoch);
    assert!(to <= Utc::now() + chrono::Duration::seconds(1));
}

#[test]
fn query_range_all_variants_produce_valid_ranges() {
    for variant in [
        QueryRange::Today,
        QueryRange::Yesterday,
        QueryRange::Week,
        QueryRange::Month,
        QueryRange::All,
    ] {
        let (from, to) = variant.to_range();
        assert!(from <= to, "{:?}: start must be <= end", variant);
    }
}

#[test]
fn query_range_default_is_month() {
    let default = QueryRange::default();
    assert!(matches!(default, QueryRange::Month));
}

// --- US-007: Rhythms command ---

#[test]
fn test_rhythms_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["rhythms", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--range"))
        .stdout(predicate::str::contains("--format"));
}

#[test]
fn test_rhythms_runs_with_config() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("rhythms")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "rhythms should succeed with config"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No activity"),
        "empty DB should show no activity"
    );
}

#[test]
fn test_rhythms_range_flag() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["rhythms", "--range", "week"])
        .assert()
        .success();
}

// --- US-011: --summarize flag ---

#[test]
fn test_summarize_flag_accepted_today() {
    // --summarize should be a valid flag (even if it errors due to no API key)
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["today", "--summarize"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should fail with "No LLM API key" not "unknown flag"
    assert!(
        !stderr.contains("unexpected argument"),
        "--summarize should be a valid flag, got stderr: {}",
        stderr
    );
}

#[test]
fn test_summarize_no_api_key_shows_error() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["today", "--summarize"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No LLM API key") || stderr.contains("llm_api_key"),
        "should show helpful error about missing API key, got stderr: {}",
        stderr
    );
}

#[test]
fn test_summarize_flag_accepted_standup() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["standup", "--summarize"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "--summarize should be valid on standup, got stderr: {}",
        stderr
    );
}

// --- US-011: Standup --webhook flag ---

#[test]
fn test_standup_webhook_flag_accepted() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["standup", "--webhook", "https://hooks.example.com/test"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "--webhook should be valid on standup, got stderr: {}",
        stderr
    );
    // Standup text should still print to stdout
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No activity") || stdout.contains("**Today"),
        "standup should print to stdout even with --webhook, got: {}",
        stdout
    );
}

#[test]
fn test_standup_webhook_flag_with_invalid_url_still_prints() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["standup", "--webhook", "http://localhost:1/bad"])
        .output()
        .unwrap();

    // Should still succeed (webhook failure doesn't crash)
    assert!(
        output.status.success(),
        "standup should not crash on webhook failure"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No activity") || stdout.contains("**Today"),
        "standup should always print to stdout, got: {}",
        stdout
    );
}

// --- US-012: Heatmap command ---

#[test]
fn test_heatmap_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["heatmap", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--weeks"));
}

#[test]
fn test_heatmap_runs_with_config() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("heatmap")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "heatmap should succeed with config"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No activity") || stdout.contains("Contribution"),
        "should show heatmap output, got: {}",
        stdout
    );
}

#[test]
fn test_heatmap_weeks_flag() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["heatmap", "--weeks", "12"])
        .assert()
        .success();
}

// --- US-010: Streak command ---

#[test]
fn test_streak_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["streak", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("streak"));
}

#[test]
fn test_streak_runs_with_config() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("streak")
        .output()
        .unwrap();

    assert!(output.status.success(), "streak should succeed with config");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No streak") || stdout.contains("Coding Streak"),
        "should show streak output, got: {}",
        stdout
    );
}

#[test]
fn test_streak_first_run_shows_welcome() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("streak")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Welcome to blackbox"),
        "streak should trigger first-run when no config, got: {}",
        stdout
    );
}

// --- US-014: Tickets command ---

#[test]
fn test_tickets_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["tickets", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--range"))
        .stdout(predicate::str::contains("--format"));
}

#[test]
fn test_tickets_runs_with_config() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("tickets")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "tickets should succeed with config"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No ticket"),
        "empty DB should show no ticket activity"
    );
}

#[test]
fn test_tickets_range_flag() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["tickets", "--range", "week"])
        .assert()
        .success();
}

// --- US-015: Trends CLI tests ---

#[test]
fn test_trends_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["trends", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--format"));
}

#[test]
fn test_trends_runs_with_config() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("trends")
        .output()
        .unwrap();

    assert!(output.status.success(), "trends should succeed with config");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No activity") || stdout.contains("Activity Trends"),
        "should show trends output, got: {}",
        stdout
    );
}

#[test]
fn test_churn_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["churn", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--threshold"))
        .stdout(predicate::str::contains("--format"))
        .stdout(predicate::str::contains("--range"));
}

#[test]
fn test_churn_runs_with_config() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("churn")
        .output()
        .unwrap();

    assert!(output.status.success(), "churn should succeed with config");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No high-churn") || stdout.contains("Code Churn"),
        "should show churn output, got: {}",
        stdout
    );
}

#[test]
fn test_churn_threshold_flag() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["churn", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("threshold"));
}

#[test]
fn test_focus_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["focus", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--range"))
        .stdout(predicate::str::contains("--format"));
}

#[test]
fn test_focus_runs_with_config() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args([
            "init",
            "--watch-dirs",
            "/tmp/repos",
            "--poll-interval",
            "300",
        ])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("focus")
        .output()
        .unwrap();

    assert!(output.status.success(), "focus should succeed with config");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No deep work") || stdout.contains("Deep Work"),
        "should show focus output, got: {}",
        stdout
    );
}

#[test]
fn test_focus_range_flag() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["focus", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("range"));
}

// --- US-021: Retro command ---

#[test]
fn test_retro_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["retro", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--sprint"))
        .stdout(predicate::str::contains("--format"));
}

#[test]
fn test_retro_runs_with_config() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("config/blackbox");
    let data_dir = tmp.path().join("data");
    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&config_dir).unwrap();
    let config_path = config_dir.join("config.toml");
    fs::write(
        &config_path,
        "watch_dirs = [\"/tmp\"]\npoll_interval = 60\n",
    )
    .unwrap();
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", config_dir.parent().unwrap())
        .env("XDG_DATA_HOME", &data_dir)
        .arg("retro")
        .output()
        .unwrap();

    assert!(output.status.success(), "retro should succeed with config");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No activity") || stdout.contains("Sprint Retro"),
        "should show retro output, got: {}",
        stdout
    );
}

#[test]
fn test_retro_sprint_flag() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["retro", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sprint"));
}

// --- US-023: Metrics command ---

#[test]
fn test_metrics_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["metrics", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--range"))
        .stdout(predicate::str::contains("--format"));
}

#[test]
fn test_metrics_runs_with_config() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("config/blackbox");
    let data_dir = tmp.path().join("data");
    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&config_dir).unwrap();
    let config_path = config_dir.join("config.toml");
    fs::write(
        &config_path,
        "watch_dirs = [\"/tmp\"]\npoll_interval = 60\n",
    )
    .unwrap();
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", config_dir.parent().unwrap())
        .env("XDG_DATA_HOME", &data_dir)
        .arg("metrics")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "metrics should succeed with config"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No metrics") || stdout.contains("DORA-lite"),
        "should show metrics output, got: {}",
        stdout
    );
}

#[test]
fn test_metrics_range_flag() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .args(["metrics", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("range"));
}
