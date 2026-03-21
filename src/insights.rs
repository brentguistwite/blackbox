use crate::query::{ActivitySummary, RepoSummary};
use chrono::{Datelike, NaiveDate, NaiveTime, Timelike, Weekday};
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone)]
pub struct ContextSwitchMetrics {
    pub branch_switches: usize,
    pub repo_switches: usize,
    pub focus_score: f64,
}

/// Count context switches across repos. Pure function — no DB access.
///
/// - branch_switches: count of "branch_switch" events across all repos
/// - repo_switches: chronological transitions between different repos
/// - focus_score: 1.0 / (1.0 + switches_per_hour)
pub fn context_switches(repos: &[RepoSummary]) -> ContextSwitchMetrics {
    if repos.is_empty() {
        return ContextSwitchMetrics {
            branch_switches: 0,
            repo_switches: 0,
            focus_score: 1.0,
        };
    }

    let branch_switches: usize = repos
        .iter()
        .flat_map(|r| &r.events)
        .filter(|e| e.event_type == "branch_switch")
        .count();

    // Flatten all events tagged with repo path, sort by timestamp
    let mut tagged: Vec<(&str, &crate::query::ActivityEvent)> = repos
        .iter()
        .flat_map(|r| r.events.iter().map(move |e| (r.repo_path.as_str(), e)))
        .collect();
    tagged.sort_by_key(|(_, e)| e.timestamp);

    let repo_switches = tagged
        .windows(2)
        .filter(|w| w[0].0 != w[1].0)
        .count();

    let total_minutes: f64 = repos.iter().map(|r| r.estimated_time.num_minutes() as f64).sum();
    let total_hours = total_minutes / 60.0;
    let total_switches = (branch_switches + repo_switches) as f64;
    let switches_per_hour = if total_hours > 0.0 { total_switches / total_hours } else { 0.0 };
    let focus_score = 1.0 / (1.0 + switches_per_hour);

    ContextSwitchMetrics {
        branch_switches,
        repo_switches,
        focus_score,
    }
}

/// Count commits per local hour of day (0-23). Only commit events counted.
pub fn hourly_distribution(repos: &[RepoSummary]) -> [usize; 24] {
    let mut buckets = [0usize; 24];
    for repo in repos {
        for event in &repo.events {
            if event.event_type == "commit" {
                let hour = event.timestamp.with_timezone(&chrono::Local).hour() as usize;
                buckets[hour] += 1;
            }
        }
    }
    buckets
}

/// Count commits per day of week (0=Mon, 6=Sun). Only commit events counted.
pub fn weekly_rhythm(repos: &[RepoSummary]) -> [usize; 7] {
    let mut buckets = [0usize; 7];
    for repo in repos {
        for event in &repo.events {
            if event.event_type == "commit" {
                let weekday = event.timestamp.with_timezone(&chrono::Local)
                    .weekday()
                    .num_days_from_monday() as usize;
                buckets[weekday] += 1;
            }
        }
    }
    buckets
}

/// Count commits per local calendar date across all repos.
/// Converts UTC timestamps to Local timezone before extracting date.
pub fn daily_commit_counts(repos: &[RepoSummary]) -> BTreeMap<NaiveDate, usize> {
    let mut counts = BTreeMap::new();
    for repo in repos {
        for event in &repo.events {
            if event.event_type == "commit" {
                let local_date = event.timestamp.with_timezone(&chrono::Local).date_naive();
                *counts.entry(local_date).or_insert(0) += 1;
            }
        }
    }
    counts
}

/// Return sorted unique local dates that have at least one commit.
pub fn active_dates(repos: &[RepoSummary]) -> Vec<NaiveDate> {
    daily_commit_counts(repos).into_keys().collect()
}

#[derive(Debug, Clone)]
pub struct WorkHoursAnalysis {
    pub total_commits: usize,
    pub after_hours_commits: usize,
    pub after_hours_pct: f64,
    pub earliest_commit: Option<NaiveTime>,
    pub latest_commit: Option<NaiveTime>,
    pub weekend_days_active: usize,
}

