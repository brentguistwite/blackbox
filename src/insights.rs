use crate::query::RepoSummary;
use chrono::{Datelike, NaiveDate, Timelike};
use std::collections::BTreeMap;

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
