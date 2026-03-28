use blackbox::db::{insert_activity, open_db};
use blackbox::output::{render_hour_histogram, render_dow_histogram, render_after_hours_stats, render_session_distribution};
use blackbox::query::{commit_hour_histogram, commit_dow_histogram, after_hours_ratio, session_length_distribution, burst_pattern, CommitPattern};
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

// === US-008: render_session_distribution tests ===

#[test]
fn render_session_distribution_zero_sessions() {
    colored::control::set_override(false);
    let dist = blackbox::query::SessionDistribution {
        sessions: vec![],
        median_minutes: 0,
        p90_minutes: 0,
        mean_minutes: 0,
    };
    let output = render_session_distribution(&dist);
    assert!(output.contains("No sessions detected"), "should show empty message, got: {output}");
}

#[test]
fn render_session_distribution_shows_stats() {
    colored::control::set_override(false);
    let dist = blackbox::query::SessionDistribution {
        sessions: vec![chrono::Duration::minutes(45), chrono::Duration::minutes(130)],
        median_minutes: 45,
        p90_minutes: 130,
        mean_minutes: 65,
    };
    let output = render_session_distribution(&dist);
    assert!(output.contains("2 sessions"), "should show session count, got: {output}");
    assert!(output.contains("median"), "should show median label");
    assert!(output.contains("p90"), "should show p90 label");
    assert!(output.contains("mean"), "should show mean label");
    assert!(output.contains("~45m"), "should format median as ~45m");
    assert!(output.contains("~2h 10m"), "should format p90 as ~2h 10m");
    assert!(output.contains("~1h 5m"), "should format mean as ~1h 5m");
}

#[test]
fn render_session_distribution_single_session() {
    colored::control::set_override(false);
    let dist = blackbox::query::SessionDistribution {
        sessions: vec![chrono::Duration::minutes(30)],
        median_minutes: 30,
        p90_minutes: 30,
        mean_minutes: 30,
    };
    let output = render_session_distribution(&dist);
    assert!(output.contains("1 session"), "singular 'session' for count=1, got: {output}");
    assert!(!output.contains("1 sessions"), "should not say '1 sessions'");
}

#[test]
fn render_session_distribution_no_evaluative_language() {
    colored::control::set_override(false);
    let dist = blackbox::query::SessionDistribution {
        sessions: vec![chrono::Duration::minutes(10)],
        median_minutes: 10,
        p90_minutes: 10,
        mean_minutes: 10,
    };
    let output = render_session_distribution(&dist).to_lowercase();
    assert!(!output.contains("good"), "no evaluative language");
    assert!(!output.contains("bad"), "no evaluative language");
    assert!(!output.contains("healthy"), "no evaluative language");
    assert!(!output.contains("warning"), "no evaluative language");
}

// === US-015: Integration test: after-hours ratio ===

#[test]
fn after_hours_integration_core_and_after_hours_ratio() {
    let (conn, _tmp) = setup_db();

    // 2025-01-15 = Wednesday
    // 3 commits at 12:00 UTC (core hours in most zones)
    for i in 0..3 {
        insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
            Some(&format!("core{i}")), Some("dev"), Some("msg"), "2025-01-15T12:00:00Z").unwrap();
    }
    // 1 commit at 02:00 UTC (after hours in most zones)
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("late0"), Some("dev"), Some("msg"), "2025-01-15T02:00:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let stats = after_hours_ratio(&conn, from, to).unwrap();

    assert_eq!(stats.total_commits, 4);

    // Dynamically compute expected after-hours count based on local tz
    let ah_12 = is_after_hours_local(2025, 1, 15, 12);
    let ah_02 = is_after_hours_local(2025, 1, 15, 2);
    let expected_ah = if ah_12 { 3 } else { 0 } + if ah_02 { 1 } else { 0 };
    assert_eq!(stats.after_hours_commits, expected_ah);

    let expected_ratio = expected_ah as f64 / 4.0;
    assert!((stats.after_hours_ratio - expected_ratio).abs() < 1e-9,
        "expected ratio ~{expected_ratio}, got {}", stats.after_hours_ratio);
}

#[test]
fn after_hours_integration_all_weekend_ratio_is_one() {
    let (conn, _tmp) = setup_db();

    // 2025-01-18 = Saturday, 12:00 UTC — weekend in all practical zones
    for i in 0..4 {
        insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
            Some(&format!("wknd{i}")), Some("dev"), Some("msg"), "2025-01-18T12:00:00Z").unwrap();
    }

    let from = Utc.with_ymd_and_hms(2025, 1, 18, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 18, 23, 59, 59).unwrap();
    let stats = after_hours_ratio(&conn, from, to).unwrap();

    assert_eq!(stats.total_commits, 4);
    if is_weekend_local(2025, 1, 18, 12) {
        assert_eq!(stats.weekend_commits, 4);
        assert_eq!(stats.weekend_ratio, 1.0, "all-weekend should yield ratio 1.0");
    }
}

#[test]
fn after_hours_integration_zero_commits_ratios_zero() {
    let (conn, _tmp) = setup_db();

    let from = Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 6, 30, 23, 59, 59).unwrap();
    let stats = after_hours_ratio(&conn, from, to).unwrap();

    assert_eq!(stats.total_commits, 0);
    assert_eq!(stats.after_hours_commits, 0);
    assert_eq!(stats.weekend_commits, 0);
    assert_eq!(stats.after_hours_ratio, 0.0, "0 commits => ratio 0.0");
    assert_eq!(stats.weekend_ratio, 0.0, "0 commits => ratio 0.0");
}

// === US-016: Integration test: session distribution ===

