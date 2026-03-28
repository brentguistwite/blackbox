use blackbox::db::{insert_activity, open_db};
use blackbox::output::render_hour_histogram;
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

// === US-002: render_hour_histogram tests ===

#[test]
fn render_hour_histogram_all_zeros_returns_empty_message() {
    colored::control::set_override(false);
    let hist = [0u32; 24];
    let output = render_hour_histogram(&hist);
    assert!(output.contains("No commit activity"), "should show empty message, got: {output}");
}

#[test]
fn render_hour_histogram_shows_all_24_hours() {
    colored::control::set_override(false);
    let mut hist = [0u32; 24];
    hist[10] = 5;
    let output = render_hour_histogram(&hist);
    // Should have a row for every hour 0–23
    for h in 0..24 {
        let label = format!("{h:>2} |");
        assert!(output.contains(&label), "missing hour {h} row in output");
    }
}

#[test]
fn render_hour_histogram_peak_label() {
    colored::control::set_override(false);
    let mut hist = [0u32; 24];
    hist[10] = 24;
    hist[9] = 18;
    hist[11] = 20;
    let output = render_hour_histogram(&hist);
    assert!(output.contains("Peak: 10:00"), "should identify peak hour 10, got: {output}");
    assert!(output.contains("24 commits"), "should show peak count");
}

#[test]
fn render_hour_histogram_bars_proportional() {
    colored::control::set_override(false);
    let mut hist = [0u32; 24];
    hist[10] = 20;
    hist[14] = 10; // half of peak
    let output = render_hour_histogram(&hist);
    let lines: Vec<&str> = output.lines().collect();
    // Find line for hour 10 and 14
    let line_10 = lines.iter().find(|l| l.starts_with("10 |")).unwrap();
    let line_14 = lines.iter().find(|l| l.starts_with("14 |")).unwrap();
    let bars_10 = line_10.matches('█').count();
    let bars_14 = line_14.matches('█').count();
    assert!(bars_10 > bars_14, "peak hour should have longer bar: {bars_10} vs {bars_14}");
    assert!(bars_14 > 0, "half-peak hour should have some bar chars");
}

#[test]
fn render_hour_histogram_zero_hours_show_no_bar() {
    colored::control::set_override(false);
    let mut hist = [0u32; 24];
    hist[10] = 5;
    let output = render_hour_histogram(&hist);
    let line_0 = output.lines().find(|l| l.starts_with(" 0 |")).unwrap();
    assert_eq!(line_0.matches('█').count(), 0, "zero-count hour should have no bar");
    assert!(line_0.contains(" 0"), "should show count 0");
}
