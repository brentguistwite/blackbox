use blackbox::query::{ActivityEvent, ActivitySummary, RepoSummary};
use blackbox::output::{format_duration, render_summary_to_string};
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
