use blackbox::query::{ActivityEvent, RepoSummary};
use blackbox::insights::{context_switches, ContextSwitchMetrics};
use chrono::{Duration, Utc};

/// Create an ActivityEvent with given type, N minutes before now.
fn make_event(event_type: &str, minutes_ago: i64) -> ActivityEvent {
    ActivityEvent {
        event_type: event_type.to_string(),
        branch: None,
        commit_hash: None,
        message: None,
        timestamp: Utc::now() - Duration::minutes(minutes_ago),
    }
}

/// Create a RepoSummary with given name, events, and estimated minutes.
fn make_repo(name: &str, events: Vec<ActivityEvent>, est_minutes: i64) -> RepoSummary {
    RepoSummary {
        repo_path: format!("/tmp/{}", name),
        repo_name: name.to_string(),
        commits: events.iter().filter(|e| e.event_type == "commit").count(),
        branches: vec![],
        estimated_time: Duration::minutes(est_minutes),
        events,
        pr_info: None,
        reviews: vec![],
        ai_sessions: vec![],
    }
}

#[test]
fn empty_repos_returns_zeros() {
    let result = context_switches(&[]);
    assert_eq!(result.branch_switches, 0);
    assert_eq!(result.repo_switches, 0);
    assert!((result.focus_score - 1.0).abs() < f64::EPSILON);
}

#[test]
fn counts_branch_switch_events() {
    let events = vec![
        make_event("commit", 60),
        make_event("branch_switch", 50),
        make_event("commit", 40),
        make_event("branch_switch", 30),
        make_event("branch_switch", 20),
        make_event("commit", 10),
    ];
    let repos = vec![make_repo("repo-a", events, 60)];
    let result = context_switches(&repos);
    assert_eq!(result.branch_switches, 3);
}

#[test]
fn counts_repo_switches_chronologically() {
    // Timeline: repo-a@60m, repo-b@50m, repo-b@40m, repo-a@30m, repo-c@20m
    // Transitions: a→b (1), b→a (2), a→c (3) = 3 repo switches
    let repos = vec![
        make_repo("repo-a", vec![
            make_event("commit", 60),
            make_event("commit", 30),
        ], 30),
        make_repo("repo-b", vec![
            make_event("commit", 50),
            make_event("commit", 40),
        ], 20),
        make_repo("repo-c", vec![
            make_event("commit", 20),
        ], 10),
    ];
    let result = context_switches(&repos);
    assert_eq!(result.repo_switches, 3);
}

#[test]
fn focus_score_decreases_with_more_switches() {
    // 2 branch switches in 1 repo, 60 min estimated = 1 hour
    // total_switches = 2, switches_per_hour = 2/1 = 2
    // focus_score = 1/(1+2) = 0.333...
    let events = vec![
        make_event("commit", 60),
        make_event("branch_switch", 40),
        make_event("branch_switch", 20),
        make_event("commit", 10),
    ];
    let repos = vec![make_repo("repo-a", events, 60)];
    let result = context_switches(&repos);
    let expected = 1.0 / (1.0 + 2.0); // 0.333...
    assert!((result.focus_score - expected).abs() < 0.001);
}