#[test]
fn session_distribution_integration_two_sessions_median_45m() {
    let (conn, _tmp) = setup_db();

    // Session 1: 10:00, 10:20 → 2 commits, 20 min span
    // Session 2: 14:00, 14:10, 14:20, 14:30, 14:40, 14:50 → 6 commits, 50 min span
    //
    // All gaps (sorted): [10,10,10,10,10,20,220] → median=10 min
    // effective_credit = clamp(10, 5, 30) = 10 min
    // effective_gap = clamp(30, 30, 120) = 30 min
    //
    // Session 1: 20 + 10 = 30 min
    // Session 2: 50 + 10 = 60 min
    // Median of [30, 60] = 45 min

    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("s1a"), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("s1b"), Some("dev"), Some("msg"), "2025-01-15T10:20:00Z").unwrap();

    // 220-min gap → new session
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("s2a"), Some("dev"), Some("msg"), "2025-01-15T14:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("s2b"), Some("dev"), Some("msg"), "2025-01-15T14:10:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("s2c"), Some("dev"), Some("msg"), "2025-01-15T14:20:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("s2d"), Some("dev"), Some("msg"), "2025-01-15T14:30:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("s2e"), Some("dev"), Some("msg"), "2025-01-15T14:40:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("s2f"), Some("dev"), Some("msg"), "2025-01-15T14:50:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let dist = session_length_distribution(&conn, from, to, 120, 30).unwrap();

    assert_eq!(dist.sessions.len(), 2, "expected 2 sessions");
    assert_eq!(dist.median_minutes, 45, "median of 30m and 60m sessions should be 45m");
}

#[test]
fn session_distribution_integration_short_session_excluded() {
    let (conn, _tmp) = setup_db();

    // Single isolated commit → session = 0 + credit.
    // With no gaps, fallback: credit = first_commit_minutes param.
    // Use first_commit_minutes=2 → session = 2 min < 5 min → excluded.
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("lone"), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let dist = session_length_distribution(&conn, from, to, 120, 2).unwrap();

    assert!(dist.sessions.is_empty(), "session < 5 min should be excluded, returns 0 sessions");
    assert_eq!(dist.median_minutes, 0);
    assert_eq!(dist.p90_minutes, 0);
    assert_eq!(dist.mean_minutes, 0);
}

// ============================================================
// US-009: Burst vs steady commit pattern query
// ============================================================

#[test]
fn burst_pattern_insufficient_with_fewer_than_3_commits() {
    let (conn, _tmp) = setup_db();
    // Insert only 2 commits
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("aaa0"), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("aaa1"), Some("dev"), Some("msg"), "2025-01-15T11:00:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let stats = burst_pattern(&conn, from, to).unwrap();

    assert_eq!(stats.commit_count, 2);
    assert_eq!(stats.pattern, CommitPattern::Insufficient);
}

#[test]
fn burst_pattern_steady_with_uniform_gaps() {
    let (conn, _tmp) = setup_db();
    // 12 commits uniformly 30 min apart
    for i in 0..12 {
        let ts = format!("2025-01-15T{:02}:{:02}:00Z", 8 + (i * 30) / 60, (i * 30) % 60);
        insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
            Some(&format!("c{i:02}")), Some("dev"), Some("msg"), &ts).unwrap();
    }

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let stats = burst_pattern(&conn, from, to).unwrap();

    assert_eq!(stats.commit_count, 12);
    assert_eq!(stats.pattern, CommitPattern::Steady);
    assert!(stats.cv_of_gaps <= 1.0, "uniform gaps should have low CV, got {}", stats.cv_of_gaps);
}

#[test]
fn burst_pattern_burst_with_clustered_commits() {
    let (conn, _tmp) = setup_db();
    // 9 commits within 2 min of each other, then 1 commit 300 min later
    let base_secs: [u32; 9] = [0, 10, 20, 30, 40, 50, 65, 80, 100];
    for (i, &s) in base_secs.iter().enumerate() {
        let min = s / 60;
        let sec = s % 60;
        let ts = format!("2025-01-15T10:{min:02}:{sec:02}Z");
        insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
            Some(&format!("c{i:02}")), Some("dev"), Some("msg"), &ts).unwrap();
    }
    // 1 commit 5 hours later
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("c09"), Some("dev"), Some("msg"), "2025-01-15T15:00:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let stats = burst_pattern(&conn, from, to).unwrap();

    assert_eq!(stats.commit_count, 10);
    assert_eq!(stats.pattern, CommitPattern::Burst);
    assert!(stats.cv_of_gaps > 1.0, "bursty pattern should have high CV, got {}", stats.cv_of_gaps);
}

#[test]
fn burst_pattern_empty_db() {
    let (conn, _tmp) = setup_db();
    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let stats = burst_pattern(&conn, from, to).unwrap();

    assert_eq!(stats.commit_count, 0);
    assert_eq!(stats.pattern, CommitPattern::Insufficient);
    assert_eq!(stats.cv_of_gaps, 0.0);
}

#[test]
fn burst_pattern_ignores_non_commit_events() {
    let (conn, _tmp) = setup_db();
    // 5 branch_switch events + 2 commits = insufficient
    for i in 0..5 {
        let ts = format!("2025-01-15T{:02}:00:00Z", 8 + i);
        insert_activity(&conn, "/repo/a", "branch_switch", Some("feat"), None,
            None, Some("dev"), None, &ts).unwrap();
    }
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("aaa0"), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/a", "commit", Some("main"), None,
        Some("aaa1"), Some("dev"), Some("msg"), "2025-01-15T11:00:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let stats = burst_pattern(&conn, from, to).unwrap();

    assert_eq!(stats.commit_count, 2);
    assert_eq!(stats.pattern, CommitPattern::Insufficient);
}