/// Analyze commit activity relative to configured work hours.
/// `work_start`/`work_end` are local hours (0..=23). Commits outside [start, end) are "after hours".
pub fn work_hours_analysis(repos: &[RepoSummary], work_start: u8, work_end: u8) -> WorkHoursAnalysis {
    let mut total = 0usize;
    let mut after_hours = 0usize;
    let mut earliest: Option<NaiveTime> = None;
    let mut latest: Option<NaiveTime> = None;
    let mut weekend_dates: HashSet<NaiveDate> = HashSet::new();

    for repo in repos {
        for event in &repo.events {
            if event.event_type != "commit" {
                continue;
            }
            let local = event.timestamp.with_timezone(&chrono::Local);
            let hour = local.hour() as u8;
            let time = local.time();
            let date = local.date_naive();

            total += 1;

            if hour < work_start || hour >= work_end {
                after_hours += 1;
            }

            earliest = Some(match earliest {
                Some(e) if e <= time => e,
                _ => time,
            });
            latest = Some(match latest {
                Some(l) if l >= time => l,
                _ => time,
            });

            let weekday = local.weekday();
            if weekday == Weekday::Sat || weekday == Weekday::Sun {
                weekend_dates.insert(date);
            }
        }
    }

    let pct = if total > 0 {
        (after_hours as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    WorkHoursAnalysis {
        total_commits: total,
        after_hours_commits: after_hours,
        after_hours_pct: pct,
        earliest_commit: earliest,
        latest_commit: latest,
        weekend_days_active: weekend_dates.len(),
    }
}

#[derive(Debug, Clone)]
pub struct StreakInfo {
    pub current_streak: usize,
    pub longest_streak: usize,
    pub longest_streak_start: Option<NaiveDate>,
    pub active_days_30d: usize,
}

/// Compute coding streak info with rest-day awareness.
/// `rest_days` uses chrono weekday numbering: 0=Mon, 5=Sat, 6=Sun.
/// Rest days are skipped (don't break streak, don't count toward it).
pub fn streak_info(repos: &[RepoSummary], rest_days: &[u8], as_of: NaiveDate) -> StreakInfo {
    let dates = active_dates(repos);
    if dates.is_empty() {
        return StreakInfo {
            current_streak: 0,
            longest_streak: 0,
            longest_streak_start: None,
            active_days_30d: 0,
        };
    }

    let date_set: HashSet<NaiveDate> = dates.iter().copied().collect();
    let is_rest = |d: NaiveDate| rest_days.contains(&(d.weekday().num_days_from_monday() as u8));

    // Current streak: walk backward from as_of
    let mut current_streak = 0;
    let mut d = as_of;
    loop {
        if is_rest(d) {
            d = d.pred_opt().unwrap();
            continue;
        }
        if date_set.contains(&d) {
            current_streak += 1;
            d = d.pred_opt().unwrap();
        } else {
            break;
        }
    }

    // Longest streak: walk forward from first to last active date
    let first = *dates.first().unwrap();
    let last = *dates.last().unwrap();
    let mut longest = 0;
    let mut longest_start = None;
    let mut run = 0usize;
    let mut run_start = None;
    d = first;
    while d <= last {
        if is_rest(d) {
            d = d.succ_opt().unwrap();
            continue;
        }
        if date_set.contains(&d) {
            if run == 0 {
                run_start = Some(d);
            }
            run += 1;
        } else {
            if run > longest {
                longest = run;
                longest_start = run_start;
            }
            run = 0;
            run_start = None;
        }
        d = d.succ_opt().unwrap();
    }
    if run > longest {
        longest = run;
        longest_start = run_start;
    }

    // Active days in last 30 days
    let cutoff = as_of - chrono::Duration::days(30);
    let active_days_30d = dates.iter().filter(|&&d| d > cutoff && d <= as_of).count();

    StreakInfo {
        current_streak,
        longest_streak: longest,
        longest_streak_start: longest_start,
        active_days_30d,
    }
}

/// Compute estimated minutes per local calendar date across all repos.
/// Distributes each repo's estimated_time proportionally by commit count per day.
pub fn daily_estimated_minutes(repos: &[RepoSummary]) -> BTreeMap<NaiveDate, i64> {
    let mut result: BTreeMap<NaiveDate, i64> = BTreeMap::new();

    for repo in repos {
        if repo.commits == 0 {
            continue;
        }
        // Count commits per day for this repo
        let mut day_counts: BTreeMap<NaiveDate, usize> = BTreeMap::new();
        for event in &repo.events {
            if event.event_type == "commit" {
                let date = event.timestamp.with_timezone(&chrono::Local).date_naive();
                *day_counts.entry(date).or_insert(0) += 1;
            }
        }
        let total_mins = repo.estimated_time.num_minutes();
        let total_commits = repo.commits as i64;
        for (date, count) in &day_counts {
            let mins = (total_mins * *count as i64) / total_commits;
            *result.entry(*date).or_insert(0) += mins;
        }
    }

    result
}

#[derive(Debug, Clone)]
pub struct TicketSummary {
    pub ticket_id: String,
    pub branches: Vec<String>,
    pub repos: Vec<String>,
    pub commits: usize,
    pub estimated_minutes: i64,
}

#[derive(Debug, Clone)]
pub struct DeepWorkSession {
    pub repo_name: String,
    pub branch: String,
    pub duration_minutes: i64,
    pub commit_count: usize,
}

/// Detect deep work sessions per repo. A "run" is a sequence of events on the same branch.
/// branch_switch or different-branch commit ends the run. Runs >= threshold with >= 1 commit qualify.
/// Open-ended runs end at last event timestamp. Results sorted by duration descending.
pub fn deep_work_sessions(repos: &[RepoSummary], threshold_minutes: i64) -> Vec<DeepWorkSession> {
    let mut sessions = Vec::new();

    for repo in repos {
        let mut events: Vec<&crate::query::ActivityEvent> = repo.events.iter().collect();
        events.sort_by_key(|e| e.timestamp);

        if events.is_empty() {
            continue;
        }

        let mut run_branch: Option<&str> = None;
        let mut run_start = events[0].timestamp;
        let mut run_commits = 0usize;

        for event in &events {
            let event_branch = event.branch.as_deref().unwrap_or("");

            match run_branch {
                None => {
                    // Start new run
                    run_branch = Some(event.branch.as_deref().unwrap_or(""));
                    run_start = event.timestamp;
                    run_commits = if event.event_type == "commit" { 1 } else { 0 };
                }
                Some(current) => {
                    let breaks = event.event_type == "branch_switch" || event_branch != current;
                    if breaks {
                        // End current run, check if qualifies
                        let duration = (event.timestamp - run_start).num_minutes();
                        if duration >= threshold_minutes && run_commits >= 1 {
                            sessions.push(DeepWorkSession {
                                repo_name: repo.repo_name.clone(),
                                branch: current.to_string(),
                                duration_minutes: duration,
                                commit_count: run_commits,
                            });
                        }
                        // Start new run
                        run_branch = Some(event.branch.as_deref().unwrap_or(""));
                        run_start = event.timestamp;
                        run_commits = if event.event_type == "commit" { 1 } else { 0 };
                    } else {
                        if event.event_type == "commit" {
                            run_commits += 1;
                        }
                    }
                }
            }
        }

        // Close final run
        if let Some(current) = run_branch {
            let last_ts = events.last().unwrap().timestamp;
            let duration = (last_ts - run_start).num_minutes();
            if duration >= threshold_minutes && run_commits >= 1 {
                sessions.push(DeepWorkSession {
                    repo_name: repo.repo_name.clone(),
                    branch: current.to_string(),
                    duration_minutes: duration,
                    commit_count: run_commits,
                });
            }
        }
    }

    sessions.sort_by(|a, b| b.duration_minutes.cmp(&a.duration_minutes));
    sessions
}

/// Extract unique ticket IDs from branch names using configurable regex patterns.
pub fn extract_ticket_ids(branches: &[String], patterns: &[String]) -> Vec<String> {
    let regexes: Vec<Regex> = patterns
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect();

    let mut seen = HashSet::new();
    let mut ids = Vec::new();

    for branch in branches {
        for re in &regexes {
            for m in re.find_iter(branch) {
                let id = m.as_str().to_string();
                if seen.insert(id.clone()) {
                    ids.push(id);
                }
            }
        }
    }

    ids
}

#[derive(Debug, Clone, Serialize)]
pub struct RetroSummary {
    pub total_commits: usize,
    pub total_reviews: usize,
    pub total_estimated_minutes: i64,
    pub total_ai_session_minutes: i64,
    pub active_repos: usize,
    pub branch_switches: usize,
    pub repo_switches: usize,
    pub focus_score: f64,
    pub deep_work_session_count: usize,
    pub total_deep_work_minutes: i64,
    pub after_hours_pct: f64,
    pub weekend_days_active: usize,
    pub busiest_day: Option<NaiveDate>,
    pub busiest_day_commits: usize,
    pub peak_hour: Option<usize>,
    pub peak_hour_commits: usize,
}

/// Compose all insight functions into a sprint retrospective summary.
/// Pure function — no DB access. All data comes from pre-fetched ActivitySummary.
pub fn retro_summary(
    summary: &ActivitySummary,
    work_start: u8,
    work_end: u8,
    deep_work_threshold: i64,
) -> RetroSummary {
    let ctx = context_switches(&summary.repos);
    let sessions = deep_work_sessions(&summary.repos, deep_work_threshold);
    let work_hours = work_hours_analysis(&summary.repos, work_start, work_end);
    let hourly = hourly_distribution(&summary.repos);
    let daily = daily_commit_counts(&summary.repos);

    let total_deep_work_minutes: i64 = sessions.iter().map(|s| s.duration_minutes).sum();

    let (busiest_day, busiest_day_commits) = daily
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(date, count)| (Some(*date), *count))
        .unwrap_or((None, 0));

    let (peak_hour, peak_hour_commits) = hourly
        .iter()
        .enumerate()
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| **count > 0)
        .map(|(hour, count)| (Some(hour), *count))
        .unwrap_or((None, 0));

    RetroSummary {
        total_commits: summary.total_commits,
        total_reviews: summary.total_reviews,
        total_estimated_minutes: summary.total_estimated_time.num_minutes(),
        total_ai_session_minutes: summary.total_ai_session_time.num_minutes(),
        active_repos: summary.total_repos,
        branch_switches: ctx.branch_switches,
        repo_switches: ctx.repo_switches,
        focus_score: ctx.focus_score,
        deep_work_session_count: sessions.len(),
        total_deep_work_minutes,
        after_hours_pct: work_hours.after_hours_pct,
        weekend_days_active: work_hours.weekend_days_active,
        busiest_day,
        busiest_day_commits,
        peak_hour,
        peak_hour_commits,
    }
}

