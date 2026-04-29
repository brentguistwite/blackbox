use blackbox::db;
use blackbox::poller::{
    probe_watch_dirs, record_repo_outcome, write_discovery_metrics, write_health_snapshot,
    write_poll_metrics, DiscoveryMetrics, HealthSnapshot, PollMetrics,
};
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
fn probe_watch_dirs_empty_list_returns_no_errors() {
    let errors = probe_watch_dirs(&[]);
    assert!(errors.is_empty());
}

#[test]
fn probe_watch_dirs_existing_dir_passes() {
    let tmp = TempDir::new().unwrap();
    let errors = probe_watch_dirs(&[tmp.path().to_path_buf()]);
    assert!(errors.is_empty(), "readable dir should not error");
}

#[test]
fn probe_watch_dirs_nonexistent_path_is_error() {
    let path = PathBuf::from("/nonexistent_blackbox_test_path_xyz_12345");
    let errors = probe_watch_dirs(&[path.clone()]);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].0, path);
    assert!(!errors[0].1.is_empty(), "error message should not be empty");
}

#[test]
fn probe_watch_dirs_returns_only_failing_entries() {
    let tmp = TempDir::new().unwrap();
    let bad = PathBuf::from("/nonexistent_blackbox_test_path_xyz_67890");
    let errors = probe_watch_dirs(&[tmp.path().to_path_buf(), bad.clone()]);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].0, bad);
}

#[test]
fn write_discovery_metrics_persists_count_and_sample() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();

    let metrics = DiscoveryMetrics {
        failures: vec![
            (PathBuf::from("/Users/me/Documents/flosports"), "Operation not permitted".into()),
            (PathBuf::from("/Users/me/Documents/personal"), "Operation not permitted".into()),
        ],
    };
    write_discovery_metrics(&conn, &metrics).unwrap();

    let count = db::get_daemon_state(&conn, "last_poll_discovery_failed")
        .unwrap()
        .unwrap();
    assert_eq!(count, "2");

    let sample = db::get_daemon_state(&conn, "last_poll_discovery_failed_sample")
        .unwrap()
        .unwrap();
    assert!(sample.contains("flosports"));
    assert!(sample.contains("personal"));
}

#[test]
fn watcher_recovery_path_clears_failure_metric_via_write_poll_metrics() {
    // Code-reviewer round 3: failure → success transition over the watcher
    // path must end with last_poll_repos_failed == "0". Wiring test for the
    // record_repo_outcome → metrics_from_set → write_poll_metrics chain.
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();

    let mut failed: HashSet<PathBuf> = HashSet::new();
    let path = PathBuf::from("/repo/flaky");

    // Cycle 1: watcher event reports failure.
    record_repo_outcome(&mut failed, &path, true);
    write_poll_metrics(&conn, &blackbox::poller::metrics_from_set(&failed)).unwrap();
    let after_fail = db::get_daemon_state(&conn, "last_poll_repos_failed").unwrap().unwrap();
    assert_eq!(after_fail, "1");

    // Cycle 2: subsequent watcher event for the same repo succeeds. Metric
    // must reset to 0 with empty sample so doctor stops alarming.
    record_repo_outcome(&mut failed, &path, false);
    write_poll_metrics(&conn, &blackbox::poller::metrics_from_set(&failed)).unwrap();
    let after_recovery = db::get_daemon_state(&conn, "last_poll_repos_failed").unwrap().unwrap();
    assert_eq!(after_recovery, "0");
    let sample = db::get_daemon_state(&conn, "last_poll_failed_sample").unwrap().unwrap_or_default();
    assert!(sample.trim().is_empty(), "sample should clear on recovery");
}

#[test]
fn write_discovery_metrics_zero_clears_previous_state() {
    // Recovery: a previous run had failures, this run is clean → metric must
    // reset to 0 so doctor stops alarming.
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();

    let with_errs = DiscoveryMetrics {
        failures: vec![(PathBuf::from("/x"), "denied".into())],
    };
    write_discovery_metrics(&conn, &with_errs).unwrap();

    let clean = DiscoveryMetrics { failures: vec![] };
    write_discovery_metrics(&conn, &clean).unwrap();

    let count = db::get_daemon_state(&conn, "last_poll_discovery_failed").unwrap().unwrap();
    assert_eq!(count, "0");
    let sample = db::get_daemon_state(&conn, "last_poll_discovery_failed_sample")
        .unwrap()
        .unwrap_or_default();
    assert!(sample.trim().is_empty());
}

#[test]
fn record_repo_outcome_independent_paths() {
    let mut set: HashSet<PathBuf> = HashSet::new();
    record_repo_outcome(&mut set, &PathBuf::from("/repo/a"), true);
    record_repo_outcome(&mut set, &PathBuf::from("/repo/b"), false);
    assert!(set.contains(&PathBuf::from("/repo/a")));
    assert!(!set.contains(&PathBuf::from("/repo/b")));
}

