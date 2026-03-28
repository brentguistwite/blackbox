use blackbox::db::{insert_activity, open_db};
use blackbox::output::{render_hour_histogram, render_dow_histogram, render_after_hours_stats};
use blackbox::query::{commit_hour_histogram, commit_dow_histogram, after_hours_ratio, session_length_distribution};
use chrono::{Datelike, TimeZone, Utc, Local, Timelike};
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

/// Helper: given a UTC date, return the local weekday index (Mon=0..Sun=6).
fn utc_date_to_local_dow(year: i32, month: u32, day: u32, hour: u32) -> usize {
    let dt = Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap();
    dt.with_timezone(&Local).weekday().num_days_from_monday() as usize
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

// === US-003: commit_dow_histogram tests ===

#[test]
fn dow_histogram_empty_db() {
    let (conn, _tmp) = setup_db();
    let from = Utc.with_ymd_and_hms(2025, 1, 13, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 19, 23, 59, 59).unwrap();
    let hist = commit_dow_histogram(&conn, from, to).unwrap();
    assert_eq!(hist, [0u32; 7]);
}

#[test]
fn dow_histogram_counts_by_local_weekday() {
    let (conn, _tmp) = setup_db();

    // 2025-01-15 = Wednesday (UTC). Insert 3 commits.
    // 2025-01-18 = Saturday (UTC). Insert 2 commits.
    for i in 0..3 {
        insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
            Some(&format!("w{i}")), Some("dev"), Some("msg"), "2025-01-15T12:00:00Z").unwrap();
    }
    for i in 0..2 {
        insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
            Some(&format!("s{i}")), Some("dev"), Some("msg"), "2025-01-18T12:00:00Z").unwrap();
    }

    let from = Utc.with_ymd_and_hms(2025, 1, 13, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 19, 23, 59, 59).unwrap();
    let hist = commit_dow_histogram(&conn, from, to).unwrap();

    let wed_idx = utc_date_to_local_dow(2025, 1, 15, 12);
    let sat_idx = utc_date_to_local_dow(2025, 1, 18, 12);
    assert_eq!(hist[wed_idx], 3);
    assert_eq!(hist[sat_idx], 2);

    let total: u32 = hist.iter().sum();
    assert_eq!(total, 5);
}

#[test]
fn dow_histogram_excludes_non_commit_events() {
    let (conn, _tmp) = setup_db();

    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c1"), Some("dev"), Some("msg"), "2025-01-15T12:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "branch_switch", Some("feat"), None,
        None, Some("dev"), None, "2025-01-15T12:05:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 13, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 19, 23, 59, 59).unwrap();
    let hist = commit_dow_histogram(&conn, from, to).unwrap();

    let total: u32 = hist.iter().sum();
    assert_eq!(total, 1, "only commit events counted");
}

#[test]
fn dow_histogram_respects_time_range() {
    let (conn, _tmp) = setup_db();

    // Outside range
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c0"), Some("dev"), Some("msg"), "2025-01-12T12:00:00Z").unwrap();
    // Inside range
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c1"), Some("dev"), Some("msg"), "2025-01-15T12:00:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 13, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 19, 23, 59, 59).unwrap();
    let hist = commit_dow_histogram(&conn, from, to).unwrap();

    let total: u32 = hist.iter().sum();
    assert_eq!(total, 1, "only commits within range counted");
}

// === US-004: render_dow_histogram tests ===

#[test]
fn render_dow_histogram_all_zeros_returns_empty_message() {
    colored::control::set_override(false);
    let hist = [0u32; 7];
    let output = render_dow_histogram(&hist);
    assert!(output.contains("No commit activity"), "should show empty message, got: {output}");
}

#[test]
fn render_dow_histogram_shows_all_7_days() {
    colored::control::set_override(false);
    let mut hist = [0u32; 7];
    hist[2] = 5; // Wed
    let output = render_dow_histogram(&hist);
    for label in &["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] {
        assert!(output.contains(label), "missing day label {label} in output");
    }
}