#[derive(Debug, Clone)]
pub struct DoraLiteMetrics {
    pub commits_per_day: f64,
    pub prs_merged_per_week: f64,
    pub velocity_trend: f64,
}

/// Compute DORA-lite metrics from activity summary.
/// `period_days` is the number of days in the analysis window.
/// velocity_trend: compare commit counts in first half vs second half of period.
/// Positive = accelerating, negative = decelerating, 0.0 = flat or no first-half data.
pub fn dora_lite_metrics(summary: &ActivitySummary, period_days: i64) -> DoraLiteMetrics {
    if period_days <= 0 || summary.repos.is_empty() {
        return DoraLiteMetrics {
            commits_per_day: 0.0,
            prs_merged_per_week: 0.0,
            velocity_trend: 0.0,
        };
    }

    let total_commits = summary.total_commits;
    let commits_per_day = total_commits as f64 / period_days as f64;

    // Count merged PRs across all repos
    let merged_prs: usize = summary.repos.iter()
        .filter_map(|r| r.pr_info.as_ref())
        .flat_map(|prs| prs.iter())
        .filter(|pr| pr.state == "MERGED")
        .count();
    let weeks = period_days as f64 / 7.0;
    let prs_merged_per_week = if weeks > 0.0 { merged_prs as f64 / weeks } else { 0.0 };

    // Velocity trend: first half vs second half commit counts via daily_commit_counts
    let daily = daily_commit_counts(&summary.repos);
    let now = chrono::Local::now().date_naive();
    let period_start = now - chrono::Duration::days(period_days);
    let midpoint = now - chrono::Duration::days(period_days / 2);

    let first_half: usize = daily.iter()
        .filter(|(d, _)| **d >= period_start && **d < midpoint)
        .map(|(_, c)| *c)
        .sum();
    let second_half: usize = daily.iter()
        .filter(|(d, _)| **d >= midpoint && **d <= now)
        .map(|(_, c)| *c)
        .sum();

    let velocity_trend = if first_half == 0 {
        0.0
    } else {
        (second_half as f64 - first_half as f64) / first_half as f64
    };

    DoraLiteMetrics {
        commits_per_day,
        prs_merged_per_week,
        velocity_trend,
    }
}

