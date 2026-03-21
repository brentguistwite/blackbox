use crate::enrichment::PrInfo;
use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Utc};
use clap::ValueEnum;
use rusqlite::Connection;
use std::collections::BTreeMap;

const MIN_GAP_FLOOR_MINS: i64 = 30;
const MAX_GAP_CAP_MINS: i64 = 120;
const MIN_CREDIT_MINS: i64 = 5;
const MAX_CREDIT_MINS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeInterval {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// Sort intervals by start, merge overlapping/adjacent, return merged list + total duration.
pub fn merge_intervals(intervals: &mut [TimeInterval]) -> (Vec<TimeInterval>, Duration) {
    if intervals.is_empty() {
        return (vec![], Duration::zero());
    }
    intervals.sort_by_key(|i| i.start);
    let mut merged = vec![intervals[0]];
    for iv in &intervals[1..] {
        let last = merged.last_mut().unwrap();
        if iv.start <= last.end {
            last.end = last.end.max(iv.end);
        } else {
            merged.push(*iv);
        }
    }
    let total = merged
        .iter()
        .fold(Duration::zero(), |acc, iv| acc + (iv.end - iv.start));
    (merged, total)
}

/// Compute median gap between consecutive commit events (ignoring non-commit events).
/// Returns None if < 2 commits.
pub fn median_commit_gap(events: &[ActivityEvent]) -> Option<Duration> {
    let commit_times: Vec<DateTime<Utc>> = events
        .iter()
        .filter(|e| e.event_type == "commit")
        .map(|e| e.timestamp)
        .collect();
    if commit_times.len() < 2 {
        return None;
    }
    let mut gaps: Vec<Duration> = commit_times
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| d.num_seconds() > 0) // ignore zero-gap duplicates
        .collect();
    if gaps.is_empty() {
        return None;
    }
    gaps.sort();
    let mid = gaps.len() / 2;
    if gaps.len().is_multiple_of(2) {
        Some((gaps[mid - 1] + gaps[mid]) / 2)
    } else {
        Some(gaps[mid])
    }
}

/// Time estimation v2: git events + AI sessions, adaptive thresholds.
/// No presence data. AI session intervals must be pre-clipped to query window.
///
/// With empty ai_sessions and < 2 commits, falls back to config gap/credit values
/// (matching legacy behavior).
pub fn estimate_time_v2(
    events: &[ActivityEvent],
    ai_sessions: &[TimeInterval],
    session_gap_minutes: u64,
    first_commit_minutes: u64,
) -> (Duration, Vec<TimeInterval>) {
    // Step 1: Compute adaptive thresholds from commit patterns
    let (effective_gap, effective_credit) = match median_commit_gap(events) {
        Some(median) => {
            let median_mins = median.num_minutes();
            let gap = (median_mins * 3).clamp(MIN_GAP_FLOOR_MINS, MAX_GAP_CAP_MINS);
            let credit = median_mins.clamp(MIN_CREDIT_MINS, MAX_CREDIT_MINS);
            (Duration::minutes(gap), Duration::minutes(credit))
        }
        None => (
            Duration::minutes(session_gap_minutes as i64),
            Duration::minutes(first_commit_minutes as i64),
        ),
    };

    // Step 2: Group git events into sessions → tentative intervals with credit
    let mut git_intervals: Vec<TimeInterval> = Vec::new();
    if !events.is_empty() {
        let mut session_start_idx = 0;
        for i in 1..=events.len() {
            let is_end = i == events.len()
                || (events[i].timestamp - events[i - 1].timestamp) >= effective_gap;
            if is_end {
                let first_event = events[session_start_idx].timestamp;
                let last_event = events[i - 1].timestamp;
                let tentative_start = first_event - effective_credit;
                git_intervals.push(TimeInterval {
                    start: tentative_start,
                    end: last_event,
                });
                if i < events.len() {
                    session_start_idx = i;
                }
            }
        }
    }

    // Step 3: Credit suppression — if AI session covers [first_event - credit, first_event],
    // shrink git interval start to first_event (real data > guess)
    for iv in &mut git_intervals {
        let credit_window_start = iv.start;
        let credit_window_end = iv.start + effective_credit;
        let has_ai_overlap = ai_sessions
            .iter()
            .any(|ai| ai.start < credit_window_end && ai.end > credit_window_start);
        if has_ai_overlap {
            iv.start = credit_window_end; // shrink to first_event
        }
    }

    // Step 4: Collect git + AI intervals
    let mut all_intervals: Vec<TimeInterval> = Vec::new();
    all_intervals.extend_from_slice(&git_intervals);
    all_intervals.extend_from_slice(ai_sessions);

    // Step 5: Merge and return total + intervals
    let (merged, total) = merge_intervals(&mut all_intervals);
    (total, merged)
}

