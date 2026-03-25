use blackbox::insights::{
    active_dates, aggregate_time_per_ticket, context_switches, daily_commit_counts,
    deep_work_sessions, dora_lite_metrics, extract_ticket_ids, hourly_distribution, retro_summary,
    streak_info, weekly_rhythm, work_hours_analysis,
};
use blackbox::query::ActivitySummary;
use blackbox::query::{ActivityEvent, RepoSummary};
use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Timelike, Utc};

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
        make_repo(
            "repo-a",
            vec![make_event("commit", 60), make_event("commit", 30)],
            30,
        ),
        make_repo(
            "repo-b",
            vec![make_event("commit", 50), make_event("commit", 40)],
            20,
        ),
        make_repo("repo-c", vec![make_event("commit", 20)], 10),
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

#[test]
fn daily_commit_counts_empty_returns_empty() {
    let result = daily_commit_counts(&[]);
    assert!(result.is_empty());
}

/// Create event at a specific local date+hour (for deterministic date-bucketing tests).
fn make_event_at(event_type: &str, year: i32, month: u32, day: u32, hour: u32) -> ActivityEvent {
    let local_dt = Local
        .with_ymd_and_hms(year, month, day, hour, 0, 0)
        .unwrap();
    ActivityEvent {
        event_type: event_type.to_string(),
        branch: None,
        commit_hash: None,
        message: None,
        timestamp: local_dt.with_timezone(&Utc),
    }
}

#[test]
fn daily_commit_counts_groups_by_local_date() {
    // 2 commits on Mar 10, 1 on Mar 11, branch_switch ignored
    let events = vec![
        make_event_at("commit", 2025, 3, 10, 9),
        make_event_at("commit", 2025, 3, 10, 17),
        make_event_at("branch_switch", 2025, 3, 10, 12),
        make_event_at("commit", 2025, 3, 11, 10),
    ];
    let repos = vec![make_repo("repo-a", events, 60)];
    let counts = daily_commit_counts(&repos);

    let mar10 = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
    let mar11 = NaiveDate::from_ymd_opt(2025, 3, 11).unwrap();
    assert_eq!(counts[&mar10], 2);
    assert_eq!(counts[&mar11], 1);
    assert_eq!(counts.len(), 2); // no entry for branch_switch-only dates
}

#[test]
fn active_dates_empty_returns_empty() {
    let result = active_dates(&[]);
    assert!(result.is_empty());
}

#[test]
fn hourly_distribution_empty_returns_all_zeros() {
    let result = hourly_distribution(&[]);
    assert_eq!(result, [0usize; 24]);
}

#[test]
fn hourly_distribution_buckets_by_local_hour() {
    // Commit at local 10:00 → bucket 10, commit at local 00:05 → bucket 0
    let events = vec![
        make_event_at("commit", 2025, 3, 10, 10),
        make_event_at("commit", 2025, 3, 11, 0),
    ];
    let repos = vec![make_repo("repo-a", events, 30)];
    let result = hourly_distribution(&repos);
    assert_eq!(result[10], 1);
    assert_eq!(result[0], 1);
    assert_eq!(result.iter().sum::<usize>(), 2);
}

#[test]
fn hourly_distribution_ignores_non_commit_events() {
    let events = vec![
        make_event_at("commit", 2025, 3, 10, 14),
        make_event_at("branch_switch", 2025, 3, 10, 15),
        make_event_at("merge", 2025, 3, 10, 16),
    ];
    let repos = vec![make_repo("repo-a", events, 30)];
    let result = hourly_distribution(&repos);
    assert_eq!(result[14], 1);
    assert_eq!(result[15], 0); // branch_switch ignored
    assert_eq!(result[16], 0); // merge ignored
    assert_eq!(result.iter().sum::<usize>(), 1);
}

#[test]
fn weekly_rhythm_empty_returns_all_zeros() {
    let result = weekly_rhythm(&[]);
    assert_eq!(result, [0usize; 7]);
}

