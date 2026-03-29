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
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
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
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
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

    assert!(output.status.success(), "standup should succeed with config");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // In piped mode (test), TTY auto-detection selects JSON output
    assert!(stdout.contains("No activity") || stdout.contains("**Today") || stdout.contains("period_label"), "should show standup output, got: {}", stdout);
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
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
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
    // In piped mode (test), TTY auto-detection selects JSON output
    assert!(stdout.contains("No activity") || stdout.contains("**This Week") || stdout.contains("period_label"), "should show week output, got: {}", stdout);
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
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
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
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
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
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
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

// --- US-018: CLI integration test: blackbox rhythm ---

/// Helper: init config + open DB + insert sample commits, returns (config_dir, data_dir)
fn setup_rhythm_env() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    // Init config
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
        .assert()
        .success();

    // Create DB with sample data
    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();
    let now = chrono::Utc::now();
    for i in 0..5 {
        let ts = now - chrono::Duration::hours(i);
        conn.execute(
            "INSERT INTO git_activity (repo_path, event_type, branch, commit_hash, author, message, timestamp)
             VALUES (?1, 'commit', 'main', ?2, 'tester', 'test commit', ?3)",
            rusqlite::params!["/tmp/repos/foo", format!("abc{i}"), ts.to_rfc3339()],
        )
        .unwrap();
    }

    (tmp, config_dir, data_dir)
}

#[test]
fn test_rhythm_exits_zero_with_populated_db() {
    let (_tmp, config_dir, data_dir) = setup_rhythm_env();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .arg("rhythm")
        .assert()
        .success()
        .stdout(predicate::str::contains("Work Rhythm"));
}

#[test]
fn test_rhythm_json_format_valid() {
    let (_tmp, config_dir, data_dir) = setup_rhythm_env();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["rhythm", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "rhythm --format json should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("stdout should be valid JSON");
    assert!(parsed.get("hour_histogram").is_some(), "JSON should contain hour_histogram");
    assert!(parsed.get("dow_histogram").is_some(), "JSON should contain dow_histogram");
}

#[test]
fn test_rhythm_days_zero_errors() {
    let (_tmp, config_dir, data_dir) = setup_rhythm_env();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["rhythm", "--days", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("days must be >= 1"));
}

// --- US-002: --json / --csv shorthand flags ---

/// Helper: init config + open DB (no data), returns (TempDir, config_dir, data_dir)
fn setup_empty_env() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
        .assert()
        .success();

    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    (tmp, config_dir, data_dir)
}

#[test]
fn test_today_json_flag_outputs_valid_json() {
    let (_tmp, config_dir, data_dir) = setup_empty_env();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["today", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "today --json should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("today --json stdout should be valid JSON");
    assert!(parsed.get("period_label").is_some());
}

#[test]
fn test_today_csv_flag_outputs_csv_header() {
    let (_tmp, config_dir, data_dir) = setup_empty_env();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["today", "--csv"])
        .output()
        .unwrap();

    assert!(output.status.success(), "today --csv should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("period,repo_name,event_type"), "should contain CSV header");
}

#[test]
fn test_week_json_flag_outputs_valid_json() {
    let (_tmp, config_dir, data_dir) = setup_empty_env();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["week", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "week --json should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<serde_json::Value>(&stdout)
        .expect("week --json stdout should be valid JSON");
}

#[test]
fn test_month_csv_flag_outputs_csv() {
    let (_tmp, config_dir, data_dir) = setup_empty_env();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["month", "--csv"])
        .output()
        .unwrap();

    assert!(output.status.success(), "month --csv should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("period,repo_name"), "should contain CSV header");
}

#[test]
fn test_json_and_csv_conflict() {
    let (_tmp, config_dir, data_dir) = setup_empty_env();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["today", "--json", "--csv"])
        .assert()
        .failure();
}

// --- US-004: Strip ANSI codes in non-TTY ---

#[test]
fn test_pretty_output_no_ansi_in_pipe() {
    let (_tmp, config_dir, data_dir) = setup_rhythm_env(); // has commits → non-empty pretty output

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["today", "--format", "pretty"])
        .output()
        .unwrap();

    assert!(output.status.success(), "today --format pretty should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("\x1b["),
        "piped pretty output should contain no ANSI escape sequences, got: {}",
        stdout
    );
}

#[test]
fn test_rhythm_pretty_no_ansi_in_pipe() {
    let (_tmp, config_dir, data_dir) = setup_rhythm_env();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["rhythm", "--format", "pretty"])
        .output()
        .unwrap();

    assert!(output.status.success(), "rhythm --format pretty should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // rhythm uses colored output (bold, cyan, green, yellow) — must be stripped in pipe
    assert!(
        !stdout.is_empty(),
        "rhythm output should not be empty"
    );
    assert!(
        !stdout.contains("\x1b["),
        "piped rhythm pretty output should contain no ANSI escape sequences, got: {}",
        stdout
    );
}

// --- US-005: Standup --json / --csv ---

#[test]
fn test_standup_json_flag_outputs_valid_json() {
    let (_tmp, config_dir, data_dir) = setup_empty_env();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["standup", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "standup --json should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("standup --json stdout should be valid JSON");
    assert!(parsed.get("period_label").is_some());
}

#[test]
fn test_standup_csv_flag_outputs_csv_header() {
    let (_tmp, config_dir, data_dir) = setup_empty_env();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["standup", "--csv"])
        .output()
        .unwrap();

    assert!(output.status.success(), "standup --csv should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("period,repo_name,event_type"), "should contain CSV header");
}

#[test]
fn test_standup_json_and_csv_conflict() {
    let (_tmp, config_dir, data_dir) = setup_empty_env();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["standup", "--json", "--csv"])
        .assert()
        .failure();
}

#[test]
fn test_standup_week_json_flag() {
    let (_tmp, config_dir, data_dir) = setup_empty_env();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["standup", "--week", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "standup --week --json should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("standup --week --json should be valid JSON");
    assert_eq!(parsed["period_label"], "This Week");
}

// --- US-006: --json help text describes JSON schema ---

#[test]
fn test_today_help_shows_json_schema() {
    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["today", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("period_label") && stdout.contains("total_commits") && stdout.contains("repos"),
        "--json help should describe JSON shape, got: {}",
        stdout
    );
}

#[test]
fn test_standup_help_shows_json_schema() {
    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["standup", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("period_label") && stdout.contains("total_commits"),
        "--json help on standup should describe JSON shape, got: {}",
        stdout
    );
}

#[test]
fn test_json_flag_overrides_format_pretty() {
    let (_tmp, config_dir, data_dir) = setup_empty_env();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["today", "--format", "pretty", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<serde_json::Value>(&stdout)
        .expect("--json should override --format pretty");
}
