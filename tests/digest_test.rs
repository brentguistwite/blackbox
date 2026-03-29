use assert_cmd::Command;
use blackbox::db;
use blackbox::output::{render_digest_csv, render_digest_json, render_digest_to_string, WeeklyDigest};
use blackbox::query::{
    digest_week_range, ActivityEvent, ActivitySummary, RepoSummary,
};
use chrono::{Datelike, Duration, TimeZone, Timelike, Utc, Weekday};
use tempfile::TempDir;

#[test]
fn test_digest_week_range_last_week_monday_start() {
    // For any "now", offset=-1 should give previous full Mon-Sun week
    let (start, end) = digest_week_range(-1, Weekday::Mon);

    // start should be a Monday at midnight local (converted to UTC)
    let start_local = start.with_timezone(&chrono::Local);
    assert_eq!(start_local.weekday(), Weekday::Mon);
    assert_eq!(start_local.hour(), 0);
    assert_eq!(start_local.minute(), 0);

    // end should be exactly 7 days after start (full week boundary)
    let diff = end - start;
    assert_eq!(diff.num_days(), 7);
}

#[test]
fn test_digest_week_range_current_week_capped_at_now() {
    let before = Utc::now();
    let (start, end) = digest_week_range(0, Weekday::Mon);
    let after = Utc::now();

    // start should be this week's Monday (or earlier if today is Mon)
    let start_local = start.with_timezone(&chrono::Local);
    assert_eq!(start_local.weekday(), Weekday::Mon);

    // end should be <= now (capped), not a full week boundary
    assert!(end >= before);
    assert!(end <= after + chrono::Duration::seconds(1));

    // end should be at most 7 days from start
    assert!(end - start <= chrono::Duration::weeks(1));
}

#[test]
fn test_digest_week_range_current_week_start_is_monday_midnight() {
    let (start, _end) = digest_week_range(0, Weekday::Mon);
    let start_local = start.with_timezone(&chrono::Local);
    assert_eq!(start_local.weekday(), Weekday::Mon);
    assert_eq!(start_local.hour(), 0);
    assert_eq!(start_local.minute(), 0);
    assert_eq!(start_local.second(), 0);
}

#[test]
fn test_digest_week_range_sunday_start() {
    let (start, _end) = digest_week_range(0, Weekday::Sun);
    let start_local = start.with_timezone(&chrono::Local);
    assert_eq!(start_local.weekday(), Weekday::Sun);
    assert_eq!(start_local.hour(), 0);
}

#[test]
fn test_digest_week_range_last_week_sunday_start() {
    let (start, end) = digest_week_range(-1, Weekday::Sun);
    let start_local = start.with_timezone(&chrono::Local);
    assert_eq!(start_local.weekday(), Weekday::Sun);
    // Past week: full 7-day range
    assert_eq!((end - start).num_days(), 7);
}

#[test]
fn test_digest_week_range_large_negative_offset() {
    // offset=-52 (a year ago) should not panic
    let (start, end) = digest_week_range(-52, Weekday::Mon);
    let start_local = start.with_timezone(&chrono::Local);
    assert_eq!(start_local.weekday(), Weekday::Mon);
    assert_eq!((end - start).num_days(), 7);
}

// --- Helper ---

fn make_event(event_type: &str, ts: chrono::DateTime<Utc>) -> ActivityEvent {
    ActivityEvent {
        event_type: event_type.to_string(),
        branch: Some("main".to_string()),
        commit_hash: Some("abc1234".to_string()),
        message: Some("test commit".to_string()),
        timestamp: ts,
    }
}

fn make_summary(
    label: &str,
    commits: usize,
    reviews: usize,
    repos: Vec<RepoSummary>,
    time_mins: i64,
    ai_mins: i64,
) -> ActivitySummary {
    ActivitySummary {
        period_label: label.to_string(),
        total_commits: commits,
        total_reviews: reviews,
        total_repos: repos.len(),
        total_estimated_time: Duration::minutes(time_mins),
        total_ai_session_time: Duration::minutes(ai_mins),
        streak_days: 0,
        total_branch_switches: 0,
        repos,
    }
}