#[test]
fn weekly_rhythm_buckets_by_weekday() {
    // 2025-03-10 = Monday (bucket 0), 2025-03-12 = Wednesday (bucket 2)
    let events = vec![
        make_event_at("commit", 2025, 3, 10, 9),  // Mon
        make_event_at("commit", 2025, 3, 12, 14), // Wed
        make_event_at("commit", 2025, 3, 12, 16), // Wed again
    ];
    let repos = vec![make_repo("repo-a", events, 30)];
    let result = weekly_rhythm(&repos);
    assert_eq!(result[0], 1); // Monday
    assert_eq!(result[2], 2); // Wednesday
    assert_eq!(result.iter().sum::<usize>(), 3);
}

#[test]
fn weekly_rhythm_ignores_non_commit_events() {
    // 2025-03-10 = Monday
    let events = vec![
        make_event_at("commit", 2025, 3, 10, 9),
        make_event_at("branch_switch", 2025, 3, 10, 10),
        make_event_at("merge", 2025, 3, 10, 11),
    ];
    let repos = vec![make_repo("repo-a", events, 30)];
    let result = weekly_rhythm(&repos);
    assert_eq!(result[0], 1); // only the commit
    assert_eq!(result.iter().sum::<usize>(), 1);
}

#[test]
fn active_dates_sorted_unique_across_repos() {
    // repo-a has commits on Mar 10 and Mar 12
    // repo-b has commits on Mar 10 and Mar 11
    // result should be [Mar 10, Mar 11, Mar 12] — sorted, deduplicated
    let repos = vec![
        make_repo(
            "repo-a",
            vec![
                make_event_at("commit", 2025, 3, 10, 9),
                make_event_at("commit", 2025, 3, 12, 14),
            ],
            30,
        ),
        make_repo(
            "repo-b",
            vec![
                make_event_at("commit", 2025, 3, 10, 15),
                make_event_at("commit", 2025, 3, 11, 10),
            ],
            30,
        ),
    ];
    let dates = active_dates(&repos);

    let mar10 = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
    let mar11 = NaiveDate::from_ymd_opt(2025, 3, 11).unwrap();
    let mar12 = NaiveDate::from_ymd_opt(2025, 3, 12).unwrap();
    assert_eq!(dates, vec![mar10, mar11, mar12]);
}

// --- US-008: work_hours_analysis ---

#[test]
fn work_hours_analysis_empty_returns_zeros() {
    let result = work_hours_analysis(&[], 8, 18);
    assert_eq!(result.total_commits, 0);
    assert_eq!(result.after_hours_commits, 0);
    assert!((result.after_hours_pct - 0.0).abs() < f64::EPSILON);
    assert!(result.earliest_commit.is_none());
    assert!(result.latest_commit.is_none());
    assert_eq!(result.weekend_days_active, 0);
}

#[test]
fn work_hours_analysis_counts_after_hours_commits() {
    // Work hours 9-17. Commits at 8am (before), 12pm (during), 20pm (after)
    let events = vec![
        make_event_at("commit", 2025, 3, 10, 8), // Mon 8am — before work
        make_event_at("commit", 2025, 3, 10, 12), // Mon 12pm — during work
        make_event_at("commit", 2025, 3, 10, 20), // Mon 8pm — after work
        make_event_at("branch_switch", 2025, 3, 10, 21), // ignored (not commit)
    ];
    let repos = vec![make_repo("repo-a", events, 60)];
    let result = work_hours_analysis(&repos, 9, 17);
    assert_eq!(result.total_commits, 3);
    assert_eq!(result.after_hours_commits, 2); // 8am and 8pm
    // 2/3 ≈ 66.7%
    assert!((result.after_hours_pct - 66.666).abs() < 1.0);
}

#[test]
fn work_hours_analysis_earliest_latest_commit_times() {
    // Commits at 6am, 10am, 22pm
    let events = vec![
        make_event_at("commit", 2025, 3, 10, 6),
        make_event_at("commit", 2025, 3, 10, 10),
        make_event_at("commit", 2025, 3, 10, 22),
    ];
    let repos = vec![make_repo("repo-a", events, 60)];
    let result = work_hours_analysis(&repos, 8, 18);
    assert_eq!(result.earliest_commit.unwrap().hour(), 6);
    assert_eq!(result.latest_commit.unwrap().hour(), 22);
}