/// Aggregate estimated time per ticket across repos.
/// If a repo matches multiple tickets, time and commits are split equally.
/// Results sorted by estimated_minutes descending.
pub fn aggregate_time_per_ticket(repos: &[RepoSummary], patterns: &[String]) -> Vec<TicketSummary> {
    let mut ticket_map: BTreeMap<String, TicketSummary> = BTreeMap::new();

    for repo in repos {
        let ticket_ids = extract_ticket_ids(&repo.branches, patterns);
        if ticket_ids.is_empty() {
            continue;
        }

        let n = ticket_ids.len() as i64;
        let split_minutes = repo.estimated_time.num_minutes() / n;
        let split_commits = repo.commits / ticket_ids.len();

        for tid in &ticket_ids {
            let entry = ticket_map.entry(tid.clone()).or_insert_with(|| TicketSummary {
                ticket_id: tid.clone(),
                branches: vec![],
                repos: vec![],
                commits: 0,
                estimated_minutes: 0,
            });
            entry.estimated_minutes += split_minutes;
            entry.commits += split_commits;
            if !entry.repos.contains(&repo.repo_name) {
                entry.repos.push(repo.repo_name.clone());
            }
            for b in &repo.branches {
                if !entry.branches.contains(b) {
                    entry.branches.push(b.clone());
                }
            }
        }
    }

    let mut results: Vec<TicketSummary> = ticket_map.into_values().collect();
    results.sort_by(|a, b| b.estimated_minutes.cmp(&a.estimated_minutes));
    results
}
