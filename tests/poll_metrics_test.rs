use blackbox::db;
use blackbox::poller::{record_repo_outcome, write_poll_metrics, PollMetrics};
use std::collections::HashSet;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn write_poll_metrics_writes_failed_count() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();

    let metrics = PollMetrics {
        failed_paths: vec![
            PathBuf::from("/repo/a"),
            PathBuf::from("/repo/b"),
        ],
    };
    write_poll_metrics(&conn, &metrics).unwrap();

    let failed = db::get_daemon_state(&conn, "last_poll_repos_failed")
        .unwrap()
        .expect("failed count must be set");
    assert_eq!(failed, "2");
}

#[test]
fn write_poll_metrics_writes_sample_paths() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();

    let metrics = PollMetrics {
        failed_paths: vec![
            PathBuf::from("/repo/a"),
            PathBuf::from("/repo/b"),
        ],
    };
    write_poll_metrics(&conn, &metrics).unwrap();

    let sample = db::get_daemon_state(&conn, "last_poll_failed_sample")
        .unwrap()
        .expect("sample must be set");
    assert!(sample.contains("/repo/a"));
    assert!(sample.contains("/repo/b"));
}

#[test]
fn write_poll_metrics_caps_sample_at_five_paths() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();

    let metrics = PollMetrics {
        failed_paths: (0..10).map(|i| PathBuf::from(format!("/repo/{i}"))).collect(),
    };
    write_poll_metrics(&conn, &metrics).unwrap();

    let sample = db::get_daemon_state(&conn, "last_poll_failed_sample")
        .unwrap()
        .unwrap();
    let lines: Vec<&str> = sample.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines.len() <= 5, "sample should be capped, got {}", lines.len());

    // Failed count still reports the real total
    let failed = db::get_daemon_state(&conn, "last_poll_repos_failed")
        .unwrap()
        .unwrap();
    assert_eq!(failed, "10");
}

#[test]
fn record_repo_outcome_adds_failure() {
    let mut set: HashSet<PathBuf> = HashSet::new();
    let path = PathBuf::from("/repo/a");
    record_repo_outcome(&mut set, &path, true);
    assert!(set.contains(&path));
}

#[test]
fn record_repo_outcome_clears_failure_on_subsequent_success() {
    // The watcher-path bug Codex flagged: a repo that failed in full_scan
    // should drop out of the failure set as soon as a watcher-driven poll
    // succeeds — otherwise doctor reports stale failures forever.
    let mut set: HashSet<PathBuf> = HashSet::new();
    let path = PathBuf::from("/repo/a");
    record_repo_outcome(&mut set, &path, true);
    record_repo_outcome(&mut set, &path, false);
    assert!(!set.contains(&path), "success after failure should clear the entry");
}

#[test]
fn record_repo_outcome_idempotent_on_repeated_failure() {
    let mut set: HashSet<PathBuf> = HashSet::new();
    let path = PathBuf::from("/repo/a");
    record_repo_outcome(&mut set, &path, true);
    record_repo_outcome(&mut set, &path, true);
    assert_eq!(set.len(), 1);
}

#[test]
fn record_repo_outcome_independent_paths() {
    let mut set: HashSet<PathBuf> = HashSet::new();
    record_repo_outcome(&mut set, &PathBuf::from("/repo/a"), true);
    record_repo_outcome(&mut set, &PathBuf::from("/repo/b"), false);
    assert!(set.contains(&PathBuf::from("/repo/a")));
    assert!(!set.contains(&PathBuf::from("/repo/b")));
}

#[test]
fn write_poll_metrics_zero_failures_writes_zero_and_empty_sample() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();

    let metrics = PollMetrics { failed_paths: vec![] };
    write_poll_metrics(&conn, &metrics).unwrap();

    let failed = db::get_daemon_state(&conn, "last_poll_repos_failed")
        .unwrap()
        .unwrap();
    assert_eq!(failed, "0");

    let sample = db::get_daemon_state(&conn, "last_poll_failed_sample")
        .unwrap()
        .unwrap_or_default();
    assert!(sample.trim().is_empty(), "sample should be empty when no failures");
}