#[test]
fn work_hours_analysis_weekend_days_counts_unique_dates() {
    // Sat Mar 8 with 3 commits, Sun Mar 9 with 1 commit = 2 weekend days
    let events = vec![
        make_event_at("commit", 2025, 3, 8, 10), // Sat
        make_event_at("commit", 2025, 3, 8, 14), // Sat (same day)
        make_event_at("commit", 2025, 3, 8, 18), // Sat (same day)
        make_event_at("commit", 2025, 3, 9, 11), // Sun
    ];
    let repos = vec![make_repo("repo-a", events, 60)];
    let result = work_hours_analysis(&repos, 8, 18);
    assert_eq!(result.weekend_days_active, 2);
}

// --- US-009: streak_info ---

#[test]
fn streak_info_empty_returns_zeros() {
    let today = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
    let result = streak_info(&[], &[5, 6], today);
    assert_eq!(result.current_streak, 0);
    assert_eq!(result.longest_streak, 0);
    assert!(result.longest_streak_start.is_none());
    assert_eq!(result.active_days_30d, 0);
}

#[test]
fn streak_weekday_activity_with_weekend_rest() {
    // Mon Mar 10 – Fri Mar 14, 2025: all weekdays active, rest=[Sat,Sun]
    let events = vec![
        make_event_at("commit", 2025, 3, 10, 9), // Mon
        make_event_at("commit", 2025, 3, 11, 9), // Tue
        make_event_at("commit", 2025, 3, 12, 9), // Wed
        make_event_at("commit", 2025, 3, 13, 9), // Thu
        make_event_at("commit", 2025, 3, 14, 9), // Fri
    ];
    let repos = vec![make_repo("repo-a", events, 60)];
    let friday = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
    let result = streak_info(&repos, &[5, 6], friday);
    assert_eq!(result.current_streak, 5);
}

#[test]
fn streak_missing_weekday_breaks_streak() {
    // Mon, Tue active; Wed missing; Thu, Fri active → current = 2
    let events = vec![
        make_event_at("commit", 2025, 3, 10, 9), // Mon
        make_event_at("commit", 2025, 3, 11, 9), // Tue
        // Wed Mar 12 missing
        make_event_at("commit", 2025, 3, 13, 9), // Thu
        make_event_at("commit", 2025, 3, 14, 9), // Fri
    ];
    let repos = vec![make_repo("repo-a", events, 60)];
    let friday = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
    let result = streak_info(&repos, &[5, 6], friday);
    assert_eq!(result.current_streak, 2); // Thu + Fri only
}

#[test]
fn streak_longest_separate_from_current() {
    // Week 1 Mon-Fri (5 days), weekend rest, Week 2 Mon missing, Tue-Fri (4 days)
    // longest=5 (week 1), current=4 (week 2 Tue-Fri)
    let events = vec![
        make_event_at("commit", 2025, 3, 3, 9), // Mon W1
        make_event_at("commit", 2025, 3, 4, 9), // Tue
        make_event_at("commit", 2025, 3, 5, 9), // Wed
        make_event_at("commit", 2025, 3, 6, 9), // Thu
        make_event_at("commit", 2025, 3, 7, 9), // Fri
        // Sat 8, Sun 9 = rest
        // Mon 10 = missing (breaks)
        make_event_at("commit", 2025, 3, 11, 9), // Tue W2
        make_event_at("commit", 2025, 3, 12, 9), // Wed
        make_event_at("commit", 2025, 3, 13, 9), // Thu
        make_event_at("commit", 2025, 3, 14, 9), // Fri
    ];
    let repos = vec![make_repo("repo-a", events, 120)];
    let friday = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
    let result = streak_info(&repos, &[5, 6], friday);
    assert_eq!(result.current_streak, 4);
    assert_eq!(result.longest_streak, 5);
    assert_eq!(
        result.longest_streak_start,
        Some(NaiveDate::from_ymd_opt(2025, 3, 3).unwrap())
    );
}