#[test]
fn render_dow_histogram_weekend_marker() {
    colored::control::set_override(false);
    let mut hist = [0u32; 7];
    hist[5] = 3; // Sat
    hist[6] = 2; // Sun
    hist[0] = 1; // Mon
    let output = render_dow_histogram(&hist);
    // Sat and Sun histogram rows (containing "|") should have [wknd] marker
    for line in output.lines() {
        if !line.contains('|') { continue; }
        if line.contains("Sat") || line.contains("Sun") {
            assert!(line.contains("[wknd]"), "weekend row should have [wknd] marker: {line}");
        }
        if line.starts_with("Mon") {
            assert!(!line.contains("[wknd]"), "weekday row should NOT have [wknd] marker: {line}");
        }
    }
}

#[test]
fn render_dow_histogram_bars_proportional() {
    colored::control::set_override(false);
    let hist = [10, 5, 0, 0, 0, 0, 0]; // Mon=10, Tue=5
    let output = render_dow_histogram(&hist);
    let lines: Vec<&str> = output.lines().collect();
    let line_mon = lines.iter().find(|l| l.contains("Mon")).unwrap();
    let line_tue = lines.iter().find(|l| l.contains("Tue")).unwrap();
    let bars_mon = line_mon.matches('█').count();
    let bars_tue = line_tue.matches('█').count();
    assert!(bars_mon > bars_tue, "Mon (10) should have longer bar than Tue (5): {bars_mon} vs {bars_tue}");
    assert!(bars_tue > 0, "Tue should have some bar chars");
}

#[test]
fn render_dow_histogram_peak_label() {
    colored::control::set_override(false);
    let hist = [5, 12, 3, 0, 0, 0, 0]; // Tue=12 is peak
    let output = render_dow_histogram(&hist);
    assert!(output.contains("peak"), "should show peak indicator");
    assert!(output.contains("Tue"), "peak should reference Tue");
}

// === US-014: Integration test: day-of-week histogram query ===

#[test]
fn dow_histogram_wednesday_saturday_others_zero() {
    let (conn, _tmp) = setup_db();

    // 2025-01-15 12:00 UTC = Wednesday, 2025-01-18 12:00 UTC = Saturday
    for i in 0..3 {
        insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
            Some(&format!("w{i}")), Some("dev"), Some("msg"), "2025-01-15T12:00:00Z").unwrap();
    }
    for i in 0..2 {
        insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
            Some(&format!("s{i}")), Some("dev"), Some("msg"), "2025-01-18T12:00:00Z").unwrap();
    }

    let from = Utc.with_ymd_and_hms(2025, 1, 13, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 19, 23, 59, 59).unwrap();
    let hist = commit_dow_histogram(&conn, from, to).unwrap();

    let wed_idx = utc_date_to_local_dow(2025, 1, 15, 12);
    let sat_idx = utc_date_to_local_dow(2025, 1, 18, 12);

    assert!(hist[wed_idx] > 0, "Wednesday bucket should have commits");
    assert!(hist[sat_idx] > 0, "Saturday bucket should have commits");

    // All other DOW slots must be 0
    for (i, &count) in hist.iter().enumerate() {
        if i != wed_idx && i != sat_idx {
            assert_eq!(count, 0, "DOW index {i} should be 0, got {count}");
        }
    }
}

#[test]
fn dow_histogram_empty_returns_all_zeros() {
    let (conn, _tmp) = setup_db();
    let from = Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 6, 30, 23, 59, 59).unwrap();
    let hist = commit_dow_histogram(&conn, from, to).unwrap();
    assert_eq!(hist, [0u32; 7], "empty DB should return all-zero histogram");
}

// === US-005: after_hours_ratio tests ===

/// Helper: given a UTC hour on a specific date, determine if it's "after hours" in local time.
/// After-hours = local hour < 9 OR local hour >= 18.
fn is_after_hours_local(year: i32, month: u32, day: u32, utc_hour: u32) -> bool {
    let dt = Utc.with_ymd_and_hms(year, month, day, utc_hour, 0, 0).unwrap();
    let local_hour = dt.with_timezone(&Local).hour();
    local_hour < 9 || local_hour >= 18
}