/// Query directory_presence for a time range, grouped by repo_path.
/// NULL left_at capped at entered_at + session_gap_minutes.
/// Intervals clipped to [from, to].
pub fn query_presence(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    session_gap_minutes: u64,
) -> anyhow::Result<BTreeMap<String, Vec<TimeInterval>>> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();
    let gap = Duration::minutes(session_gap_minutes as i64);

    let mut stmt = conn.prepare(
        "SELECT repo_path, entered_at, left_at
         FROM directory_presence
         WHERE entered_at <= ?2 AND (left_at >= ?1 OR left_at IS NULL)
         ORDER BY repo_path, entered_at ASC",
    )?;

    let rows = stmt.query_map(rusqlite::params![from_str, to_str], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;

    let mut map: BTreeMap<String, Vec<TimeInterval>> = BTreeMap::new();
    for row in rows {
        let (repo_path, entered_str, left_str) = row?;
        let entered = DateTime::parse_from_rfc3339(&entered_str)?.with_timezone(&Utc);
        let effective_end = match left_str {
            Some(ref s) => DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc),
            None => entered + gap,
        };

        // Clip to [from, to]
        let start = entered.max(from);
        let end = effective_end.min(to);
        if start < end {
            map.entry(repo_path)
                .or_default()
                .push(TimeInterval { start, end });
        }
    }

    Ok(map)
}

#[derive(Debug, Clone)]
pub struct ActivityEvent {
    pub event_type: String,
    pub branch: Option<String>,
    pub commit_hash: Option<String>,
    pub message: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ReviewInfo {
    pub pr_number: i64,
    pub pr_title: String,
    pub action: String,
    pub reviewed_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AiSessionInfo {
    pub session_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration: Duration,
    pub turns: Option<i64>,
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
    pub reviews: Vec<ReviewInfo>,
    pub ai_sessions: Vec<AiSessionInfo>,
}

#[derive(Debug, Clone)]
pub struct ActivitySummary {
    pub period_label: String,
    pub total_commits: usize,
    pub total_reviews: usize,
    pub total_repos: usize,
    pub total_estimated_time: Duration,
    pub total_ai_session_time: Duration,
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
            total += first_credit;
        } else {
            total += gap;
        }
    }

    total
}

/// Shared time range for analytics commands.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum QueryRange {
    Today,
    Yesterday,
    Week,
    #[default]
    Month,
    All,
}

impl QueryRange {
    pub fn to_range(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        match self {
            QueryRange::Today => today_range(),
            QueryRange::Yesterday => yesterday_range(),
            QueryRange::Week => week_range(),
            QueryRange::Month => month_range(),
            QueryRange::All => {
                let epoch = Utc.timestamp_opt(0, 0).single().expect("epoch");
                (epoch, Utc::now())
            }
        }
    }
}