#[test]
fn streak_active_days_30d_counts_recent() {
    let today = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
    let events = vec![
        make_event_at("commit", 2025, 2, 1, 9), // 41 days ago — outside
        make_event_at("commit", 2025, 3, 1, 9), // 13 days ago — inside
        make_event_at("commit", 2025, 3, 10, 9), // 4 days ago — inside
        make_event_at("commit", 2025, 3, 14, 9), // today — inside
    ];
    let repos = vec![make_repo("repo-a", events, 60)];
    let result = streak_info(&repos, &[5, 6], today);
    assert_eq!(result.active_days_30d, 3);
}

// --- US-013: ticket extraction ---

#[test]
fn extract_ticket_ids_finds_jira_style_in_branch() {
    let branches = vec!["feature/JIRA-123-fix-auth".to_string()];
    let patterns = vec![r"[A-Z]+-\d+".to_string()];
    let ids = extract_ticket_ids(&branches, &patterns);
    assert_eq!(ids, vec!["JIRA-123"]);
}

#[test]
fn extract_ticket_ids_finds_github_issue_with_custom_pattern() {
    let branches = vec!["fix/issue-#42-login".to_string()];
    let patterns = vec![r"#\d+".to_string()];
    let ids = extract_ticket_ids(&branches, &patterns);
    assert_eq!(ids, vec!["#42"]);
}

#[test]
fn extract_ticket_ids_deduplicates_across_branches() {
    let branches = vec![
        "feature/PROJ-100-part1".to_string(),
        "feature/PROJ-100-part2".to_string(),
    ];
    let patterns = vec![r"[A-Z]+-\d+".to_string()];
    let ids = extract_ticket_ids(&branches, &patterns);
    assert_eq!(ids, vec!["PROJ-100"]);
}

/// Helper: make_repo with specific branches
fn make_repo_with_branches(
    name: &str,
    branches: Vec<String>,
    events: Vec<ActivityEvent>,
    est_minutes: i64,
) -> RepoSummary {
    RepoSummary {
        repo_path: format!("/tmp/{}", name),
        repo_name: name.to_string(),
        commits: events.iter().filter(|e| e.event_type == "commit").count(),
        branches,
        estimated_time: Duration::minutes(est_minutes),
        events,
        pr_info: None,
        reviews: vec![],
        ai_sessions: vec![],
    }
}

#[test]
fn aggregate_time_per_ticket_groups_by_ticket() {
    // Two repos on same ticket JIRA-123
    let patterns = vec![r"[A-Z]+-\d+".to_string()];
    let repos = vec![
        make_repo_with_branches(
            "repo-a",
            vec!["feature/JIRA-123-auth".to_string()],
            vec![make_event("commit", 30)],
            60,
        ),
        make_repo_with_branches(
            "repo-b",
            vec!["feature/JIRA-123-tests".to_string()],
            vec![make_event("commit", 20)],
            40,
        ),
    ];
    let result = aggregate_time_per_ticket(&repos, &patterns);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].ticket_id, "JIRA-123");
    assert_eq!(result[0].commits, 2);
    assert_eq!(result[0].estimated_minutes, 100); // 60 + 40
}

#[test]
fn aggregate_time_per_ticket_splits_time_for_multi_ticket_repo() {
    // One repo matches two tickets → time split equally
    let patterns = vec![r"[A-Z]+-\d+".to_string()];
    let repos = vec![make_repo_with_branches(
        "repo-a",
        vec!["feature/JIRA-10-and-JIRA-20".to_string()],
        vec![make_event("commit", 30), make_event("commit", 20)],
        60,
    )];
    let result = aggregate_time_per_ticket(&repos, &patterns);
    assert_eq!(result.len(), 2);
    // Each ticket gets 60/2 = 30 minutes, 2/2 = 1 commit each
    let jira10 = result.iter().find(|t| t.ticket_id == "JIRA-10").unwrap();
    let jira20 = result.iter().find(|t| t.ticket_id == "JIRA-20").unwrap();
    assert_eq!(jira10.estimated_minutes, 30);
    assert_eq!(jira20.estimated_minutes, 30);
    assert_eq!(jira10.commits, 1);
    assert_eq!(jira20.commits, 1);
}