fn make_repo(name: &str, commits: usize, time_mins: i64, events: Vec<ActivityEvent>) -> RepoSummary {
    RepoSummary {
        repo_path: format!("/tmp/{}", name),
        repo_name: name.to_string(),
        commits,
        branches: vec!["main".to_string()],
        estimated_time: Duration::minutes(time_mins),
        events,
        pr_info: None,
        reviews: vec![],
        ai_sessions: vec![],
        presence_intervals: vec![],
        branch_switches: 0,
    }
}

// --- US-4: Pretty formatter tests ---

#[test]
fn test_render_digest_header_contains_date_range() {
    // Week of Mar 24-30, 2025 (Mon-Sun)
    let week_start = Utc.with_ymd_and_hms(2025, 3, 24, 0, 0, 0).unwrap();
    let week_end = Utc.with_ymd_and_hms(2025, 3, 30, 23, 59, 59).unwrap();

    let digest = WeeklyDigest {
        current: make_summary("Week of Mar 24, 2025", 5, 0, vec![], 120, 0),
        previous: None,
        week_start,
        week_end,
    };

    let output = render_digest_to_string(&digest);
    assert!(output.contains("Weekly Digest"), "should contain 'Weekly Digest' header");
    assert!(output.contains("Mar 24"), "should contain start date");
    assert!(output.contains("Mar 30"), "should contain end date");
}

#[test]
fn test_render_digest_top_level_stats() {
    let week_start = Utc.with_ymd_and_hms(2025, 3, 24, 0, 0, 0).unwrap();
    let week_end = Utc.with_ymd_and_hms(2025, 3, 30, 23, 59, 59).unwrap();

    let digest = WeeklyDigest {
        current: make_summary("Week of Mar 24, 2025", 23, 2, vec![], 320, 70),
        previous: None,
        week_start,
        week_end,
    };

    let output = render_digest_to_string(&digest);
    assert!(output.contains("23 commits"), "should show commit count");
    assert!(output.contains("2 reviews"), "should show review count");
    assert!(output.contains("~5h 20m"), "should show total time");
    assert!(output.contains("AI:"), "should show AI session time");
}

#[test]
fn test_render_digest_daily_breakdown() {
    let week_start = Utc.with_ymd_and_hms(2025, 3, 24, 0, 0, 0).unwrap();
    let week_end = Utc.with_ymd_and_hms(2025, 3, 30, 23, 59, 59).unwrap();

    // Events on Mon Mar 24 and Wed Mar 26
    let mon_event = make_event("commit", Utc.with_ymd_and_hms(2025, 3, 24, 10, 0, 0).unwrap());
    let wed_event = make_event("commit", Utc.with_ymd_and_hms(2025, 3, 26, 14, 0, 0).unwrap());

    let repo = make_repo("myrepo", 2, 60, vec![mon_event, wed_event]);
    let digest = WeeklyDigest {
        current: make_summary("Week of Mar 24, 2025", 2, 0, vec![repo], 60, 0),
        previous: None,
        week_start,
        week_end,
    };

    let output = render_digest_to_string(&digest);
    // Mon and Wed should have commits
    assert!(output.contains("Mon"), "should list Monday");
    assert!(output.contains("Wed"), "should list Wednesday");
    // Days with no activity should show dash
    assert!(output.contains("\u{2014}"), "inactive days should show em-dash");
}

#[test]
fn test_render_digest_wow_comparison_present() {
    let week_start = Utc.with_ymd_and_hms(2025, 3, 24, 0, 0, 0).unwrap();
    let week_end = Utc.with_ymd_and_hms(2025, 3, 30, 23, 59, 59).unwrap();

    let digest = WeeklyDigest {
        current: make_summary("Week of Mar 24, 2025", 23, 2, vec![], 320, 70),
        previous: Some(make_summary("Week of Mar 17, 2025", 18, 2, vec![], 275, 50)),
        week_start,
        week_end,
    };

    let output = render_digest_to_string(&digest);
    assert!(output.contains("vs Last Week"), "should contain WoW section");
    assert!(output.contains("Commits"), "should compare commits");
    assert!(output.contains("Time"), "should compare time");
}