/// Helper: given a UTC date, determine if it's a weekend in local time.
fn is_weekend_local(year: i32, month: u32, day: u32, utc_hour: u32) -> bool {
    let dt = Utc.with_ymd_and_hms(year, month, day, utc_hour, 0, 0).unwrap();
    let dow = dt.with_timezone(&Local).weekday().num_days_from_monday();
    dow >= 5 // Sat=5, Sun=6
}

#[test]
fn after_hours_ratio_empty_db() {
    let (conn, _tmp) = setup_db();
    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let stats = after_hours_ratio(&conn, from, to).unwrap();
    assert_eq!(stats.total_commits, 0);
    assert_eq!(stats.after_hours_commits, 0);
    assert_eq!(stats.weekend_commits, 0);
    assert_eq!(stats.after_hours_ratio, 0.0);
    assert_eq!(stats.weekend_ratio, 0.0);
}

#[test]
fn after_hours_ratio_mixed_commits() {
    let (conn, _tmp) = setup_db();

    // 2025-01-15 = Wednesday
    // Insert 3 commits at 10:00 UTC (core hours in most zones)
    // Insert 1 commit at 22:00 UTC (after hours in most zones)
    for i in 0..3 {
        insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
            Some(&format!("c{i}")), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();
    }
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c3"), Some("dev"), Some("msg"), "2025-01-15T22:00:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let stats = after_hours_ratio(&conn, from, to).unwrap();

    assert_eq!(stats.total_commits, 4);

    // Compute expected after-hours count based on local tz
    let ah_10 = is_after_hours_local(2025, 1, 15, 10);
    let ah_22 = is_after_hours_local(2025, 1, 15, 22);
    let expected_ah = if ah_10 { 3 } else { 0 } + if ah_22 { 1 } else { 0 };
    assert_eq!(stats.after_hours_commits, expected_ah);

    // Weekend: Wednesday is never weekend regardless of tz
    assert_eq!(stats.weekend_commits, 0);
    assert_eq!(stats.weekend_ratio, 0.0);

    // Ratio check
    let expected_ratio = expected_ah as f64 / 4.0;
    assert!((stats.after_hours_ratio - expected_ratio).abs() < 1e-9);
}

#[test]
fn after_hours_ratio_all_weekend() {
    let (conn, _tmp) = setup_db();

    // 2025-01-18 = Saturday, 12:00 UTC — weekend in all practical timezones
    for i in 0..5 {
        insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
            Some(&format!("w{i}")), Some("dev"), Some("msg"), "2025-01-18T12:00:00Z").unwrap();
    }

    let from = Utc.with_ymd_and_hms(2025, 1, 18, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 18, 23, 59, 59).unwrap();
    let stats = after_hours_ratio(&conn, from, to).unwrap();

    assert_eq!(stats.total_commits, 5);

    if is_weekend_local(2025, 1, 18, 12) {
        assert_eq!(stats.weekend_commits, 5);
        assert_eq!(stats.weekend_ratio, 1.0);
    }
}

#[test]
fn after_hours_ratio_excludes_non_commit_events() {
    let (conn, _tmp) = setup_db();

    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c1"), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "branch_switch", Some("feat"), None,
        None, Some("dev"), None, "2025-01-15T22:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "merge", Some("main"), None,
        Some("m1"), Some("dev"), Some("merge"), "2025-01-15T23:00:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let stats = after_hours_ratio(&conn, from, to).unwrap();

    assert_eq!(stats.total_commits, 1, "only commit events counted");
}

// === US-006: render_after_hours_stats tests ===

