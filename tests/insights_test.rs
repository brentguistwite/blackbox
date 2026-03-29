use blackbox::query::{aggregate_insights_data, ActivityEvent, RepoSummary};
use chrono::{Duration, Utc};

/// Helper: build a RepoSummary with commit events at given timestamps+messages.
fn repo(
    name: &str,
    events: Vec<(&str, Option<&str>)>, // (RFC3339 timestamp, optional message)
) -> RepoSummary {
    let activity_events: Vec<ActivityEvent> = events
        .iter()
        .map(|(ts, msg)| ActivityEvent {
            event_type: "commit".to_string(),
            branch: Some("main".to_string()),
            commit_hash: Some("abc123".to_string()),
            message: msg.map(|m| m.to_string()),
            timestamp: chrono::DateTime::parse_from_rfc3339(ts)
                .unwrap()
                .with_timezone(&Utc),
        })
        .collect();
    let commits = activity_events.len();
    RepoSummary {
        repo_path: format!("/repo/{}", name),
        repo_name: name.to_string(),
        commits,
        branches: vec!["main".to_string()],
        estimated_time: Duration::minutes(30 * commits as i64),
        events: activity_events,
        pr_info: None,
        reviews: vec![],
        ai_sessions: vec![],
        presence_intervals: vec![],
        branch_switches: 0,
    }
}

#[test]
fn bugfix_classification() {
    let repos = vec![repo(
        "alpha",
        vec![
            ("2025-01-13T10:00:00Z", Some("fix: login bug")),
            ("2025-01-13T11:00:00Z", Some("feat: add button")),
            ("2025-01-13T12:00:00Z", Some("bug: crash on startup")),
        ],
    )];
    let data = aggregate_insights_data(&repos, "This Week");
    assert_eq!(data.bugfix_commits, 2);
    assert_eq!(data.total_commits_with_msg, 3);
    assert_eq!(data.total_commits, 3);
}

#[test]
fn commits_by_dow_bucketing() {
    // 2025-01-13 = Monday, 2025-01-15 = Wednesday
    let repos = vec![repo(
        "alpha",
        vec![
            ("2025-01-13T10:00:00Z", Some("mon commit 1")),
            ("2025-01-13T11:00:00Z", Some("mon commit 2")),
            ("2025-01-15T10:00:00Z", Some("wed commit")),
        ],
    )];
    let data = aggregate_insights_data(&repos, "This Week");

    // Mon=0, Wed=2 in local time (depends on timezone but UTC 10:00 is likely same DOW)
    // The exact DOW index depends on Local timezone conversion, but total should be 3
    let total_dow: u32 = data.commits_by_dow.iter().sum();
    assert_eq!(total_dow, 3);
    assert_eq!(data.total_repos, 1);
}

#[test]
fn commits_by_hour_bucketing() {
    let repos = vec![repo(
        "alpha",
        vec![
            ("2025-01-13T10:00:00Z", Some("morning")),
            ("2025-01-13T10:30:00Z", Some("morning 2")),
            ("2025-01-13T15:00:00Z", Some("afternoon")),
        ],
    )];
    let data = aggregate_insights_data(&repos, "This Week");
    let total_hour: u32 = data.commits_by_hour.iter().sum();
    assert_eq!(total_hour, 3);
}

#[test]
fn empty_repos_returns_zeroed_struct() {
    let repos: Vec<RepoSummary> = vec![];
    let data = aggregate_insights_data(&repos, "This Week");
    assert_eq!(data.total_commits, 0);
    assert_eq!(data.total_repos, 0);
    assert_eq!(data.commits_by_dow, [0u32; 7]);
    assert_eq!(data.commits_by_hour, [0u32; 24]);
    assert_eq!(data.avg_msg_len_by_dow, [0.0f64; 7]);
    assert_eq!(data.bugfix_commits, 0);
    assert_eq!(data.total_commits_with_msg, 0);
    assert!(data.pr_merge_times_hours.is_empty());
    assert!(data.per_repo.is_empty());
}

#[test]
fn avg_msg_len_by_dow() {
    // Mon commit with 20-char msg, Fri commit with 8-char msg
    // 2025-01-13 = Monday, 2025-01-17 = Friday
    let repos = vec![repo(
        "alpha",
        vec![
            ("2025-01-13T10:00:00Z", Some("twelve chars here!!")), // 19 chars
            ("2025-01-17T10:00:00Z", Some("short!!!")),            // 8 chars
        ],
    )];
    let data = aggregate_insights_data(&repos, "This Week");

    // Find which DOW indices got populated (depends on local timezone)
    // At minimum: the two values should differ and be non-zero
    let non_zero: Vec<f64> = data.avg_msg_len_by_dow.iter().copied().filter(|&v| v > 0.0).collect();
    assert_eq!(non_zero.len(), 2);
}

#[test]
fn per_repo_stats() {
    let repos = vec![
        repo(
            "alpha",
            vec![
                ("2025-01-13T10:00:00Z", Some("commit one")),
                ("2025-01-13T11:00:00Z", Some("commit two")),
            ],
        ),
        repo(
            "beta",
            vec![("2025-01-14T10:00:00Z", Some("only commit"))],
        ),
    ];
    let data = aggregate_insights_data(&repos, "This Week");
    assert_eq!(data.per_repo.len(), 2);
    let alpha = data.per_repo.iter().find(|r| r.repo_name == "alpha").unwrap();
    assert_eq!(alpha.commits, 2);
    assert_eq!(alpha.branches_touched, 1);
    assert!(!alpha.has_prs);
    assert!(alpha.avg_commit_msg_len > 0.0);
}

#[test]
fn no_message_commits_skipped_for_msg_stats() {
    let repos = vec![repo(
        "alpha",
        vec![
            ("2025-01-13T10:00:00Z", None),
            ("2025-01-13T11:00:00Z", Some("has a message")),
        ],
    )];
    let data = aggregate_insights_data(&repos, "This Week");
    assert_eq!(data.total_commits, 2);
    assert_eq!(data.total_commits_with_msg, 1);
    // bugfix should be 0 (None skipped, "has a message" not a bugfix)
    assert_eq!(data.bugfix_commits, 0);
}

#[test]
fn period_label_passthrough() {
    let repos: Vec<RepoSummary> = vec![];
    let data = aggregate_insights_data(&repos, "This Month");
    assert_eq!(data.period_label, "This Month");
}