#[test]
fn test_render_digest_wow_comparison_omitted_when_no_previous() {
    let week_start = Utc.with_ymd_and_hms(2025, 3, 24, 0, 0, 0).unwrap();
    let week_end = Utc.with_ymd_and_hms(2025, 3, 30, 23, 59, 59).unwrap();

    let digest = WeeklyDigest {
        current: make_summary("Week of Mar 24, 2025", 23, 2, vec![], 320, 70),
        previous: None,
        week_start,
        week_end,
    };

    let output = render_digest_to_string(&digest);
    assert!(!output.contains("vs Last Week"), "should NOT contain WoW section when no previous");
}

#[test]
fn test_render_digest_empty_week() {
    let week_start = Utc.with_ymd_and_hms(2025, 3, 24, 0, 0, 0).unwrap();
    let week_end = Utc.with_ymd_and_hms(2025, 3, 30, 23, 59, 59).unwrap();

    let digest = WeeklyDigest {
        current: make_summary("Week of Mar 24, 2025", 0, 0, vec![], 0, 0),
        previous: None,
        week_start,
        week_end,
    };

    let output = render_digest_to_string(&digest);
    assert!(output.contains("No activity"), "empty week should say 'No activity'");
}

#[test]
fn test_render_digest_repos_section() {
    let week_start = Utc.with_ymd_and_hms(2025, 3, 24, 0, 0, 0).unwrap();
    let week_end = Utc.with_ymd_and_hms(2025, 3, 30, 23, 59, 59).unwrap();

    let event = make_event("commit", Utc.with_ymd_and_hms(2025, 3, 24, 10, 0, 0).unwrap());
    let repo = make_repo("blackbox", 1, 30, vec![event]);

    let digest = WeeklyDigest {
        current: make_summary("Week of Mar 24, 2025", 1, 0, vec![repo], 30, 0),
        previous: None,
        week_start,
        week_end,
    };

    let output = render_digest_to_string(&digest);
    assert!(output.contains("Repos"), "should contain Repos section");
    assert!(output.contains("blackbox"), "should list repo name");
}

// --- US-5: JSON output tests ---

#[test]
fn test_render_digest_json_parses_cleanly() {
    let week_start = Utc.with_ymd_and_hms(2025, 3, 24, 0, 0, 0).unwrap();
    let week_end = Utc.with_ymd_and_hms(2025, 3, 30, 23, 59, 59).unwrap();

    let event = make_event("commit", Utc.with_ymd_and_hms(2025, 3, 24, 10, 0, 0).unwrap());
    let repo = make_repo("myrepo", 3, 90, vec![event]);

    let digest = WeeklyDigest {
        current: make_summary("Week of Mar 24, 2025", 3, 1, vec![repo], 90, 20),
        previous: Some(make_summary("Week of Mar 17, 2025", 5, 0, vec![], 60, 0)),
        week_start,
        week_end,
    };

    let json_str = render_digest_json(&digest);
    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .expect("render_digest_json should produce valid JSON");

    // Check top-level fields
    assert!(parsed["week_start"].is_string(), "should have week_start");
    assert!(parsed["week_end"].is_string(), "should have week_end");
    assert_eq!(parsed["total_commits"].as_u64(), Some(3));
    assert!(parsed["previous_week"].is_object(), "should have previous_week when provided");
    assert_eq!(parsed["previous_week"]["total_commits"].as_u64(), Some(5));
}

#[test]
fn test_render_digest_json_omits_previous_when_none() {
    let week_start = Utc.with_ymd_and_hms(2025, 3, 24, 0, 0, 0).unwrap();
    let week_end = Utc.with_ymd_and_hms(2025, 3, 30, 23, 59, 59).unwrap();

    let digest = WeeklyDigest {
        current: make_summary("Week of Mar 24, 2025", 1, 0, vec![], 10, 0),
        previous: None,
        week_start,
        week_end,
    };

    let json_str = render_digest_json(&digest);
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert!(parsed.get("previous_week").is_none(), "should omit previous_week when None");
}