/// Returns (start_of_today_local_as_utc, now_utc)
pub fn today_range() -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    let local_today = Local::now().date_naive();
    let start_local = local_today.and_hms_opt(0, 0, 0).unwrap();
    let start_utc = Local
        .from_local_datetime(&start_local)
        .unwrap()
        .with_timezone(&Utc);
    (start_utc, now)
}

/// Returns (yesterday_midnight_local_as_utc, today_midnight_local_as_utc)
pub fn yesterday_range() -> (DateTime<Utc>, DateTime<Utc>) {
    let local_today = Local::now().date_naive();
    let yesterday = local_today - Duration::days(1);
    let start_local = yesterday.and_hms_opt(0, 0, 0).unwrap();
    let end_local = local_today.and_hms_opt(0, 0, 0).unwrap();
    let start_utc = Local
        .from_local_datetime(&start_local)
        .unwrap()
        .with_timezone(&Utc);
    let end_utc = Local
        .from_local_datetime(&end_local)
        .unwrap()
        .with_timezone(&Utc);
    (start_utc, end_utc)
}

/// Parse YYYY-MM-DD strings into UTC range. End is next-day midnight (exclusive).
pub fn custom_range(from: &str, to: &str) -> anyhow::Result<(DateTime<Utc>, DateTime<Utc>)> {
    use chrono::NaiveDate;
    let from_date = NaiveDate::parse_from_str(from, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("Invalid from date '{}': {}", from, e))?;
    let to_date = NaiveDate::parse_from_str(to, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("Invalid to date '{}': {}", to, e))?;

    if to_date < from_date {
        anyhow::bail!("'from' date ({}) must be before 'to' date ({})", from, to);
    }

    let start_local = from_date.and_hms_opt(0, 0, 0).unwrap();
    // Exclusive end: next day midnight
    let end_date = to_date + Duration::days(1);
    let end_local = end_date.and_hms_opt(0, 0, 0).unwrap();

    let start_utc = Local
        .from_local_datetime(&start_local)
        .unwrap()
        .with_timezone(&Utc);
    let end_utc = Local
        .from_local_datetime(&end_local)
        .unwrap()
        .with_timezone(&Utc);

    Ok((start_utc, end_utc))
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
    let first_of_month = local_today.with_day(1).unwrap();
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
        repo_map.entry(repo_path).or_default().push(ActivityEvent {
            event_type,
            branch,
            commit_hash,
            message,
            timestamp,
        });
    }

    // Query reviews in the same time range
    let review_map = query_reviews(conn, from, to)?;

    // Query AI sessions in the same time range
    let ai_session_map = query_ai_sessions(conn, from, to)?;

    // Collect all repo paths. Filter out AI sessions from user's home dir.
    let home_dir = etcetera::home_dir()
        .ok()
        .map(|h| h.to_string_lossy().to_string());
    let not_home = |k: &&String| home_dir.as_ref().is_none_or(|h| k.as_str() != h);
    let all_repo_paths: std::collections::BTreeSet<String> = repo_map
        .keys()
        .chain(review_map.keys())
        .chain(ai_session_map.keys().filter(not_home))
        .cloned()
        .collect();

    let now = Utc::now();
    let mut repos: Vec<RepoSummary> = Vec::new();
    for repo_path in all_repo_paths {
        let events = repo_map.remove(&repo_path).unwrap_or_default();
        let reviews = review_map.get(&repo_path).cloned().unwrap_or_default();
        let ai_sessions = ai_session_map.get(&repo_path).cloned().unwrap_or_default();

        let repo_name = std::path::Path::new(&repo_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| repo_path.clone());

        let commits = events.iter().filter(|e| e.event_type == "commit").count();

        let mut branches: Vec<String> = events
            .iter()
            .filter_map(|e| e.branch.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        branches.sort();

        // Extract AI session time intervals, clip to [from, to]
        let ai_intervals: Vec<TimeInterval> = ai_sessions
            .iter()
            .filter_map(|s| {
                let end = s.ended_at.unwrap_or(now);
                let start = s.started_at.max(from);
                let end = end.min(to);
                if start < end {
                    Some(TimeInterval { start, end })
                } else {
                    None
                }
            })
            .collect();

        let (estimated_time, _) = estimate_time_v2(
            &events,
            &ai_intervals,
            session_gap_minutes,
            first_commit_minutes,
        );

        repos.push(RepoSummary {
            repo_path,
            repo_name,
            commits,
            branches,
            estimated_time,
            events,
            pr_info: None,
            reviews,
            ai_sessions,
        });
    }

    Ok(repos)
}

