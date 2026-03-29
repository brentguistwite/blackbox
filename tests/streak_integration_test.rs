use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;
use blackbox::db;

/// Helper: init config + open DB, returns (TempDir, config_dir, data_dir, Connection).
/// TempDir must be held alive for the duration of the test.
fn setup_env_with_commits(timestamps: &[chrono::DateTime<chrono::Utc>]) -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
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

    // Create DB with commit data
    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    for (i, ts) in timestamps.iter().enumerate() {
        conn.execute(
            "INSERT INTO git_activity (repo_path, event_type, branch, commit_hash, author, message, timestamp)
             VALUES (?1, 'commit', 'main', ?2, 'tester', 'test commit', ?3)",
            rusqlite::params!["/tmp/repos/foo", format!("hash{i}"), ts.to_rfc3339()],
        )
        .unwrap();
    }

    (tmp, config_dir, data_dir)
}

#[test]
fn today_json_streak_three_consecutive_days() {
    let now = chrono::Utc::now();
    let timestamps = vec![
        now,
        now - chrono::Duration::days(1),
        now - chrono::Duration::days(2),
    ];
    let (_tmp, config_dir, data_dir) = setup_env_with_commits(&timestamps);

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["today", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "today --format json should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .expect("stdout should be valid JSON");
    assert_eq!(v["streak_days"], 3, "3 consecutive days of commits → streak_days == 3, got: {}", v["streak_days"]);
}

#[test]
fn today_pretty_streak_three_consecutive_days() {
    // Disable ANSI colors so dimmed text renders as plain
    let now = chrono::Utc::now();
    let timestamps = vec![
        now,
        now - chrono::Duration::days(1),
        now - chrono::Duration::days(2),
    ];
    let (_tmp, config_dir, data_dir) = setup_env_with_commits(&timestamps);

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .env("NO_COLOR", "1")
        .arg("today")
        .output()
        .unwrap();

    assert!(output.status.success(), "today should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("3-day streak"),
        "pretty output should contain '3-day streak', got: {}",
        stdout
    );
}

#[test]
fn today_json_streak_one_day() {
    let now = chrono::Utc::now();
    let timestamps = vec![now];
    let (_tmp, config_dir, data_dir) = setup_env_with_commits(&timestamps);

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["today", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["streak_days"], 1, "single today commit → streak_days == 1, got: {}", v["streak_days"]);
}

#[test]
fn today_pretty_streak_one_day() {
    let now = chrono::Utc::now();
    let timestamps = vec![now];
    let (_tmp, config_dir, data_dir) = setup_env_with_commits(&timestamps);

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .env("NO_COLOR", "1")
        .arg("today")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1-day streak"),
        "pretty output should contain '1-day streak', got: {}",
        stdout
    );
}
