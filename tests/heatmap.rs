use assert_cmd::Command;
use tempfile::TempDir;

fn setup_temp_env() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(config_dir.join("blackbox")).unwrap();
    std::fs::write(
        config_dir.join("blackbox").join("config.toml"),
        "watch_dirs = []\npoll_interval_secs = 60\n",
    )
    .unwrap();
    (tmp, data_dir, config_dir)
}

fn open_test_db(data_dir: &std::path::Path) -> rusqlite::Connection {
    let db_path = data_dir.join("blackbox").join("blackbox.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    blackbox::db::open_db(&db_path).unwrap()
}

#[test]
fn heatmap_exits_zero_with_empty_db() {
    let (_tmp, data_dir, config_dir) = setup_temp_env();
    let _conn = open_test_db(&data_dir);

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .arg("heatmap")
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .output()
        .unwrap();

    assert!(output.status.success(), "heatmap should exit 0 with empty DB");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No commits"),
        "empty DB should show 'No commits' message, got: {stdout}"
    );
}

#[test]
fn heatmap_weeks_12_exits_zero() {
    let (_tmp, data_dir, config_dir) = setup_temp_env();
    let _conn = open_test_db(&data_dir);

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["heatmap", "--weeks", "12"])
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .output()
        .unwrap();

    assert!(output.status.success(), "heatmap --weeks 12 should exit 0");
}

#[test]
fn heatmap_weeks_zero_returns_error() {
    let (_tmp, data_dir, config_dir) = setup_temp_env();
    let _conn = open_test_db(&data_dir);

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["heatmap", "--weeks", "0"])
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .output()
        .unwrap();

    assert!(!output.status.success(), "heatmap --weeks 0 should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("weeks must be between 1 and 260"),
        "expected validation error, got: {stderr}"
    );
}

#[test]
fn heatmap_weeks_261_returns_error() {
    let (_tmp, data_dir, config_dir) = setup_temp_env();
    let _conn = open_test_db(&data_dir);

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["heatmap", "--weeks", "261"])
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .output()
        .unwrap();

    assert!(!output.status.success(), "heatmap --weeks 261 should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("weeks must be between 1 and 260"),
        "expected validation error, got: {stderr}"
    );
}

#[test]
fn heatmap_with_commits_exits_zero_and_contains_blocks() {
    let (_tmp, data_dir, config_dir) = setup_temp_env();
    let conn = open_test_db(&data_dir);

    // Insert 5 commits across 3 different dates (recent, so they appear in default 52-week range)
    let commits = [
        ("2026-03-28T10:00:00+00:00", "commit 1", "aaa001"),
        ("2026-03-28T14:00:00+00:00", "commit 2", "aaa002"),
        ("2026-03-27T09:00:00+00:00", "commit 3", "aaa003"),
        ("2026-03-27T18:00:00+00:00", "commit 4", "aaa004"),
        ("2026-03-25T12:00:00+00:00", "commit 5", "aaa005"),
    ];
    for (ts, msg, hash) in &commits {
        conn.execute(
            "INSERT INTO git_activity (repo_path, event_type, branch, commit_hash, author, message, timestamp)
             VALUES (?1, 'commit', 'main', ?2, 'tester', ?3, ?4)",
            rusqlite::params!["/tmp/test-repo", hash, msg, ts],
        )
        .unwrap();
    }

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .arg("heatmap")
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .output()
        .unwrap();

    assert!(output.status.success(), "heatmap should exit 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains('\u{2588}'),
        "stdout should contain block char (█), got: {stdout}"
    );
}