#[test]
fn aggregate_time_per_ticket_sorted_by_time_desc() {
    let patterns = vec![r"[A-Z]+-\d+".to_string()];
    let repos = vec![
        make_repo_with_branches(
            "repo-a",
            vec!["feature/AAA-1".to_string()],
            vec![make_event("commit", 10)],
            20,
        ),
        make_repo_with_branches(
            "repo-b",
            vec!["feature/BBB-2".to_string()],
            vec![make_event("commit", 10)],
            80,
        ),
    ];
    let result = aggregate_time_per_ticket(&repos, &patterns);
    assert_eq!(result[0].ticket_id, "BBB-2"); // 80 min first
    assert_eq!(result[1].ticket_id, "AAA-1"); // 20 min second
}

// --- US-019: deep_work_sessions ---

/// Create an event with a specific branch, N minutes before now.
fn make_branched_event(event_type: &str, branch: &str, minutes_ago: i64) -> ActivityEvent {
    ActivityEvent {
        event_type: event_type.to_string(),
        branch: Some(branch.to_string()),
        commit_hash: None,
        message: None,
        timestamp: Utc::now() - Duration::minutes(minutes_ago),
    }
}

#[test]
fn deep_work_90min_single_branch_detected() {
    // 90min of commits on "main" branch → qualifies at threshold=60
    let events = vec![
        make_branched_event("commit", "main", 90),
        make_branched_event("commit", "main", 60),
        make_branched_event("commit", "main", 30),
        make_branched_event("commit", "main", 0),
    ];
    let repos = vec![make_repo("repo-a", events, 90)];
    let sessions = deep_work_sessions(&repos, 60);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].repo_name, "repo-a");
    assert_eq!(sessions[0].branch, "main");
    assert!(sessions[0].duration_minutes >= 90);
    assert_eq!(sessions[0].commit_count, 4);
}

#[test]
fn deep_work_branch_switch_splits_into_short_runs() {
    // 90 min total, but branch_switch at 45min splits into two 45min runs → no deep work
    let events = vec![
        make_branched_event("commit", "main", 90),
        make_branched_event("commit", "main", 60),
        make_branched_event("branch_switch", "feature", 45),
        make_branched_event("commit", "feature", 30),
        make_branched_event("commit", "feature", 0),
    ];
    let repos = vec![make_repo("repo-a", events, 90)];
    let sessions = deep_work_sessions(&repos, 60);
    assert_eq!(sessions.len(), 0); // both runs < 60 min
}

#[test]
fn deep_work_single_commit_not_counted() {
    // One commit = 0 duration, never qualifies
    let events = vec![make_branched_event("commit", "main", 0)];
    let repos = vec![make_repo("repo-a", events, 10)];
    let sessions = deep_work_sessions(&repos, 60);
    assert_eq!(sessions.len(), 0);
}

#[test]
fn deep_work_sessions_sorted_by_duration_desc() {
    // Two repos: repo-a has 120min session, repo-b has 90min session
    let repos = vec![
        make_repo(
            "repo-b",
            vec![
                make_branched_event("commit", "feat", 90),
                make_branched_event("commit", "feat", 0),
            ],
            90,
        ),
        make_repo(
            "repo-a",
            vec![
                make_branched_event("commit", "main", 120),
                make_branched_event("commit", "main", 0),
            ],
            120,
        ),
    ];
    let sessions = deep_work_sessions(&repos, 60);
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].repo_name, "repo-a"); // 120min first
    assert_eq!(sessions[1].repo_name, "repo-b"); // 90min second
}

// --- US-021: retro_summary ---

fn make_empty_summary() -> ActivitySummary {
    ActivitySummary {
        period_label: "Sprint".to_string(),
        total_commits: 0,
        total_reviews: 0,
        total_repos: 0,
        total_estimated_time: Duration::zero(),
        total_ai_session_time: Duration::zero(),
        repos: vec![],
    }
}

