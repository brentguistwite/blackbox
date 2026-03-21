use crate::query::RepoSummary;
use chrono::{Datelike, NaiveDate, NaiveTime, Timelike, Weekday};
use regex::Regex;
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

#[derive(Debug, Clone)]
pub struct TicketSummary {
    pub ticket_id: String,
    pub branches: Vec<String>,
    pub repos: Vec<String>,
    pub commits: usize,
    pub estimated_minutes: i64,
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