/// Compute global estimated time by merging all per-repo intervals.
/// Avoids double-counting when working across multiple repos simultaneously.
pub fn global_estimated_time(
    repos: &[RepoSummary],
    session_gap_minutes: u64,
    first_commit_minutes: u64,
) -> Duration {
    let now = Utc::now();
    let mut all_intervals: Vec<TimeInterval> = Vec::new();
    for repo in repos {
        let ai_intervals: Vec<TimeInterval> = repo
            .ai_sessions
            .iter()
            .filter_map(|s| {
                let end = s.ended_at.unwrap_or(now);
                if s.started_at < end {
                    Some(TimeInterval {
                        start: s.started_at,
                        end,
                    })
                } else {
                    None
                }
            })
            .collect();
        let (_, intervals) = estimate_time_v2(
            &repo.events,
            &ai_intervals,
            session_gap_minutes,
            first_commit_minutes,
        );
        all_intervals.extend_from_slice(&intervals);
    }
    let (_, total) = merge_intervals(&mut all_intervals);
    total
}

/// Query review_activity table for a given time range, grouped by repo.
fn query_reviews(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<BTreeMap<String, Vec<ReviewInfo>>> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();

    let mut stmt = conn.prepare(
        "SELECT repo_path, pr_number, pr_title, review_action, reviewed_at
         FROM review_activity
         WHERE reviewed_at >= ?1 AND reviewed_at <= ?2
         ORDER BY repo_path, reviewed_at ASC",
    )?;

    let rows = stmt.query_map(rusqlite::params![from_str, to_str], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;

    let mut map: BTreeMap<String, Vec<ReviewInfo>> = BTreeMap::new();
    for row in rows {
        let (repo_path, pr_number, pr_title, action, reviewed_at_str) = row?;
        let reviewed_at = DateTime::parse_from_rfc3339(&reviewed_at_str)?.with_timezone(&Utc);
        map.entry(repo_path).or_default().push(ReviewInfo {
            pr_number,
            pr_title,
            action,
            reviewed_at,
        });
    }

    Ok(map)
}

/// Query ai_sessions table for a given time range, grouped by repo.
fn query_ai_sessions(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<BTreeMap<String, Vec<AiSessionInfo>>> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();

    let mut stmt = conn.prepare(
        "SELECT repo_path, session_id, started_at, ended_at, turns
         FROM ai_sessions
         WHERE started_at >= ?1 AND started_at <= ?2
         ORDER BY repo_path, started_at ASC",
    )?;

    let now = Utc::now();
    let rows = stmt.query_map(rusqlite::params![from_str, to_str], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<i64>>(4)?,
        ))
    })?;

    let mut map: BTreeMap<String, Vec<AiSessionInfo>> = BTreeMap::new();
    for row in rows {
        let (repo_path, session_id, started_at_str, ended_at_str, turns) = row?;
        let started_at = DateTime::parse_from_rfc3339(&started_at_str)?.with_timezone(&Utc);
        let ended_at = ended_at_str
            .as_deref()
            .map(|s| DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc)))
            .transpose()?;
        let end = ended_at.unwrap_or(now);
        let duration = end - started_at;

        map.entry(repo_path).or_default().push(AiSessionInfo {
            session_id,
            started_at,
            ended_at,
            duration,
            turns,
        });
    }

    Ok(map)
}