fn make_summary_with_repos(repos: Vec<RepoSummary>) -> ActivitySummary {
    let total_commits: usize = repos.iter().map(|r| r.commits).sum();
    let total_reviews: usize = repos.iter().map(|r| r.reviews.len()).sum();
    let total_time: Duration = repos
        .iter()
        .map(|r| r.estimated_time)
        .fold(Duration::zero(), |a, b| a + b);
    let total_ai_time: Duration = repos
        .iter()
        .flat_map(|r| &r.ai_sessions)
        .map(|s| s.duration)
        .fold(Duration::zero(), |a, b| a + b);
    ActivitySummary {
        period_label: "Sprint".to_string(),
        total_commits,
        total_reviews,
        total_repos: repos.len(),
        total_estimated_time: total_time,
        total_ai_session_time: total_ai_time,
        repos,
    }
}

#[test]
fn retro_summary_empty_returns_zeros() {
    let summary = make_empty_summary();
    let retro = retro_summary(&summary, 8, 18, 60);
    assert_eq!(retro.total_commits, 0);
    assert_eq!(retro.total_reviews, 0);
    assert_eq!(retro.total_estimated_minutes, 0);
    assert_eq!(retro.total_ai_session_minutes, 0);
    assert_eq!(retro.active_repos, 0);
    assert_eq!(retro.deep_work_session_count, 0);
    assert_eq!(retro.total_deep_work_minutes, 0);
    assert!((retro.after_hours_pct - 0.0).abs() < f64::EPSILON);
    assert!(retro.busiest_day.is_none());
    assert!(retro.peak_hour.is_none());
    assert!((retro.focus_score - 1.0).abs() < f64::EPSILON);
}

#[test]
fn retro_summary_populated_computes_aggregates() {
    // 3 commits on Mon Mar 10, 2 on Tue Mar 11 at different hours
    let events = vec![
        make_event_at("commit", 2025, 3, 10, 9),
        make_event_at("commit", 2025, 3, 10, 10),
        make_event_at("commit", 2025, 3, 10, 10), // 2 at hour 10
        make_event_at("commit", 2025, 3, 11, 14),
        make_event_at("commit", 2025, 3, 11, 22), // after hours
    ];
    let repos = vec![make_repo("repo-a", events, 120)];
    let summary = make_summary_with_repos(repos);
    let retro = retro_summary(&summary, 8, 18, 60);

    assert_eq!(retro.total_commits, 5);
    assert_eq!(retro.active_repos, 1);
    assert_eq!(retro.total_estimated_minutes, 120);

    // Busiest day: Mar 10 with 3 commits
    let mar10 = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
    assert_eq!(retro.busiest_day, Some(mar10));
    assert_eq!(retro.busiest_day_commits, 3);

    // Peak hour: 10 with 2 commits
    assert_eq!(retro.peak_hour, Some(10));
    assert_eq!(retro.peak_hour_commits, 2);

    // After hours: 1 of 5 commits at 22:00 = 20%
    assert!((retro.after_hours_pct - 20.0).abs() < 1.0);
}

#[test]
fn retro_summary_serializes_to_json() {
    let summary = make_empty_summary();
    let retro = retro_summary(&summary, 8, 18, 60);
    let json = serde_json::to_string(&retro).unwrap();
    assert!(json.contains("total_commits"));
    assert!(json.contains("focus_score"));
    assert!(json.contains("busiest_day"));
}

// --- US-023: dora_lite_metrics ---

#[test]
fn dora_lite_metrics_empty_returns_zeros() {
    let summary = make_empty_summary();
    let today = Local::now().date_naive();
    let m = dora_lite_metrics(&summary, 7, today - chrono::Duration::days(7), today);
    assert!((m.commits_per_day - 0.0).abs() < f64::EPSILON);
    assert!((m.prs_merged_per_week - 0.0).abs() < f64::EPSILON);
    assert!((m.velocity_trend - 0.0).abs() < f64::EPSILON);
}