#[test]
fn test_render_digest_json_empty_week() {
    let week_start = Utc.with_ymd_and_hms(2025, 3, 24, 0, 0, 0).unwrap();
    let week_end = Utc.with_ymd_and_hms(2025, 3, 30, 23, 59, 59).unwrap();

    let digest = WeeklyDigest {
        current: make_summary("Week of Mar 24, 2025", 0, 0, vec![], 0, 0),
        previous: None,
        week_start,
        week_end,
    };

    let json_str = render_digest_json(&digest);
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["total_commits"].as_u64(), Some(0));
}

// --- US-5: CSV output tests ---

#[test]
fn test_render_digest_csv_contains_week_range_period() {
    let week_start = Utc.with_ymd_and_hms(2025, 3, 24, 0, 0, 0).unwrap();
    let week_end = Utc.with_ymd_and_hms(2025, 3, 30, 23, 59, 59).unwrap();

    let event = make_event("commit", Utc.with_ymd_and_hms(2025, 3, 25, 12, 0, 0).unwrap());
    let repo = make_repo("myrepo", 1, 30, vec![event]);

    let digest = WeeklyDigest {
        current: make_summary("Week of Mar 24, 2025", 1, 0, vec![repo], 30, 0),
        previous: None,
        week_start,
        week_end,
    };

    let csv_str = render_digest_csv(&digest);
    // CSV should have a header and at least one data row
    assert!(csv_str.contains("period"), "CSV should have header with 'period'");
    // Period should reflect the week date range (local time), not the original label
    // The exact date depends on local timezone, but it should contain "Mar" and a year
    assert!(csv_str.contains("Mar"), "CSV period should contain month abbreviation");
    assert!(csv_str.contains("2025"), "CSV period should contain year");
}

// --- US-7: --output-file tests ---

#[test]
fn test_output_file_writes_json() {
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

    // Create DB
    let db_dir = data_dir.join("blackbox");
    std::fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let out_path = tmp.path().join("nested").join("digest.json");

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["digest", "--format", "json", "--output-file"])
        .arg(&out_path)
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .output()
        .unwrap();

    assert!(output.status.success(), "digest --output-file should succeed: {}", String::from_utf8_lossy(&output.stderr));
    assert!(out_path.exists(), "output file should be created");
    let contents = std::fs::read_to_string(&out_path).unwrap();
    let _parsed: serde_json::Value = serde_json::from_str(&contents)
        .expect("output file should contain valid JSON");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Digest written to"), "should print success message");
}

// --- US-9: CLI integration tests ---

#[test]
fn cli_digest_json_format_returns_valid_json() {
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

    let db_dir = data_dir.join("blackbox");
    std::fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["digest", "--format", "json"])
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("BLACKBOX_FORMAT", "json")
        .output()
        .unwrap();

    assert!(output.status.success(), "digest --format json should succeed: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("CLI json output should parse");
    assert!(parsed["week_start"].is_string());
    assert!(parsed["week_end"].is_string());
}

#[test]
fn cli_digest_pretty_empty_week_shows_no_activity() {
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

    let db_dir = data_dir.join("blackbox");
    std::fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["digest", "--format", "pretty", "--week=-52"])
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("BLACKBOX_FORMAT", "pretty")
        .output()
        .unwrap();

    assert!(output.status.success(), "digest --format pretty should succeed: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No activity"), "empty week should show 'No activity': got {stdout}");
}

#[test]
fn cli_digest_csv_format_succeeds() {
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

    let db_dir = data_dir.join("blackbox");
    std::fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["digest", "--format", "csv"])
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("BLACKBOX_FORMAT", "csv")
        .output()
        .unwrap();

    assert!(output.status.success(), "digest --format csv should succeed: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn cli_digest_week_offset_negative_succeeds() {
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

    let db_dir = data_dir.join("blackbox");
    std::fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["digest", "--format", "json", "--week=-1"])
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("BLACKBOX_FORMAT", "json")
        .output()
        .unwrap();

    assert!(output.status.success(), "digest --week -1 should succeed: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(parsed["week_start"].is_string());
}
