use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Helper: set XDG env vars to isolate test from real config/data
fn setup_xdg(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &config_dir) };
    unsafe { std::env::set_var("XDG_DATA_HOME", &data_dir) };
    (config_dir, data_dir)
}

#[test]
fn test_pid_file_path() {
    let tmp = TempDir::new().unwrap();
    let (_config_dir, _data_dir) = setup_xdg(&tmp);

    let path = blackbox::daemon::pid_file_path().unwrap();
    assert!(path.to_string_lossy().ends_with("blackbox.pid"));
    assert!(path.to_string_lossy().contains("blackbox"));
}

#[test]
fn test_is_daemon_running_no_file() {
    let tmp = TempDir::new().unwrap();
    let (_config_dir, _data_dir) = setup_xdg(&tmp);

    let result = blackbox::daemon::is_daemon_running().unwrap();
    assert!(result.is_none());
}

#[test]
fn test_is_daemon_running_stale_pid() {
    let tmp = TempDir::new().unwrap();
    let (_config_dir, data_dir) = setup_xdg(&tmp);

    // Write a PID file with a non-existent PID
    let pid_dir = data_dir.join("blackbox");
    std::fs::create_dir_all(&pid_dir).unwrap();
    let pid_file = pid_dir.join("blackbox.pid");
    std::fs::write(&pid_file, "999999").unwrap();

    let result = blackbox::daemon::is_daemon_running().unwrap();
    assert!(result.is_none(), "Stale PID should return None");
    assert!(!pid_file.exists(), "Stale PID file should be cleaned up");
}

#[test]
fn test_stop_when_not_running() {
    let tmp = TempDir::new().unwrap();
    let (_config_dir, _data_dir) = setup_xdg(&tmp);

    // stop_daemon when not running should not error
    let result = blackbox::daemon::stop_daemon();
    assert!(result.is_ok());
}

#[test]
fn test_start_stop_integration() {
    let tmp = TempDir::new().unwrap();
    let (config_dir, data_dir) = setup_xdg(&tmp);

    // Create a config with empty watch_dirs and short poll interval
    let bb_config = config_dir.join("blackbox");
    std::fs::create_dir_all(&bb_config).unwrap();
    std::fs::write(
        bb_config.join("config.toml"),
        "watch_dirs = []\npoll_interval_secs = 60\n",
    )
    .unwrap();

    // Start daemon via CLI
    let bin = assert_cmd::cargo::cargo_bin("blackbox");
    let mut cmd = std::process::Command::new(&bin);
    cmd.env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir);
    let output = cmd.arg("start").output().unwrap();
    assert!(output.status.success(), "start failed: {}", String::from_utf8_lossy(&output.stderr));

    // PID file should exist
    let pid_file = data_dir.join("blackbox").join("blackbox.pid");
    // Give daemon a moment to fork and write PID
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(pid_file.exists(), "PID file should exist after start");

    // Status should say running
    let mut cmd = std::process::Command::new(&bin);
    cmd.env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir);
    let output = cmd.arg("status").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Running"),
        "status should say Running, got: {}",
        stdout
    );

    // Stop daemon
    let mut cmd = std::process::Command::new(&bin);
    cmd.env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir);
    let output = cmd.arg("stop").output().unwrap();
    assert!(output.status.success(), "stop failed: {}", String::from_utf8_lossy(&output.stderr));

    // PID file should be gone
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(!pid_file.exists(), "PID file should be removed after stop");
}

/// Helper: create a git repo with an initial commit using git2
fn create_test_repo(path: &Path) -> git2::Repository {
    let repo = git2::Repository::init(path).unwrap();
    {
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        let tree_id = {
            let mut index = repo.index().unwrap();
            let file = path.join("README.md");
            std::fs::write(&file, "# test").unwrap();
            index.add_path(Path::new("README.md")).unwrap();
            index.write().unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
            .unwrap();
    }
    repo
}

/// E2E: daemon discovers repo, detects new commit, records to DB
#[test]
#[ignore] // slow -- run with --ignored
fn test_e2e_daemon_records_commit() {
    let tmp = TempDir::new().unwrap();
    let (config_dir, data_dir) = setup_xdg(&tmp);

    // Create a test git repo
    let repo_dir = tmp.path().join("repos").join("myproject");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let repo = create_test_repo(&repo_dir);

    // Write config pointing at the repos dir with 1s poll interval
    let bb_config = config_dir.join("blackbox");
    std::fs::create_dir_all(&bb_config).unwrap();
    let config_content = format!(
        "watch_dirs = [\"{}\"]\npoll_interval_secs = 10\n",
        tmp.path().join("repos").to_string_lossy().replace('\\', "/")
    );
    std::fs::write(bb_config.join("config.toml"), &config_content).unwrap();

    // Start daemon
    let bin = assert_cmd::cargo::cargo_bin("blackbox");
    let mut cmd = std::process::Command::new(&bin);
    cmd.env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir);
    let output = cmd.arg("start").output().unwrap();
    assert!(
        output.status.success(),
        "start failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Wait for first poll cycle (seeds state)
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Make a new commit in the test repo
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
    let tree_id = {
        let mut index = repo.index().unwrap();
        let file = repo_dir.join("new_file.txt");
        std::fs::write(&file, "new content").unwrap();
        index.add_path(Path::new("new_file.txt")).unwrap();
        index.write().unwrap();
        index.write_tree().unwrap()
    };
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "second commit for e2e test",
        &tree,
        &[&head_commit],
    )
    .unwrap();

    // Wait for another poll cycle to detect the new commit
    std::thread::sleep(std::time::Duration::from_secs(12));

    // Stop daemon
    let mut cmd = std::process::Command::new(&bin);
    cmd.env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir);
    let output = cmd.arg("stop").output().unwrap();
    assert!(
        output.status.success(),
        "stop failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Open DB and verify commit was recorded
    let db_path = data_dir.join("blackbox").join("blackbox.db");
    assert!(db_path.exists(), "DB should exist");
    let conn = blackbox::db::open_db(&db_path).unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM git_activity WHERE event_type = 'commit' AND message LIKE '%second commit%'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert!(
        count >= 1,
        "DB should contain the second commit, found {} matching rows",
        count
    );
}