#[test]
fn render_after_hours_stats_typical() {
    colored::control::set_override(false);
    let stats = blackbox::query::AfterHoursStats {
        total_commits: 20,
        after_hours_commits: 3,
        weekend_commits: 2,
        after_hours_ratio: 0.15,
        weekend_ratio: 0.10,
    };
    let output = render_after_hours_stats(&stats);
    assert!(output.contains("After-hours:"), "should have after-hours label");
    assert!(output.contains("3/20"), "should show 3/20 commits");
    assert!(output.contains("15%"), "should show 15%");
    assert!(output.contains("Weekend:"), "should have weekend label");
    assert!(output.contains("2/20"), "should show 2/20 commits");
    assert!(output.contains("10%"), "should show 10%");
}

#[test]
fn render_after_hours_stats_zero_commits() {
    colored::control::set_override(false);
    let stats = blackbox::query::AfterHoursStats {
        total_commits: 0,
        after_hours_commits: 0,
        weekend_commits: 0,
        after_hours_ratio: 0.0,
        weekend_ratio: 0.0,
    };
    let output = render_after_hours_stats(&stats);
    assert!(output.contains("0/0"), "should handle zero gracefully");
    assert!(output.contains("0%"), "should show 0%");
}

#[test]
fn render_after_hours_stats_no_evaluative_language() {
    colored::control::set_override(false);
    let stats = blackbox::query::AfterHoursStats {
        total_commits: 10,
        after_hours_commits: 8,
        weekend_commits: 6,
        after_hours_ratio: 0.8,
        weekend_ratio: 0.6,
    };
    let output = render_after_hours_stats(&stats);
    let lower = output.to_lowercase();
    assert!(!lower.contains("bad"), "no evaluative language");
    assert!(!lower.contains("healthy"), "no evaluative language");
    assert!(!lower.contains("warning"), "no evaluative language");
    assert!(!lower.contains("good"), "no evaluative language");
    assert!(!lower.contains("concerning"), "no evaluative language");
}

#[test]
fn render_after_hours_stats_high_ratio_note() {
    colored::control::set_override(false);
    let stats = blackbox::query::AfterHoursStats {
        total_commits: 10,
        after_hours_commits: 6,
        weekend_commits: 0,
        after_hours_ratio: 0.6,
        weekend_ratio: 0.0,
    };
    let output = render_after_hours_stats(&stats);
    assert!(output.contains("more than half outside core hours"),
        "should show neutral note when after_hours_ratio > 0.5, got: {output}");
}

#[test]
fn render_after_hours_stats_no_note_at_50_percent() {
    colored::control::set_override(false);
    let stats = blackbox::query::AfterHoursStats {
        total_commits: 10,
        after_hours_commits: 5,
        weekend_commits: 0,
        after_hours_ratio: 0.5,
        weekend_ratio: 0.0,
    };
    let output = render_after_hours_stats(&stats);
    assert!(!output.contains("more than half"), "note only shown when ratio > 0.5, not ==0.5");
}

// === US-007: session_length_distribution tests ===

#[test]
fn session_distribution_empty_db() {
    let (conn, _tmp) = setup_db();
    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let dist = session_length_distribution(&conn, from, to, 120, 30).unwrap();
    assert!(dist.sessions.is_empty());
    assert_eq!(dist.median_minutes, 0);
    assert_eq!(dist.p90_minutes, 0);
    assert_eq!(dist.mean_minutes, 0);
}

#[test]
fn session_distribution_excludes_short_sessions() {
    let (conn, _tmp) = setup_db();

    // Single isolated commit = session of just the credit time.
    // With median gap fallback (< 2 commits), credit = first_commit_minutes = 2 min.
    // 2 min < 5 min threshold => excluded.
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c1"), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    // Use first_commit_minutes=2 so session is 2 min (< 5 min threshold)
    let dist = session_length_distribution(&conn, from, to, 120, 2).unwrap();
    assert!(dist.sessions.is_empty(), "sessions < 5 min should be excluded");
    assert_eq!(dist.median_minutes, 0);
}

