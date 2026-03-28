use blackbox::db::{insert_activity, open_db};
use blackbox::query::commit_hour_histogram;
use chrono::{TimeZone, Utc, Local, Timelike};
use tempfile::NamedTempFile;

fn setup_db() -> (rusqlite::Connection, NamedTempFile) {
    let tmp = NamedTempFile::new().unwrap();
    let conn = open_db(tmp.path()).unwrap();
    (conn, tmp)
}

/// Helper: given a UTC hour, return the local hour it maps to on the test runner.
fn utc_hour_to_local(utc_hour: u32) -> usize {
    let dt = Utc.with_ymd_and_hms(2025, 1, 15, utc_hour, 0, 0).unwrap();
    dt.with_timezone(&Local).hour() as usize
}

#[test]
fn hour_histogram_empty_db() {
    let (conn, _tmp) = setup_db();
    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let hist = commit_hour_histogram(&conn, from, to).unwrap();
    assert_eq!(hist, [0u32; 24]);
}

#[test]
fn hour_histogram_counts_commits_by_local_hour() {
    let (conn, _tmp) = setup_db();

    // 3 commits at 10:00 UTC, 2 at 15:00 UTC
    for i in 0..3 {
        insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
            Some(&format!("aaa{i}")), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();
    }
    for i in 0..2 {
        insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
            Some(&format!("bbb{i}")), Some("dev"), Some("msg"), "2025-01-15T15:00:00Z").unwrap();
    }

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let hist = commit_hour_histogram(&conn, from, to).unwrap();

    let local_10 = utc_hour_to_local(10);
    let local_15 = utc_hour_to_local(15);
    assert_eq!(hist[local_10], 3);
    assert_eq!(hist[local_15], 2);

    // Total should be 5
    let total: u32 = hist.iter().sum();
    assert_eq!(total, 5);
}

#[test]
fn hour_histogram_excludes_non_commit_events() {
    let (conn, _tmp) = setup_db();

    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c1"), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "branch_switch", Some("feat"), None,
        None, Some("dev"), None, "2025-01-15T10:05:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "merge", Some("main"), None,
        Some("m1"), Some("dev"), Some("merge msg"), "2025-01-15T10:10:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let hist = commit_hour_histogram(&conn, from, to).unwrap();

    let total: u32 = hist.iter().sum();
    assert_eq!(total, 1, "only commit events counted");
}

#[test]
fn hour_histogram_respects_time_range() {
    let (conn, _tmp) = setup_db();

    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c1"), Some("dev"), Some("msg"), "2025-01-14T10:00:00Z").unwrap(); // outside range
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c2"), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap(); // inside range

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let hist = commit_hour_histogram(&conn, from, to).unwrap();

    let total: u32 = hist.iter().sum();
    assert_eq!(total, 1, "only commits within range counted");
}
