use crate::enrichment::PrInfo;
use chrono::{Datelike, DateTime, Duration, Local, TimeZone, Utc};
use rusqlite::Connection;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct ActivityEvent {
    pub event_type: String,
    pub branch: Option<String>,
    pub commit_hash: Option<String>,
    pub message: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RepoSummary {
    pub repo_path: String,
    pub repo_name: String,
    pub commits: usize,
    pub branches: Vec<String>,
    pub estimated_time: Duration,
    pub events: Vec<ActivityEvent>,
    pub pr_info: Option<Vec<PrInfo>>,
}

#[derive(Debug, Clone)]
pub struct ActivitySummary {
    pub period_label: String,
    pub total_commits: usize,
    pub total_repos: usize,
    pub total_estimated_time: Duration,
    pub repos: Vec<RepoSummary>,
}

/// Session-gap time estimation algorithm.
/// Events must be sorted by timestamp ascending.
/// Each session starts with first_commit_minutes credit.
/// Gaps between events within session_gap_minutes are added.
/// Gaps >= session_gap_minutes start a new session.
pub fn estimate_time(
    events: &[ActivityEvent],
    session_gap_minutes: u64,
    first_commit_minutes: u64,
) -> Duration {
    if events.is_empty() {
        return Duration::zero();
    }

    let gap_threshold = Duration::minutes(session_gap_minutes as i64);
    let first_credit = Duration::minutes(first_commit_minutes as i64);
    let mut total = first_credit; // first event gets credit

    for i in 1..events.len() {
        let gap = events[i].timestamp - events[i - 1].timestamp;
        if gap >= gap_threshold {
            // New session
            total = total + first_credit;
        } else {
            total = total + gap;
        }
    }

    total
}

/// Returns (start_of_today_local_as_utc, now_utc)
pub fn today_range() -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    let local_today = Local::now().date_naive();
    let start_local = local_today
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let start_utc = Local
        .from_local_datetime(&start_local)
        .unwrap()
        .with_timezone(&Utc);
    (start_utc, now)
}

/// Returns (monday_midnight_local_as_utc, now_utc)
pub fn week_range() -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    let local_today = Local::now().date_naive();
    let weekday = local_today.weekday().num_days_from_monday();
    let monday = local_today - Duration::days(weekday as i64);
    let start_local = monday.and_hms_opt(0, 0, 0).unwrap();
    let start_utc = Local
        .from_local_datetime(&start_local)
        .unwrap()
        .with_timezone(&Utc);
    (start_utc, now)
}

/// Returns (1st_of_month_midnight_local_as_utc, now_utc)
pub fn month_range() -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    let local_today = Local::now().date_naive();
    let first_of_month = local_today
        .with_day(1)
        .unwrap();
    let start_local = first_of_month.and_hms_opt(0, 0, 0).unwrap();
    let start_utc = Local
        .from_local_datetime(&start_local)
        .unwrap()
        .with_timezone(&Utc);
    (start_utc, now)
}

/// Query activity from DB, grouped by repo, with time estimates per repo.
pub fn query_activity(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    session_gap_minutes: u64,
    first_commit_minutes: u64,
) -> anyhow::Result<Vec<RepoSummary>> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();

    let mut stmt = conn.prepare(
        "SELECT repo_path, event_type, branch, commit_hash, message, timestamp
         FROM git_activity
         WHERE timestamp >= ?1 AND timestamp <= ?2
         ORDER BY repo_path, timestamp ASC",
    )?;

    let rows = stmt.query_map(rusqlite::params![from_str, to_str], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;

    let mut repo_map: BTreeMap<String, Vec<ActivityEvent>> = BTreeMap::new();

    for row in rows {
        let (repo_path, event_type, branch, commit_hash, message, timestamp_str) = row?;
        let timestamp = DateTime::parse_from_rfc3339(&timestamp_str)?.with_timezone(&Utc);
        repo_map
            .entry(repo_path)
            .or_default()
            .push(ActivityEvent {
                event_type,
                branch,
                commit_hash,
                message,
                timestamp,
            });
    }

    let mut repos: Vec<RepoSummary> = Vec::new();
    for (repo_path, events) in repo_map {
        let repo_name = std::path::Path::new(&repo_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| repo_path.clone());

        let commits = events
            .iter()
            .filter(|e| e.event_type == "commit")
            .count();

        let mut branches: Vec<String> = events
            .iter()
            .filter_map(|e| e.branch.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        branches.sort();

        let estimated_time = estimate_time(&events, session_gap_minutes, first_commit_minutes);

        repos.push(RepoSummary {
            repo_path,
            repo_name,
            commits,
            branches,
            estimated_time,
            events,
            pr_info: None,
        });
    }

    Ok(repos)
}
