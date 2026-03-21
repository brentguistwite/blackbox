use blackbox::query::{ActivityEvent, RepoSummary};
use blackbox::insights::{context_switches, daily_commit_counts, active_dates, hourly_distribution, weekly_rhythm, work_hours_analysis, streak_info, ContextSwitchMetrics};
use chrono::{Duration, Local, NaiveDate, TimeZone, Timelike, Utc, Datelike};

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

#[test]
fn daily_commit_counts_empty_returns_empty() {
    let result = daily_commit_counts(&[]);
    assert!(result.is_empty());
}

/// Create event at a specific local date+hour (for deterministic date-bucketing tests).
fn make_event_at(event_type: &str, year: i32, month: u32, day: u32, hour: u32) -> ActivityEvent {
    let local_dt = Local.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap();
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
        make_repo("repo-a", vec![
            make_event_at("commit", 2025, 3, 10, 9),
            make_event_at("commit", 2025, 3, 12, 14),
        ], 30),
        make_repo("repo-b", vec![
            make_event_at("commit", 2025, 3, 10, 15),
            make_event_at("commit", 2025, 3, 11, 10),
        ], 30),
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
        make_event_at("commit", 2025, 3, 10, 8),  // Mon 8am — before work
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
        make_event_at("commit", 2025, 3, 8, 10),  // Sat
        make_event_at("commit", 2025, 3, 8, 14),  // Sat (same day)
        make_event_at("commit", 2025, 3, 8, 18),  // Sat (same day)
        make_event_at("commit", 2025, 3, 9, 11),  // Sun
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
        make_event_at("commit", 2025, 3, 10, 9),  // Mon
        make_event_at("commit", 2025, 3, 11, 9),  // Tue
        make_event_at("commit", 2025, 3, 12, 9),  // Wed
        make_event_at("commit", 2025, 3, 13, 9),  // Thu
        make_event_at("commit", 2025, 3, 14, 9),  // Fri
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
        make_event_at("commit", 2025, 3, 10, 9),  // Mon
        make_event_at("commit", 2025, 3, 11, 9),  // Tue
        // Wed Mar 12 missing
        make_event_at("commit", 2025, 3, 13, 9),  // Thu
        make_event_at("commit", 2025, 3, 14, 9),  // Fri
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
        make_event_at("commit", 2025, 3, 3, 9),   // Mon W1
        make_event_at("commit", 2025, 3, 4, 9),   // Tue
        make_event_at("commit", 2025, 3, 5, 9),   // Wed
        make_event_at("commit", 2025, 3, 6, 9),   // Thu
        make_event_at("commit", 2025, 3, 7, 9),   // Fri
        // Sat 8, Sun 9 = rest
        // Mon 10 = missing (breaks)
        make_event_at("commit", 2025, 3, 11, 9),  // Tue W2
        make_event_at("commit", 2025, 3, 12, 9),  // Wed
        make_event_at("commit", 2025, 3, 13, 9),  // Thu
        make_event_at("commit", 2025, 3, 14, 9),  // Fri
    ];
    let repos = vec![make_repo("repo-a", events, 120)];
    let friday = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
    let result = streak_info(&repos, &[5, 6], friday);
    assert_eq!(result.current_streak, 4);
    assert_eq!(result.longest_streak, 5);
    assert_eq!(result.longest_streak_start, Some(NaiveDate::from_ymd_opt(2025, 3, 3).unwrap()));
}

#[test]
fn streak_active_days_30d_counts_recent() {
    let today = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
    let events = vec![
        make_event_at("commit", 2025, 2, 1, 9),   // 41 days ago — outside
        make_event_at("commit", 2025, 3, 1, 9),   // 13 days ago — inside
        make_event_at("commit", 2025, 3, 10, 9),  // 4 days ago — inside
        make_event_at("commit", 2025, 3, 14, 9),  // today — inside
    ];
    let repos = vec![make_repo("repo-a", events, 60)];
    let result = streak_info(&repos, &[5, 6], today);
    assert_eq!(result.active_days_30d, 3);
}