#[test]
fn dora_lite_metrics_commits_per_day() {
    // 10 commits over 5 days = 2.0 commits/day
    let events: Vec<ActivityEvent> = (0..10).map(|i| make_event("commit", i * 10)).collect();
    let repos = vec![make_repo("repo-a", events, 60)];
    let summary = make_summary_with_repos(repos);
    let today = Local::now().date_naive();
    let m = dora_lite_metrics(&summary, 5, today - chrono::Duration::days(5), today);
    assert!((m.commits_per_day - 2.0).abs() < f64::EPSILON);
}

#[test]
fn dora_lite_metrics_prs_merged_per_week() {
    use blackbox::enrichment::PrInfo;
    // 3 merged PRs + 1 open over 14 days (2 weeks) = 1.5 merged/week
    let mut repo = make_repo("repo-a", vec![make_event("commit", 10)], 60);
    repo.pr_info = Some(vec![
        PrInfo {
            number: 1,
            title: "a".into(),
            state: "MERGED".into(),
            head_ref_name: "main".into(),
        },
        PrInfo {
            number: 2,
            title: "b".into(),
            state: "MERGED".into(),
            head_ref_name: "main".into(),
        },
        PrInfo {
            number: 3,
            title: "c".into(),
            state: "MERGED".into(),
            head_ref_name: "main".into(),
        },
        PrInfo {
            number: 4,
            title: "d".into(),
            state: "OPEN".into(),
            head_ref_name: "main".into(),
        },
    ]);
    let summary = make_summary_with_repos(vec![repo]);
    let today = Local::now().date_naive();
    let m = dora_lite_metrics(&summary, 14, today - chrono::Duration::days(14), today);
    assert!((m.prs_merged_per_week - 1.5).abs() < f64::EPSILON);
}

#[test]
fn dora_lite_metrics_velocity_trend_positive() {
    // 10-day period. 2 commits in first half (days -10..-5), 6 in second half (days -5..0)
    // trend = (6-2)/2 = 2.0
    let today = Local::now().date_naive();
    let first_half_events: Vec<ActivityEvent> = (0..2)
        .map(|i| {
            let d = today - chrono::Duration::days(8 - i);
            let local_dt = Local
                .with_ymd_and_hms(d.year(), d.month(), d.day(), 10, 0, 0)
                .unwrap();
            ActivityEvent {
                event_type: "commit".to_string(),
                branch: None,
                commit_hash: None,
                message: None,
                timestamp: local_dt.with_timezone(&Utc),
            }
        })
        .collect();
    let second_half_events: Vec<ActivityEvent> = (0..6)
        .map(|i| {
            let d = today - chrono::Duration::days(3 - i % 3);
            let local_dt = Local
                .with_ymd_and_hms(d.year(), d.month(), d.day(), 10 + i as u32, 0, 0)
                .unwrap();
            ActivityEvent {
                event_type: "commit".to_string(),
                branch: None,
                commit_hash: None,
                message: None,
                timestamp: local_dt.with_timezone(&Utc),
            }
        })
        .collect();
    let mut all_events = first_half_events;
    all_events.extend(second_half_events);
    let repos = vec![make_repo("repo-a", all_events, 120)];
    let summary = make_summary_with_repos(repos);
    let m = dora_lite_metrics(&summary, 10, today - chrono::Duration::days(10), today);
    assert!(
        m.velocity_trend > 0.0,
        "trend should be positive, got {}",
        m.velocity_trend
    );
}

#[test]
fn dora_lite_metrics_velocity_trend_zero_when_first_half_empty() {
    // All commits in second half, none in first → trend = 0.0
    let events: Vec<ActivityEvent> = (0..5).map(|i| make_event("commit", i * 10)).collect();
    let repos = vec![make_repo("repo-a", events, 60)];
    let summary = make_summary_with_repos(repos);
    // 30-day period, all events are "now" (minutes ago) → all in second half
    let today = Local::now().date_naive();
    let m = dora_lite_metrics(&summary, 30, today - chrono::Duration::days(30), today);
    assert!(
        (m.velocity_trend - 0.0).abs() < f64::EPSILON,
        "trend should be 0.0 when first half empty, got {}",
        m.velocity_trend
    );
}