fn make_snap(mode: &str) -> HealthSnapshot {
    HealthSnapshot {
        last_poll_at: chrono::Utc::now(),
        poll_mode: mode.into(),
        repos_watched: 3,
        effective_poll_interval_secs: 600,
        poll_metrics: PollMetrics {
            failed_paths: vec![PathBuf::from("/repo/x")],
        },
        discovery_metrics: DiscoveryMetrics {
            failures: vec![(PathBuf::from("/watch/a"), "denied".into())],
        },
    }
}

#[test]
fn write_health_snapshot_writes_all_seven_keys() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();
    let snap = make_snap("watcher");

    write_health_snapshot(&conn, &snap).unwrap();

    for key in &[
        "last_poll_at",
        "last_poll_mode",
        "repos_watched",
        "effective_poll_interval_secs",
        "last_poll_repos_failed",
        "last_poll_failed_sample",
        "last_poll_discovery_failed",
        "last_poll_discovery_failed_sample",
    ] {
        let v = db::get_daemon_state(&conn, key).unwrap();
        assert!(v.is_some(), "key '{key}' must be set after snapshot write");
    }
    assert_eq!(db::get_daemon_state(&conn, "last_poll_mode").unwrap().unwrap(), "watcher");
    assert_eq!(db::get_daemon_state(&conn, "repos_watched").unwrap().unwrap(), "3");
    assert_eq!(db::get_daemon_state(&conn, "effective_poll_interval_secs").unwrap().unwrap(), "600");
    assert_eq!(db::get_daemon_state(&conn, "last_poll_repos_failed").unwrap().unwrap(), "1");
    assert_eq!(db::get_daemon_state(&conn, "last_poll_discovery_failed").unwrap().unwrap(), "1");
}

#[test]
fn write_health_snapshot_mode_transitions_from_polling_to_watcher() {
    // Locks in the "mode derived live every snapshot" contract. An
    // implementer who hoists mode out of the loop would silently break
    // recovery from polling-fallback → watcher.
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();

    write_health_snapshot(&conn, &make_snap("polling")).unwrap();
    assert_eq!(db::get_daemon_state(&conn, "last_poll_mode").unwrap().unwrap(), "polling");

    write_health_snapshot(&conn, &make_snap("watcher")).unwrap();
    assert_eq!(db::get_daemon_state(&conn, "last_poll_mode").unwrap().unwrap(), "watcher");
}

#[test]
fn write_health_snapshot_rollback_preserves_prior_values() {
    // Pre-seed all 7 keys with known old values, then force write_health_snapshot
    // to fail by holding BEGIN EXCLUSIVE on a second connection. With a
    // tightened busy_timeout the call returns Err quickly. Assert NO key
    // advanced — the whole snapshot rolls back as a unit, preserving the
    // paired-keys invariant doctor depends on.
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let originals = [
        ("last_poll_at", "2026-04-01T00:00:00+00:00"),
        ("last_poll_mode", "polling"),
        ("repos_watched", "99"),
        ("effective_poll_interval_secs", "300"),
        ("last_poll_repos_failed", "3"),
        ("last_poll_failed_sample", "/old/a\n/old/b"),
        ("last_poll_discovery_failed", "2"),
        ("last_poll_discovery_failed_sample", "/old/watch"),
    ];
    for (k, v) in &originals {
        db::set_daemon_state(&conn, k, v).unwrap();
    }

    // Tighten primary conn's busy_timeout so the failure is fast (~50ms).
    conn.pragma_update(None, "busy_timeout", 10).unwrap();

    // Hold an exclusive write transaction on a second connection.
    let blocker = rusqlite::Connection::open(&db_path).unwrap();
    blocker.pragma_update(None, "busy_timeout", 10).unwrap();
    blocker.execute("BEGIN EXCLUSIVE", []).unwrap();

    let snap = HealthSnapshot {
        last_poll_at: chrono::Utc::now(),
        poll_mode: "watcher".into(),
        repos_watched: 999,
        effective_poll_interval_secs: 7200,
        poll_metrics: PollMetrics { failed_paths: vec![PathBuf::from("/new")] },
        discovery_metrics: DiscoveryMetrics { failures: vec![] },
    };
    let result = write_health_snapshot(&conn, &snap);
    assert!(result.is_err(), "snapshot write should fail under exclusive lock");

    drop(blocker);

    // Every key must still hold its ORIGINAL pre-seeded value — no partial advance.
    for (k, expected) in &originals {
        let actual = db::get_daemon_state(&conn, k).unwrap().unwrap();
        assert_eq!(
            &actual, expected,
            "key '{k}' must roll back to original on snapshot failure"
        );
    }
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
