use blackbox::query::{ActivityEvent, ActivitySummary, RepoSummary};
use blackbox::output::{format_duration, render_summary_to_string, render_json, render_csv, OutputFormat};
use chrono::{Duration, Utc};

#[test]
fn format_duration_hours_and_minutes() {
    let d = Duration::minutes(90);
    assert_eq!(format_duration(d), "~1h 30m");
}

#[test]
fn format_duration_minutes_only() {
    let d = Duration::minutes(45);
    assert_eq!(format_duration(d), "~45m");
}

#[test]
fn format_duration_zero() {
    let d = Duration::zero();
    assert_eq!(format_duration(d), "~0m");
}

#[test]
fn format_duration_exact_hour() {
    let d = Duration::hours(2);
    assert_eq!(format_duration(d), "~2h 0m");
}

#[test]
fn render_summary_with_repos() {
    colored::control::set_override(false);

    let summary = ActivitySummary {
        period_label: "Today".to_string(),
        total_commits: 5,
        total_repos: 1,
        total_estimated_time: Duration::minutes(90),
        repos: vec![RepoSummary {
            repo_path: "/home/user/code/myproject".to_string(),
            repo_name: "myproject".to_string(),
            commits: 5,
            branches: vec!["main".to_string(), "feature-x".to_string()],
            estimated_time: Duration::minutes(90),
            events: vec![
                ActivityEvent {
                    event_type: "commit".to_string(),
                    branch: Some("main".to_string()),
                    commit_hash: Some("abc1234def5678".to_string()),
                    message: Some("fix login bug".to_string()),
                    timestamp: Utc::now(),
                },
                ActivityEvent {
                    event_type: "branch_switch".to_string(),
                    branch: Some("feature-x".to_string()),
                    commit_hash: None,
                    message: None,
                    timestamp: Utc::now(),
                },
                ActivityEvent {
                    event_type: "commit".to_string(),
                    branch: Some("feature-x".to_string()),
                    commit_hash: Some("def5678abc1234".to_string()),
                    message: Some("add new feature".to_string()),
                    timestamp: Utc::now(),
                },
            ],
        }],
    };

    let output = render_summary_to_string(&summary);
    assert!(output.contains("Today"), "should contain period label");
    assert!(output.contains("5 commits across 1 repo"), "should contain summary stats");
    assert!(output.contains("~1h 30m"), "should contain approximate time");
    assert!(output.contains("myproject"), "should contain repo name");
    assert!(output.contains("main"), "should contain branch");
    assert!(output.contains("feature-x"), "should contain branch");
    assert!(output.contains("abc1234"), "should contain short commit hash");
    assert!(output.contains("fix login bug"), "should contain commit message");
    assert!(output.contains("branch_switch"), "should show branch switch");
}

#[test]
fn render_empty_summary() {
    colored::control::set_override(false);

    let summary = ActivitySummary {
        period_label: "This Week".to_string(),
        total_commits: 0,
        total_repos: 0,
        total_estimated_time: Duration::zero(),
        repos: vec![],
    };

    let output = render_summary_to_string(&summary);
    assert!(output.contains("No activity recorded for This Week"), "should show no activity message");
}

fn make_test_summary() -> ActivitySummary {
    ActivitySummary {
        period_label: "Today".to_string(),
        total_commits: 3,
        total_repos: 1,
        total_estimated_time: Duration::minutes(45),
        repos: vec![RepoSummary {
            repo_path: "/home/user/code/myproject".to_string(),
            repo_name: "myproject".to_string(),
            commits: 3,
            branches: vec!["main".to_string()],
            estimated_time: Duration::minutes(45),
            events: vec![
                ActivityEvent {
                    event_type: "commit".to_string(),
                    branch: Some("main".to_string()),
                    commit_hash: Some("abc1234".to_string()),
                    message: Some("fix bug".to_string()),
                    timestamp: Utc::now(),
                },
                ActivityEvent {
                    event_type: "branch_switch".to_string(),
                    branch: Some("dev".to_string()),
                    commit_hash: None,
                    message: None,
                    timestamp: Utc::now(),
                },
            ],
        }],
    }
}

fn make_empty_summary() -> ActivitySummary {
    ActivitySummary {
        period_label: "This Week".to_string(),
        total_commits: 0,
        total_repos: 0,
        total_estimated_time: Duration::zero(),
        repos: vec![],
    }
}

// --- JSON tests ---

#[test]
fn render_json_is_valid_json() {
    let summary = make_test_summary();
    let json_str = render_json(&summary);
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("should be valid JSON");
    assert!(parsed.is_object());
}

#[test]
fn render_json_has_top_level_fields() {
    let summary = make_test_summary();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(v["period_label"], "Today");
    assert_eq!(v["total_commits"], 3);
    assert_eq!(v["total_repos"], 1);
    assert_eq!(v["total_estimated_minutes"], 45);
    assert!(v["repos"].is_array());
    assert_eq!(v["repos"].as_array().unwrap().len(), 1);
}

#[test]
fn render_json_repo_fields() {
    let summary = make_test_summary();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let repo = &v["repos"][0];
    assert_eq!(repo["repo_name"], "myproject");
    assert_eq!(repo["repo_path"], "/home/user/code/myproject");
    assert_eq!(repo["commits"], 3);
    assert!(repo["branches"].is_array());
    assert_eq!(repo["estimated_minutes"], 45);
    assert!(repo["events"].is_array());
    assert_eq!(repo["events"].as_array().unwrap().len(), 2);
}

#[test]
fn render_json_event_fields() {
    let summary = make_test_summary();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let event = &v["repos"][0]["events"][0];
    assert_eq!(event["event_type"], "commit");
    assert_eq!(event["branch"], "main");
    assert_eq!(event["commit_hash"], "abc1234");
    assert_eq!(event["message"], "fix bug");
    assert!(event["timestamp"].is_string());
}

#[test]
fn render_json_empty_summary() {
    let summary = make_empty_summary();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).expect("empty should be valid JSON");
    assert_eq!(v["repos"].as_array().unwrap().len(), 0);
    assert_eq!(v["total_commits"], 0);
}

// --- CSV tests ---

#[test]
fn render_csv_has_header() {
    let summary = make_test_summary();
    let csv_str = render_csv(&summary);
    let first_line = csv_str.lines().next().unwrap();
    assert_eq!(
        first_line,
        "period,repo_name,event_type,branch,commit_hash,message,timestamp,repo_estimated_minutes"
    );
}

#[test]
fn render_csv_row_count() {
    let summary = make_test_summary();
    let csv_str = render_csv(&summary);
    let lines: Vec<&str> = csv_str.lines().collect();
    // header + 2 events = 3 lines
    assert_eq!(lines.len(), 3);
}

#[test]
fn render_csv_empty_summary_header_only() {
    let summary = make_empty_summary();
    let csv_str = render_csv(&summary);
    let lines: Vec<&str> = csv_str.lines().collect();
    assert_eq!(lines.len(), 1, "empty summary should have header only");
    assert!(lines[0].contains("period"));
}

#[test]
fn output_format_enum_exists() {
    // Just verify the enum variants exist and Default works
    let _pretty = OutputFormat::Pretty;
    let _json = OutputFormat::Json;
    let _csv = OutputFormat::Csv;
    let default = OutputFormat::default();
    assert!(matches!(default, OutputFormat::Pretty));
}