#[test]
fn session_distribution_two_sessions() {
    let (conn, _tmp) = setup_db();

    // Session 1: commits at 10:00, 10:10, 10:20 (3 commits, 8 min gaps)
    // Median gap = 10 min. effective_credit = clamp(10, 5, 30) = 10.
    // effective_gap = clamp(30, 30, 120) = 30.
    // Session 1: [9:50, 10:20] = 30 min
    //
    // Session 2: commits at 14:00, 14:30, 15:00, 15:30 (4 commits, 30 min gaps)
    // All within 30 min gap => one session: [13:50, 15:30] = 100 min
    //
    // But wait — the function queries ALL commits across ALL repos in range,
    // then groups by session gap. Let me use separate repos to ensure distinct sessions,
    // or use a gap large enough between sessions.
    //
    // Actually, the function should aggregate across repos. Let me use a single repo
    // with a gap > session_gap between the two clusters.

    // Cluster 1: 10:00, 10:10, 10:20
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c1"), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c2"), Some("dev"), Some("msg"), "2025-01-15T10:10:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c3"), Some("dev"), Some("msg"), "2025-01-15T10:20:00Z").unwrap();

    // Gap of 4 hours (>> any session gap)
    // Cluster 2: 14:00, 14:30, 15:00, 15:30
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c4"), Some("dev"), Some("msg"), "2025-01-15T14:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c5"), Some("dev"), Some("msg"), "2025-01-15T14:30:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c6"), Some("dev"), Some("msg"), "2025-01-15T15:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c7"), Some("dev"), Some("msg"), "2025-01-15T15:30:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let dist = session_length_distribution(&conn, from, to, 120, 30).unwrap();

    // Should have 2 sessions, both >= 5 min
    assert_eq!(dist.sessions.len(), 2, "expected 2 sessions, got {}", dist.sessions.len());

    // Both sessions should be > 5 min
    for s in &dist.sessions {
        assert!(s.num_minutes() >= 5, "session {} min should be >= 5", s.num_minutes());
    }

    // Median should be between the two session lengths
    assert!(dist.median_minutes > 0);
    assert!(dist.mean_minutes > 0);
    assert!(dist.p90_minutes >= dist.median_minutes);
}

#[test]
fn session_distribution_aggregates_across_repos() {
    let (conn, _tmp) = setup_db();

    // Repo A: session at 10:00-10:20
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("a1"), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("a2"), Some("dev"), Some("msg"), "2025-01-15T10:20:00Z").unwrap();

    // Repo B: session at 14:00-14:40 (separate repo, separate time)
    insert_activity(&conn, "/repo/b", "commit", Some("main"), None,
        Some("b1"), Some("dev"), Some("msg"), "2025-01-15T14:00:00Z").unwrap();
    insert_activity(&conn, "/repo/b", "commit", Some("main"), None,
        Some("b2"), Some("dev"), Some("msg"), "2025-01-15T14:40:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let dist = session_length_distribution(&conn, from, to, 120, 30).unwrap();

    // Should detect sessions from both repos
    assert!(dist.sessions.len() >= 2, "should aggregate across repos, got {} sessions", dist.sessions.len());
}

#[test]
fn session_distribution_only_counts_commits() {
    let (conn, _tmp) = setup_db();

    // Two commits forming a session
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c1"), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c2"), Some("dev"), Some("msg"), "2025-01-15T10:20:00Z").unwrap();

    // Non-commit events should not create additional sessions
    insert_activity(&conn, "/repo/a", "branch_switch", Some("feat"), None,
        None, Some("dev"), None, "2025-01-15T16:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "merge", Some("main"), None,
        Some("m1"), Some("dev"), Some("merge"), "2025-01-15T16:05:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let dist = session_length_distribution(&conn, from, to, 120, 30).unwrap();

    // Only 1 session from the 2 commits; branch_switch/merge don't count
    // (the non-commit events at 16:00 shouldn't form sessions)
    let total_sessions_over_5min: Vec<_> = dist.sessions.iter()
        .filter(|d| d.num_minutes() >= 5).collect();
    assert!(total_sessions_over_5min.len() <= 1,
        "non-commit events should not create sessions");
}
