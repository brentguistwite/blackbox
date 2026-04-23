use crate::enrichment::PrInfo;
use chrono::{Datelike, DateTime, Duration, Local, NaiveDate, TimeZone, Timelike, Utc, Weekday};
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

/// Group sorted timestamps into activity segments, splitting on gaps >= `idle_cap`.
///
/// Each segment is a `TimeInterval` from the first to last timestamp within that
/// contiguous burst. Gaps between timestamps that equal-or-exceed `idle_cap` start
/// a new segment — the gap itself is NOT counted as active time.
///
/// Motivation: AI tool sessions often stay "open" for hours while idle. Single-turn
/// timestamps (reads from tool log files) let us distinguish actual work bursts
/// from dead time. See `query_ai_sessions` for the integration point.
///
/// Timestamps are assumed sorted ascending. Zero/one timestamp → empty (no interval
/// has meaningful duration from a single instant).
pub fn segments_from_timestamps(
    timestamps: &[DateTime<Utc>],
    idle_cap: Duration,
) -> Vec<TimeInterval> {
    if timestamps.is_empty() {
        return Vec::new();
    }
    let mut segments = Vec::new();
    let mut seg_start = timestamps[0];
    let mut seg_end = timestamps[0];
    for ts in &timestamps[1..] {
        if *ts - seg_end >= idle_cap {
            segments.push(TimeInterval { start: seg_start, end: seg_end });
            seg_start = *ts;
        }
        seg_end = *ts;
    }
    segments.push(TimeInterval { start: seg_start, end: seg_end });
    segments
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
    let total = merged.iter().fold(Duration::zero(), |acc, iv| acc + (iv.end - iv.start));
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

/// Time estimation v2: adaptive thresholds from commit cadence.
///
/// # Inputs
/// - `events` — git activity (commits, branch switches) sorted chronologically.
/// - `ai_sessions` — AI tool session intervals, pre-clipped to query window.
/// - `presence_intervals` — directory-presence intervals from shell hooks.
///   Used to anchor git session starts: if a presence interval started before the
///   tentative credit window and overlaps it, the session start is pulled back to
///   `presence.start` instead of applying the estimated credit. This yields a more
///   accurate start time when the developer was provably in the directory before
///   committing. Presence-anchored sessions skip AI credit suppression.
/// - `session_gap_minutes` / `first_commit_minutes` — fallback thresholds when < 2 commits.
///
/// # Algorithm
/// 1. Compute adaptive `effective_gap` / `effective_credit` from median commit gap.
/// 2. Group git events into sessions → tentative intervals `[first_event - credit, last_event]`.
/// 3. Presence anchoring — replace tentative start with `presence.start` when applicable.
/// 4. AI credit suppression — if an AI session overlaps the credit window (and not anchored),
///    shrink session start to `first_event`.
/// 5. Union git + AI + presence intervals, merge overlapping, return total duration.
pub fn estimate_time_v2(
    events: &[ActivityEvent],
    ai_sessions: &[TimeInterval],
    presence_intervals: &[TimeInterval],
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

    // Step 2b: Presence anchoring — if a presence interval started before the
    // tentative start and overlaps the credit window, anchor session start to presence.start
    let mut anchored = vec![false; git_intervals.len()];
    for (i, iv) in git_intervals.iter_mut().enumerate() {
        let tentative_start = iv.start;
        if let Some(p) = presence_intervals.iter()
            .filter(|p| p.start < tentative_start && p.end > tentative_start)
            .min_by_key(|p| p.start)
        {
            iv.start = p.start;
            anchored[i] = true;
        }
    }

    // Step 3: Credit suppression — if AI session covers [first_event - credit, first_event],
    // shrink git interval start to first_event (real data > guess).
    // Skip for presence-anchored intervals (presence > credit guess).
    for (i, iv) in git_intervals.iter_mut().enumerate() {
        if anchored[i] { continue; }
        let credit_window_start = iv.start;
        let credit_window_end = iv.start + effective_credit;
        let has_ai_overlap = ai_sessions.iter().any(|ai| {
            ai.start < credit_window_end && ai.end > credit_window_start
        });
        if has_ai_overlap {
            iv.start = credit_window_end; // shrink to first_event
        }
    }

    // Step 4: Collect git + AI + presence intervals
    let mut all_intervals: Vec<TimeInterval> = Vec::new();
    all_intervals.extend_from_slice(&git_intervals);
    all_intervals.extend_from_slice(ai_sessions);
    all_intervals.extend_from_slice(presence_intervals);

    // Step 5: Merge and return total + intervals
    let (merged, total) = merge_intervals(&mut all_intervals);
    (total, merged)
}

/// Compute window-clipped active intervals for a single AI session.
///
/// When `turn_timestamps` is non-empty, splits into segments via
/// `segments_from_timestamps` and clips each to `[from, to]`. Empty or
/// out-of-window segments are dropped.
///
/// When `turn_timestamps` is empty (tool log missing/unreadable/unsupported),
/// falls back to legacy single-interval behavior using
/// `effective_start` / `effective_end`. This preserves old duration for tools
/// without turn-level data (Copilot, Cursor, Windsurf) while upgrading Claude
/// + Codex to segmented accuracy.
pub fn session_intervals(
    session: &AiSessionInfo,
    turn_timestamps: &[DateTime<Utc>],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    idle_cap: Duration,
) -> Vec<TimeInterval> {
    if turn_timestamps.is_empty() {
        let s = session.effective_start(from);
        let e = session.effective_end().min(to);
        return if s < e { vec![TimeInterval { start: s, end: e }] } else { vec![] };
    }
    segments_from_timestamps(turn_timestamps, idle_cap)
        .into_iter()
        .filter_map(|iv| {
            let s = iv.start.max(from);
            let e = iv.end.min(to);
            if s < e { Some(TimeInterval { start: s, end: e }) } else { None }
        })
        .collect()
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
        // Cap all entries at entered + gap. Closed entries have stale left_at
        // (set on next return, not actual departure) so treat same as NULL.
        let effective_end = match left_str {
            Some(ref s) => {
                let left = DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc);
                left.min(entered + gap)
            }
            None => entered + gap,
        };

        // Clip to [from, to]
        let start = entered.max(from);
        let end = effective_end.min(to);
        if start < end {
            map.entry(repo_path).or_default().push(TimeInterval { start, end });
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
pub struct BranchSwitchEvent {
    pub repo_path: String,
    pub from_branch: Option<String>,
    pub to_branch: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ReviewInfo {
    pub pr_number: i64,
    pub pr_title: String,
    pub action: String,
    pub reviewed_at: DateTime<Utc>,
}

/// Default idle cap for AI sessions: if no activity for this many minutes,
/// stop counting session time. Applies to open sessions (no ended_at).
const AI_IDLE_CAP_MINUTES: i64 = 30;

#[derive(Debug, Clone)]
pub struct AiSessionInfo {
    pub tool: String,
    pub session_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub last_active_at: Option<DateTime<Utc>>,
    /// Sum of `segments` durations when populated; else window-clipped
    /// `effective_end - effective_start` fallback.
    pub duration: Duration,
    pub turns: Option<i64>,
    /// Window-clipped active intervals derived from tool log turn timestamps
    /// (split by idle gap). Empty when tool has no turn-level log (Copilot,
    /// Cursor, Windsurf) or when JSONL is missing — callers should fall back
    /// to a single `[effective_start, effective_end]` interval in that case.
    pub segments: Vec<TimeInterval>,
}

impl AiSessionInfo {
    /// Effective start for time estimation within a query window.
    /// Cross-day sessions use last_active_at to avoid midnight-to-now inflation.
    pub fn effective_start(&self, window_start: DateTime<Utc>) -> DateTime<Utc> {
        if self.started_at >= window_start {
            self.started_at
        } else {
            // Session spans across days — last_active_at is the best proxy
            // for when today's work started.
            self.last_active_at.unwrap_or(self.started_at).max(window_start)
        }
    }

    /// Effective end time, accounting for idle cap.
    /// Caps both open AND closed sessions: lazy-close sets ended_at = now
    /// when a stale PID is detected, even if the session was idle for days.
    pub fn effective_end(&self) -> DateTime<Utc> {
        let now = Utc::now();
        let idle_cap = chrono::Duration::minutes(AI_IDLE_CAP_MINUTES);
        match (self.ended_at, self.last_active_at) {
            (Some(ended), Some(active)) => ended.min(active + idle_cap),
            (Some(ended), None) => ended,
            (None, Some(active)) => (active + idle_cap).min(now),
            (None, None) => now,
        }
    }
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
    pub presence_intervals: Vec<TimeInterval>,
    pub branch_switches: usize,
}

#[derive(Debug, Clone)]
pub struct ActivitySummary {
    pub period_label: String,
    pub total_commits: usize,
    pub total_reviews: usize,
    pub total_repos: usize,
    pub total_estimated_time: Duration,
    pub total_ai_session_time: Duration,
    pub streak_days: u32,
    pub total_branch_switches: usize,
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

/// Returns ((today - lookback_days) at midnight local → now_utc).
/// lookback_days=0 is identical to today_range().
pub fn standup_range(lookback_days: u32) -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    let local_today = Local::now().date_naive();
    let start_date = local_today - Duration::days(lookback_days as i64);
    let start_local = start_date.and_hms_opt(0, 0, 0).unwrap();
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

/// Returns (week_start_midnight_local_as_utc, week_end_utc) for digest.
/// `offset`: 0=current week, -1=last week, etc.
/// `week_start`: which weekday begins the week (Mon or Sun).
/// For current week (offset=0), end is capped at `now`.
/// For past weeks, end is start + 7 days (full week boundary).
pub fn digest_week_range(offset: i32, week_start: Weekday) -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    let local_today = Local::now().date_naive();

    // Days since configured week start
    let days_since_start = match week_start {
        Weekday::Sun => local_today.weekday().num_days_from_sunday(),
        _ => local_today.weekday().num_days_from_monday(),
    };

    // Start of current week
    let current_week_start = local_today - Duration::days(days_since_start as i64);
    // Apply offset (negative = past weeks)
    let target_week_start = current_week_start + Duration::weeks(offset as i64);

    let start_local = target_week_start.and_hms_opt(0, 0, 0).unwrap();
    let start_utc = Local
        .from_local_datetime(&start_local)
        .unwrap()
        .with_timezone(&Utc);

    let end_utc = if offset >= 0 {
        // Current or future week: cap at now
        let full_end = start_utc + Duration::weeks(1);
        full_end.min(now)
    } else {
        // Past week: full 7-day boundary
        start_utc + Duration::weeks(1)
    };

    (start_utc, end_utc)
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

/// Returns (quarter_start_midnight_local_as_utc, now_utc).
/// Q1=Jan-Mar, Q2=Apr-Jun, Q3=Jul-Sep, Q4=Oct-Dec.
pub fn quarter_range() -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    let local_today = Local::now().date_naive();
    let quarter_start_month = match local_today.month() {
        1..=3 => 1,
        4..=6 => 4,
        7..=9 => 7,
        _ => 10,
    };
    let first_of_quarter = NaiveDate::from_ymd_opt(local_today.year(), quarter_start_month, 1).unwrap();
    let start_local = first_of_quarter.and_hms_opt(0, 0, 0).unwrap();
    let start_utc = Local
        .from_local_datetime(&start_local)
        .unwrap()
        .with_timezone(&Utc);
    (start_utc, now)
}

/// Parse optional --from/--to date strings into a UTC date range.
/// Both None → current quarter. Both Some → custom range. One Some → bail.
/// Dates parsed as local midnight → UTC.
pub fn resolve_perf_review_range(
    from: Option<&str>,
    to: Option<&str>,
) -> anyhow::Result<(DateTime<Utc>, DateTime<Utc>)> {
    match (from, to) {
        (None, None) => Ok(quarter_range()),
        (Some(f), Some(t)) => {
            let from_date = parse_local_date(f)?;
            let to_date = parse_local_date(t)?;
            if from_date >= to_date {
                anyhow::bail!("--from must be before --to");
            }
            // to_date: end of day (23:59:59)
            let to_end = to_date
                .and_hms_opt(23, 59, 59)
                .unwrap();
            let to_utc = Local
                .from_local_datetime(&to_end)
                .unwrap()
                .with_timezone(&Utc);
            let from_local = from_date.and_hms_opt(0, 0, 0).unwrap();
            let from_utc = Local
                .from_local_datetime(&from_local)
                .unwrap()
                .with_timezone(&Utc);
            Ok((from_utc, to_utc))
        }
        _ => anyhow::bail!("--from and --to must both be provided, or both omitted"),
    }
}

/// Parse a YYYY-MM-DD string into a NaiveDate, or bail with a clear error.
fn parse_local_date(s: &str) -> anyhow::Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("Invalid date: expected YYYY-MM-DD, got '{}'", s))
}

/// Returns date range for heatmap: last N weeks aligned to calendar weeks.
/// Start = Monday of (current_week - weeks) at 00:00 local, end = today 23:59:59 local.
pub fn heatmap_range(weeks: u32) -> (DateTime<Utc>, DateTime<Utc>) {
    let local_today = Local::now().date_naive();
    let weekday = local_today.weekday().num_days_from_monday();
    // Current week's Monday
    let this_monday = local_today - Duration::days(weekday as i64);
    // Go back `weeks` more weeks
    let start_date = this_monday - Duration::days(weeks as i64 * 7);
    let start_local = start_date.and_hms_opt(0, 0, 0).unwrap();
    let start_utc = Local
        .from_local_datetime(&start_local)
        .unwrap()
        .with_timezone(&Utc);

    let end_local = local_today.and_hms_opt(23, 59, 59).unwrap();
    let end_utc = Local
        .from_local_datetime(&end_local)
        .unwrap()
        .with_timezone(&Utc);

    (start_utc, end_utc)
}

/// Returns current commit streak in calendar days (local time).
/// Streak = consecutive days ending today (or yesterday if no commits today yet)
/// with >= 1 commit each day.
/// exclude_weekends: if true, Sat/Sun gaps don't break the streak.
pub fn query_streak(conn: &Connection, exclude_weekends: bool) -> anyhow::Result<u32> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT date(timestamp, 'localtime') as day
         FROM git_activity
         WHERE event_type = 'commit'
         ORDER BY day DESC",
    )?;

    let days: Vec<NaiveDate> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .filter_map(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        .collect();

    if days.is_empty() {
        return Ok(0);
    }

    let day_set: std::collections::HashSet<NaiveDate> = days.iter().copied().collect();
    let today = Local::now().date_naive();
    let mut cursor = today;
    let mut count: u32 = 0;
    let mut grace_used = false; // allow today to have no commits

    // Walk backward from today. Max ~3 years lookback to prevent infinite loop.
    for _ in 0..1100 {
        if day_set.contains(&cursor) {
            count += 1;
            cursor -= Duration::days(1);
        } else if !grace_used && cursor == today {
            // Today has no commits yet — still alive, move to yesterday
            grace_used = true;
            cursor -= Duration::days(1);
        } else if exclude_weekends && is_weekend(cursor) {
            // Weekend day with no commit — skip (doesn't break streak)
            cursor -= Duration::days(1);
        } else {
            break;
        }
    }

    Ok(count)
}

fn is_weekend(date: NaiveDate) -> bool {
    matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun)
}

/// Query branch_switch events in [from, to], ordered by timestamp ASC.
pub fn query_branch_switches(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<Vec<BranchSwitchEvent>> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();

    let mut stmt = conn.prepare(
        "SELECT repo_path, source_branch, branch, timestamp
         FROM git_activity
         WHERE event_type = 'branch_switch' AND timestamp >= ?1 AND timestamp <= ?2
         ORDER BY timestamp ASC",
    )?;

    let rows = stmt.query_map(rusqlite::params![from_str, to_str], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;

    let mut events = Vec::new();
    for row in rows {
        let (repo_path, from_branch, to_branch, timestamp_str) = row?;
        let timestamp = DateTime::parse_from_rfc3339(&timestamp_str)?.with_timezone(&Utc);
        events.push(BranchSwitchEvent {
            repo_path,
            from_branch,
            to_branch,
            timestamp,
        });
    }

    Ok(events)
}

/// Filter out noise from branch switch events.
///
/// Removes: detached HEAD (to_branch is None), same-branch re-checkouts,
/// and round-trip pairs (A→B→A) where elapsed time < min_dwell_secs.
pub fn filter_noise_switches(
    events: &[BranchSwitchEvent],
    min_dwell_secs: u64,
) -> Vec<BranchSwitchEvent> {
    // Step 1: exclude detached HEAD and same-branch re-checkouts
    let clean: Vec<&BranchSwitchEvent> = events
        .iter()
        .filter(|e| e.to_branch.is_some())
        .filter(|e| {
            match (&e.from_branch, &e.to_branch) {
                (Some(f), Some(t)) => f != t,
                _ => true, // keep if from_branch is None (first poll, etc.)
            }
        })
        .collect();

    // Step 2: collapse round-trip pairs A→B→A where elapsed < threshold
    let threshold = chrono::Duration::seconds(min_dwell_secs as i64);
    let mut remove = vec![false; clean.len()];

    // Sliding window: if clean[i].to_branch == clean[i-2].to_branch
    // and clean[i].from_branch == clean[i-1].to_branch (consecutive pair)
    // and elapsed < threshold, remove both i-1 and i.
    let mut i = clean.len();
    while i >= 2 {
        i -= 1;
        if remove[i] || remove[i - 1] {
            continue;
        }
        let prev = clean[i - 1];
        let curr = clean[i];
        // Check: prev switches to X, curr switches back from X
        // i.e. prev.to == curr.from AND curr.to == prev.from (round-trip)
        let is_round_trip = match (&prev.to_branch, &curr.from_branch, &curr.to_branch, &prev.from_branch) {
            (Some(pt), Some(cf), Some(ct), Some(pf)) => pt == cf && ct == pf,
            _ => false,
        };
        if is_round_trip {
            let elapsed = curr.timestamp - prev.timestamp;
            if elapsed < threshold {
                remove[i - 1] = true;
                remove[i] = true;
            }
        }
    }

    clean
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| !remove[*idx])
        .map(|(_, e)| e.clone())
        .collect()
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

    // Query reviews in the same time range
    let review_map = query_reviews(conn, from, to)?;

    // Query AI sessions in the same time range
    let ai_session_map = query_ai_sessions(conn, from, to)?;

    // Query presence intervals once for all repos
    let presence_map = query_presence(conn, from, to, session_gap_minutes)?;

    // Query branch switches, filter noise, count per repo
    let all_switches = query_branch_switches(conn, from, to)?;
    let filtered_switches = filter_noise_switches(&all_switches, 120);
    let mut switch_counts: BTreeMap<String, usize> = BTreeMap::new();
    for sw in &filtered_switches {
        *switch_counts.entry(sw.repo_path.clone()).or_default() += 1;
    }

    // Collect all repo paths. Filter out AI sessions from user's home dir.
    let home_dir = etcetera::home_dir().ok().map(|h| h.to_string_lossy().to_string());
    let not_home = |k: &&String| home_dir.as_ref().is_none_or(|h| k.as_str() != h);
    let all_repo_paths: std::collections::BTreeSet<String> = repo_map
        .keys()
        .chain(review_map.keys())
        .chain(ai_session_map.keys().filter(not_home))
        .cloned()
        .collect();

    let mut repos: Vec<RepoSummary> = Vec::new();
    for repo_path in all_repo_paths {
        let events = repo_map.remove(&repo_path).unwrap_or_default();
        let reviews = review_map.get(&repo_path).cloned().unwrap_or_default();
        let ai_sessions = ai_session_map.get(&repo_path).cloned().unwrap_or_default();

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

        // AI intervals = flattened per-session segments (already window-clipped
        // by query_ai_sessions). Segments reflect actual turn-level activity
        // bursts rather than full session lifetime.
        let ai_intervals: Vec<TimeInterval> = ai_sessions
            .iter()
            .flat_map(|s| s.segments.iter().copied())
            .collect();

        let presence: Vec<TimeInterval> = presence_map.get(&repo_path).cloned().unwrap_or_default();

        let (estimated_time, _) = estimate_time_v2(
            &events, &ai_intervals, &presence, session_gap_minutes, first_commit_minutes,
        );

        let branch_switches = switch_counts.get(&repo_path).copied().unwrap_or(0);

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
            presence_intervals: presence,
            branch_switches,
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
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Duration {
    let mut all_intervals: Vec<TimeInterval> = Vec::new();
    for repo in repos {
        let ai_intervals: Vec<TimeInterval> = repo
            .ai_sessions
            .iter()
            .flat_map(|s| s.segments.iter().copied())
            .collect();
        let _ = from; let _ = to; // kept in sig for API stability; segments pre-clipped
        let (_, intervals) = estimate_time_v2(
            &repo.events, &ai_intervals, &repo.presence_intervals, session_gap_minutes, first_commit_minutes,
        );
        all_intervals.extend_from_slice(&intervals);
    }
    let (_, total) = merge_intervals(&mut all_intervals);
    total
}

/// Build a compact notification body for today's activity.
/// Returns None if no commits today (no notification needed).
pub fn daily_summary_for_notification(
    conn: &Connection,
    session_gap_minutes: u64,
    first_commit_minutes: u64,
) -> anyhow::Result<Option<String>> {
    let (from, to) = today_range();
    let repos = query_activity(conn, from, to, session_gap_minutes, first_commit_minutes)?;

    let total_commits: usize = repos.iter().map(|r| r.commits).sum();
    if total_commits == 0 {
        return Ok(None);
    }

    let repo_count = repos.len();
    let total_time = global_estimated_time(&repos, session_gap_minutes, first_commit_minutes, from, to);

    let commit_word = if total_commits == 1 { "commit" } else { "commits" };
    let repo_word = if repo_count == 1 { "repo" } else { "repos" };

    let mins = total_time.num_minutes();
    let h = mins / 60;
    let m = mins % 60;
    let time_str = match (h, m) {
        (0, 0) => "< 1m".to_string(),
        (0, _) => format!("{}m", m),
        (_, 0) => format!("{}h", h),
        _ => format!("{}h {}m", h, m),
    };

    Ok(Some(format!(
        "{} {} across {} {} — ~{}",
        total_commits, commit_word, repo_count, repo_word, time_str
    )))
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
    // Include sessions whose effective end could overlap with the query window.
    // For open sessions, effective end = last_active_at + idle_cap.
    let from_minus_idle = (from - chrono::Duration::minutes(AI_IDLE_CAP_MINUTES)).to_rfc3339();

    let mut stmt = conn.prepare(
        "SELECT repo_path, tool, session_id, started_at, ended_at, turns, last_active_at
         FROM ai_sessions
         WHERE started_at <= ?2
           AND (started_at >= ?1 OR last_active_at >= ?3)
         ORDER BY repo_path, started_at ASC",
    )?;

    let rows = stmt.query_map(rusqlite::params![from_str, to_str, from_minus_idle], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, Option<String>>(6)?,
        ))
    })?;

    let mut map: BTreeMap<String, Vec<AiSessionInfo>> = BTreeMap::new();
    for row in rows {
        let (repo_path, tool, session_id, started_at_str, ended_at_str, turns, last_active_str) = row?;
        let started_at = DateTime::parse_from_rfc3339(&started_at_str)?.with_timezone(&Utc);
        let ended_at = ended_at_str
            .as_deref()
            .map(|s| DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc)))
            .transpose()?;
        let last_active_at = last_active_str
            .as_deref()
            .map(|s| DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc)))
            .transpose()?;

        // Build struct. Duration + segments derived from per-turn timestamps
        // when available (Claude/Codex JSONL logs). Fallback: single interval
        // clipped to [from, to] via effective_start/effective_end — preserves
        // legacy behavior for tools without turn-level logs.
        let info = AiSessionInfo {
            tool: tool.clone(),
            session_id: session_id.clone(),
            started_at,
            ended_at,
            last_active_at,
            duration: chrono::Duration::zero(),
            turns,
            segments: Vec::new(),
        };
        let idle_cap = chrono::Duration::minutes(AI_IDLE_CAP_MINUTES);
        let turn_timestamps = crate::ai_tracking::session_turn_timestamps(&tool, &session_id);
        let segments = session_intervals(&info, &turn_timestamps, from, to, idle_cap);
        let duration = segments
            .iter()
            .fold(chrono::Duration::zero(), |acc, iv| acc + (iv.end - iv.start));
        map.entry(repo_path).or_default().push(AiSessionInfo { duration, segments, ..info });
    }

    Ok(map)
}

/// Query daily commit counts over an arbitrary date range.
/// Groups `git_activity` rows where `event_type='commit'` by local calendar date.
/// Returns a zero-filled BTreeMap with an entry for every date in [from, to].
pub fn query_daily_commit_counts(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<BTreeMap<NaiveDate, u32>> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();

    let mut stmt = conn.prepare(
        "SELECT date(timestamp, 'localtime') as day, COUNT(*)
         FROM git_activity
         WHERE event_type='commit' AND timestamp >= ?1 AND timestamp <= ?2
         GROUP BY day",
    )?;

    let rows = stmt.query_map(rusqlite::params![from_str, to_str], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
    })?;

    // Collect DB results into a temporary map
    let mut counts: BTreeMap<NaiveDate, u32> = BTreeMap::new();
    for row in rows {
        let (day_str, count) = row?;
        let date = NaiveDate::parse_from_str(&day_str, "%Y-%m-%d")?;
        counts.insert(date, count);
    }

    // Zero-fill: ensure every date in [from.date(), to.date()] is present
    let start_date = from.with_timezone(&Local).date_naive();
    let end_date = to.with_timezone(&Local).date_naive();
    let mut result: BTreeMap<NaiveDate, u32> = BTreeMap::new();
    let mut current = start_date;
    while current <= end_date {
        result.insert(current, counts.get(&current).copied().unwrap_or(0));
        current += Duration::days(1);
    }

    Ok(result)
}



#[derive(Debug, Clone, serde::Serialize)]
pub struct AfterHoursStats {
    pub total_commits: u32,
    pub after_hours_commits: u32,
    pub weekend_commits: u32,
    pub after_hours_ratio: f64,
    pub weekend_ratio: f64,
}

/// Count commits during after-hours (local hour < 9 or >= 18) and on weekends (Sat/Sun local).
/// Returns counts and ratios. Ratios are 0.0 when total_commits == 0.
/// Only counts event_type='commit'.
pub fn after_hours_ratio(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<AfterHoursStats> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();

    let mut stmt = conn.prepare(
        "SELECT timestamp FROM git_activity
         WHERE event_type = 'commit' AND timestamp >= ?1 AND timestamp <= ?2",
    )?;

    let mut total: u32 = 0;
    let mut after_hours: u32 = 0;
    let mut weekend: u32 = 0;

    let rows = stmt.query_map(rusqlite::params![from_str, to_str], |row| {
        row.get::<_, String>(0)
    })?;

    for row in rows {
        let ts_str = row?;
        let utc_dt = DateTime::parse_from_rfc3339(&ts_str)?.with_timezone(&Utc);
        let local_dt = utc_dt.with_timezone(&Local);
        let hour = local_dt.hour();
        let dow = local_dt.weekday().num_days_from_monday();

        total += 1;
        if !(9..18).contains(&hour) {
            after_hours += 1;
        }
        if dow >= 5 {
            weekend += 1;
        }
    }

    let ah_ratio = if total > 0 { after_hours as f64 / total as f64 } else { 0.0 };
    let wk_ratio = if total > 0 { weekend as f64 / total as f64 } else { 0.0 };

    Ok(AfterHoursStats {
        total_commits: total,
        after_hours_commits: after_hours,
        weekend_commits: weekend,
        after_hours_ratio: ah_ratio,
        weekend_ratio: wk_ratio,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionDistribution {
    #[serde(skip)]
    pub sessions: Vec<Duration>,
    pub median_minutes: i64,
    pub p90_minutes: i64,
    pub mean_minutes: i64,
}

/// Compute session length distribution from commit timestamps across all repos.
/// Uses same session-gap logic as estimate_time_v2: adaptive thresholds from median commit gap.
/// Sessions < 5 min excluded as noise. Returns all-zero struct if no qualifying sessions.
pub fn session_length_distribution(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    session_gap_minutes: u64,
    first_commit_minutes: u64,
) -> anyhow::Result<SessionDistribution> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();

    let mut stmt = conn.prepare(
        "SELECT timestamp FROM git_activity
         WHERE event_type = 'commit' AND timestamp >= ?1 AND timestamp <= ?2
         ORDER BY timestamp ASC",
    )?;

    let rows = stmt.query_map(rusqlite::params![from_str, to_str], |row| {
        row.get::<_, String>(0)
    })?;

    let mut timestamps: Vec<DateTime<Utc>> = Vec::new();
    for row in rows {
        let ts_str = row?;
        let utc_dt = DateTime::parse_from_rfc3339(&ts_str)?.with_timezone(&Utc);
        timestamps.push(utc_dt);
    }

    if timestamps.is_empty() {
        return Ok(SessionDistribution {
            sessions: vec![],
            median_minutes: 0,
            p90_minutes: 0,
            mean_minutes: 0,
        });
    }

    // Compute adaptive thresholds (same as estimate_time_v2)
    let gaps: Vec<Duration> = timestamps
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| d.num_seconds() > 0)
        .collect();

    let (effective_gap, effective_credit) = if gaps.is_empty() {
        (
            Duration::minutes(session_gap_minutes as i64),
            Duration::minutes(first_commit_minutes as i64),
        )
    } else {
        let mut sorted_gaps = gaps.clone();
        sorted_gaps.sort();
        let mid = sorted_gaps.len() / 2;
        let median = if sorted_gaps.len().is_multiple_of(2) {
            (sorted_gaps[mid - 1] + sorted_gaps[mid]) / 2
        } else {
            sorted_gaps[mid]
        };
        let median_mins = median.num_minutes();
        let gap = (median_mins * 3).clamp(MIN_GAP_FLOOR_MINS, MAX_GAP_CAP_MINS);
        let credit = median_mins.clamp(MIN_CREDIT_MINS, MAX_CREDIT_MINS);
        (Duration::minutes(gap), Duration::minutes(credit))
    };

    // Group into sessions
    let mut session_durations: Vec<Duration> = Vec::new();
    let mut session_start = 0usize;

    for i in 1..=timestamps.len() {
        let is_end = i == timestamps.len()
            || (timestamps[i] - timestamps[i - 1]) >= effective_gap;
        if is_end {
            let first_ts = timestamps[session_start];
            let last_ts = timestamps[i - 1];
            let duration = (last_ts - first_ts) + effective_credit;
            session_durations.push(duration);
            if i < timestamps.len() {
                session_start = i;
            }
        }
    }

    // Exclude sessions < 5 min
    let min_session = Duration::minutes(5);
    let mut qualifying: Vec<Duration> = session_durations
        .into_iter()
        .filter(|d| *d >= min_session)
        .collect();

    if qualifying.is_empty() {
        return Ok(SessionDistribution {
            sessions: vec![],
            median_minutes: 0,
            p90_minutes: 0,
            mean_minutes: 0,
        });
    }

    qualifying.sort();
    let n = qualifying.len();

    let median = if n.is_multiple_of(2) {
        (qualifying[n / 2 - 1] + qualifying[n / 2]) / 2
    } else {
        qualifying[n / 2]
    };

    let p90_idx = ((n as f64 * 0.9).ceil() as usize).min(n) - 1;
    let p90 = qualifying[p90_idx];

    let total_mins: i64 = qualifying.iter().map(|d| d.num_minutes()).sum();
    let mean_mins = total_mins / n as i64;

    Ok(SessionDistribution {
        sessions: qualifying,
        median_minutes: median.num_minutes(),
        p90_minutes: p90.num_minutes(),
        mean_minutes: mean_mins,
    })
}

/// Count commits per local hour of day. Returns [u32; 24] indexed 0–23.
/// Only counts event_type='commit'. Converts UTC timestamps to local time before bucketing.
pub fn commit_hour_histogram(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<[u32; 24]> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();

    let mut stmt = conn.prepare(
        "SELECT timestamp FROM git_activity
         WHERE event_type = 'commit' AND timestamp >= ?1 AND timestamp <= ?2",
    )?;

    let mut histogram = [0u32; 24];
    let rows = stmt.query_map(rusqlite::params![from_str, to_str], |row| {
        row.get::<_, String>(0)
    })?;

    for row in rows {
        let ts_str = row?;
        let utc_dt = DateTime::parse_from_rfc3339(&ts_str)?.with_timezone(&Utc);
        let local_dt = utc_dt.with_timezone(&Local);
        let hour = local_dt.hour() as usize;
        histogram[hour] += 1;
    }

    Ok(histogram)
}

/// Count commits per local day of week. Returns [u32; 7] indexed Mon=0..Sun=6.
/// Only counts event_type='commit'. Converts UTC timestamps to local time before bucketing.
pub fn commit_dow_histogram(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<[u32; 7]> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();

    let mut stmt = conn.prepare(
        "SELECT timestamp FROM git_activity
         WHERE event_type = 'commit' AND timestamp >= ?1 AND timestamp <= ?2",
    )?;

    let mut histogram = [0u32; 7];
    let rows = stmt.query_map(rusqlite::params![from_str, to_str], |row| {
        row.get::<_, String>(0)
    })?;

    for row in rows {
        let ts_str = row?;
        let utc_dt = DateTime::parse_from_rfc3339(&ts_str)?.with_timezone(&Utc);
        let local_dt = utc_dt.with_timezone(&Local);
        let dow = local_dt.weekday().num_days_from_monday() as usize;
        histogram[dow] += 1;
    }

    Ok(histogram)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum CommitPattern {
    Burst,
    Steady,
    Insufficient,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BurstStats {
    pub commit_count: u32,
    pub cv_of_gaps: f64,
    pub pattern: CommitPattern,
}

/// Classify commit pattern as burst vs steady using coefficient of variation of inter-commit gaps.
/// Insufficient when < 3 commits. Burst when CV > 1.0. Steady when CV <= 1.0.
/// CV = std_dev / mean; 0.0 if mean is 0.
pub fn burst_pattern(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<BurstStats> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();

    let mut stmt = conn.prepare(
        "SELECT timestamp FROM git_activity
         WHERE event_type = 'commit' AND timestamp >= ?1 AND timestamp <= ?2
         ORDER BY timestamp ASC",
    )?;

    let rows = stmt.query_map(rusqlite::params![from_str, to_str], |row| {
        row.get::<_, String>(0)
    })?;

    let mut timestamps: Vec<DateTime<Utc>> = Vec::new();
    for row in rows {
        let ts_str = row?;
        let utc_dt = DateTime::parse_from_rfc3339(&ts_str)?.with_timezone(&Utc);
        timestamps.push(utc_dt);
    }

    let commit_count = timestamps.len() as u32;

    if commit_count < 3 {
        return Ok(BurstStats {
            commit_count,
            cv_of_gaps: 0.0,
            pattern: CommitPattern::Insufficient,
        });
    }

    let gaps: Vec<f64> = timestamps
        .windows(2)
        .map(|w| (w[1] - w[0]).num_seconds() as f64)
        .collect();

    let n = gaps.len() as f64;
    let mean = gaps.iter().sum::<f64>() / n;

    if mean == 0.0 {
        return Ok(BurstStats {
            commit_count,
            cv_of_gaps: 0.0,
            pattern: CommitPattern::Insufficient,
        });
    }

    let variance = gaps.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / n;
    let std_dev = variance.sqrt();
    let cv = std_dev / mean;

    let pattern = if cv > 1.0 {
        CommitPattern::Burst
    } else {
        CommitPattern::Steady
    };

    Ok(BurstStats {
        commit_count,
        cv_of_gaps: cv,
        pattern,
    })
}

// --- PR cycle time types and queries ---

#[derive(Debug, Clone, serde::Serialize)]
pub struct PrMetrics {
    pub pr_number: i64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub cycle_time_hours: Option<f64>,
    pub time_to_first_review_hours: Option<f64>,
    pub size_lines: Option<i64>,
    pub iteration_count: Option<i64>,
    pub created_at: Option<DateTime<Utc>>,
    pub merged_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PrCycleStats {
    pub prs: Vec<PrMetrics>,
    pub median_cycle_time_hours: Option<f64>,
    pub median_time_to_first_review_hours: Option<f64>,
    pub median_pr_size_lines: Option<f64>,
    pub median_iteration_count: Option<f64>,
    pub total_prs: usize,
    pub merged_prs: usize,
}

fn median_f64(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        Some((values[mid - 1] + values[mid]) / 2.0)
    } else {
        Some(values[mid])
    }
}

fn parse_dt_opt(s: &Option<String>) -> Option<DateTime<Utc>> {
    s.as_deref().and_then(|v| {
        DateTime::parse_from_rfc3339(v)
            .map(|dt| dt.with_timezone(&Utc))
            .ok()
    })
}

/// Query pr_snapshots for cycle time metrics over a date range.
/// Filters on created_at_gh within [from, to]. Optional repo_path_filter.
pub fn query_pr_cycle_stats(
    conn: &Connection,
    repo_path_filter: Option<&str>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<PrCycleStats> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();

    let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match repo_path_filter {
        Some(repo) => (
            "SELECT pr_number, title, url, state, created_at_gh, merged_at, closed_at,
                    first_review_at, additions, deletions, commits, iteration_count
             FROM pr_snapshots
             WHERE created_at_gh >= ?1 AND created_at_gh <= ?2 AND repo_path = ?3
             ORDER BY created_at_gh ASC".to_string(),
            vec![
                Box::new(from_str) as Box<dyn rusqlite::types::ToSql>,
                Box::new(to_str),
                Box::new(repo.to_string()),
            ],
        ),
        None => (
            "SELECT pr_number, title, url, state, created_at_gh, merged_at, closed_at,
                    first_review_at, additions, deletions, commits, iteration_count
             FROM pr_snapshots
             WHERE created_at_gh >= ?1 AND created_at_gh <= ?2
             ORDER BY created_at_gh ASC".to_string(),
            vec![
                Box::new(from_str) as Box<dyn rusqlite::types::ToSql>,
                Box::new(to_str),
            ],
        ),
    };

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<i64>>(8)?,
            row.get::<_, Option<i64>>(9)?,
            row.get::<_, Option<i64>>(10)?,
            row.get::<_, Option<i64>>(11)?,
        ))
    })?;

    let mut prs = Vec::new();
    let mut cycle_times = Vec::new();
    let mut review_times = Vec::new();
    let mut sizes = Vec::new();
    let mut iterations = Vec::new();
    let mut merged_count = 0usize;

    for row in rows {
        let (pr_number, title, url, state, created_at_str, merged_at_str, _closed_at_str,
             first_review_at_str, additions, deletions, _commits, iteration_count) = row?;

        let created_at = parse_dt_opt(&created_at_str);
        let merged_at = parse_dt_opt(&merged_at_str);
        let first_review_at = parse_dt_opt(&first_review_at_str);

        let cycle_time_hours = match (created_at, merged_at) {
            (Some(c), Some(m)) => Some((m - c).num_seconds() as f64 / 3600.0),
            _ => None,
        };

        let time_to_first_review_hours = match (created_at, first_review_at) {
            (Some(c), Some(r)) => Some((r - c).num_seconds() as f64 / 3600.0),
            _ => None,
        };

        let size_lines = match (additions, deletions) {
            (Some(a), Some(d)) => Some(a + d),
            (Some(a), None) => Some(a),
            (None, Some(d)) => Some(d),
            (None, None) => None,
        };

        if state == "MERGED" {
            merged_count += 1;
        }
        if let Some(ct) = cycle_time_hours {
            cycle_times.push(ct);
        }
        if let Some(rt) = time_to_first_review_hours {
            review_times.push(rt);
        }
        if let Some(s) = size_lines {
            sizes.push(s as f64);
        }
        if let Some(ic) = iteration_count {
            iterations.push(ic as f64);
        }

        prs.push(PrMetrics {
            pr_number,
            title,
            url,
            state,
            cycle_time_hours,
            time_to_first_review_hours,
            size_lines,
            iteration_count,
            created_at,
            merged_at,
        });
    }

    let total_prs = prs.len();

    Ok(PrCycleStats {
        prs,
        median_cycle_time_hours: median_f64(cycle_times),
        median_time_to_first_review_hours: median_f64(review_times),
        median_pr_size_lines: median_f64(sizes),
        median_iteration_count: median_f64(iterations),
        total_prs,
        merged_prs: merged_count,
    })
}

// --- Insights data aggregation (US-001) ---

#[derive(Debug, Clone, serde::Serialize)]
pub struct RepoInsights {
    pub repo_name: String,
    pub commits: usize,
    pub estimated_minutes: i64,
    pub branches_touched: usize,
    pub has_prs: bool,
    pub avg_commit_msg_len: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InsightsData {
    pub period_label: String,
    pub total_commits: usize,
    pub total_repos: usize,
    pub commits_by_dow: [u32; 7],
    pub commits_by_hour: [u32; 24],
    pub avg_msg_len_by_dow: [f64; 7],
    pub bugfix_commits: u32,
    pub total_commits_with_msg: u32,
    pub pr_merge_times_hours: Vec<f64>,
    pub per_repo: Vec<RepoInsights>,
}

fn is_bugfix_commit(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    ["fix", "bug", "hotfix", "patch", "revert"]
        .iter()
        .any(|kw| lower.contains(kw))
}

pub fn aggregate_insights_data(repos: &[RepoSummary], period_label: &str) -> InsightsData {
    let mut commits_by_dow = [0u32; 7];
    let mut commits_by_hour = [0u32; 24];
    let mut msg_len_sum_by_dow = [0u64; 7];
    let mut msg_count_by_dow = [0u32; 7];
    let mut bugfix_commits = 0u32;
    let mut total_commits_with_msg = 0u32;
    let mut total_commits = 0usize;

    let mut per_repo = Vec::with_capacity(repos.len());

    for repo in repos {
        let mut repo_msg_len_total = 0u64;
        let mut repo_msg_count = 0u32;

        for event in &repo.events {
            if event.event_type != "commit" {
                continue;
            }
            total_commits += 1;

            let local = event.timestamp.with_timezone(&Local);
            let dow = match local.weekday() {
                chrono::Weekday::Mon => 0,
                chrono::Weekday::Tue => 1,
                chrono::Weekday::Wed => 2,
                chrono::Weekday::Thu => 3,
                chrono::Weekday::Fri => 4,
                chrono::Weekday::Sat => 5,
                chrono::Weekday::Sun => 6,
            };
            commits_by_dow[dow] += 1;
            commits_by_hour[local.hour() as usize] += 1;

            if let Some(msg) = &event.message {
                let len = msg.len();
                total_commits_with_msg += 1;
                msg_len_sum_by_dow[dow] += len as u64;
                msg_count_by_dow[dow] += 1;
                repo_msg_len_total += len as u64;
                repo_msg_count += 1;

                if is_bugfix_commit(msg) {
                    bugfix_commits += 1;
                }
            }
        }

        per_repo.push(RepoInsights {
            repo_name: repo.repo_name.clone(),
            commits: repo.commits,
            estimated_minutes: repo.estimated_time.num_minutes(),
            branches_touched: repo.branches.len(),
            has_prs: repo.pr_info.as_ref().is_some_and(|prs| !prs.is_empty()),
            avg_commit_msg_len: if repo_msg_count > 0 {
                repo_msg_len_total as f64 / repo_msg_count as f64
            } else {
                0.0
            },
        });
    }

    let mut avg_msg_len_by_dow = [0.0f64; 7];
    for i in 0..7 {
        if msg_count_by_dow[i] > 0 {
            avg_msg_len_by_dow[i] = msg_len_sum_by_dow[i] as f64 / msg_count_by_dow[i] as f64;
        }
    }

    // Compute PR merge times from enrichment data
    let mut pr_merge_times_hours = Vec::new();
    for repo in repos {
        if let Some(prs) = &repo.pr_info {
            for pr in prs {
                if pr.state == "MERGED"
                    && let (Some(created), Some(merged)) = (&pr.created_at, &pr.merged_at)
                    && let (Ok(c), Ok(m)) = (
                        DateTime::parse_from_rfc3339(created),
                        DateTime::parse_from_rfc3339(merged),
                    )
                {
                    let hours = (m - c).num_minutes() as f64 / 60.0;
                    pr_merge_times_hours.push(hours);
                }
            }
        }
    }

    InsightsData {
        period_label: period_label.to_string(),
        total_commits,
        total_repos: repos.len(),
        commits_by_dow,
        commits_by_hour,
        avg_msg_len_by_dow,
        bugfix_commits,
        total_commits_with_msg,
        pr_merge_times_hours,
        per_repo,
    }
}

// --- Commit quality trend (US-016-04) ---

#[derive(Debug, Clone, serde::Serialize)]
pub struct WeeklyQuality {
    pub week_start: NaiveDate,
    pub commit_count: usize,
    pub avg_score: f64,
    pub vague_count: usize,
    pub vague_pct: f64,
}

/// Weekly average commit quality scores over `weeks` calendar weeks back from now.
/// Groups by ISO week (Monday start). Weeks with zero scored commits get avg_score=0.0.
pub fn commit_quality_trend(
    conn: &Connection,
    weeks: u32,
) -> anyhow::Result<Vec<WeeklyQuality>> {
    let now = Local::now();
    let today = now.date_naive();
    let days_since_monday = today.weekday().num_days_from_monday();
    let this_monday = today - Duration::days(days_since_monday as i64);
    let start_monday = this_monday - Duration::days((weeks as i64 - 1) * 7);
    let from_str = start_monday.and_hms_opt(0, 0, 0).unwrap().and_local_timezone(Local).unwrap().to_rfc3339();

    let mut stmt = conn.prepare(
        "SELECT cq.score, cq.is_vague, ga.timestamp
         FROM commit_quality cq
         JOIN git_activity ga ON ga.repo_path = cq.repo_path AND ga.commit_hash = cq.commit_hash
         WHERE ga.event_type = 'commit' AND ga.timestamp >= ?1
         ORDER BY ga.timestamp ASC",
    )?;

    // Bucket by ISO week
    let mut buckets: BTreeMap<NaiveDate, (Vec<u8>, usize)> = BTreeMap::new();
    let rows = stmt.query_map(rusqlite::params![from_str], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
    })?;

    for row in rows {
        let (score, is_vague, ts_str) = row?;
        let utc_dt = DateTime::parse_from_rfc3339(&ts_str)
            .map(|d| d.with_timezone(&Local))
            .unwrap_or_else(|_| Local::now());
        let d = utc_dt.date_naive();
        let dow = d.weekday().num_days_from_monday();
        let monday = d - Duration::days(dow as i64);
        let entry = buckets.entry(monday).or_insert_with(|| (Vec::new(), 0));
        entry.0.push(score as u8);
        if is_vague != 0 {
            entry.1 += 1;
        }
    }

    // Build result with zero-filled weeks
    let mut result = Vec::with_capacity(weeks as usize);
    let mut week = start_monday;
    while week <= this_monday {
        if let Some((scores, vague_count)) = buckets.get(&week) {
            let count = scores.len();
            let sum: u64 = scores.iter().map(|&s| s as u64).sum();
            let avg = if count > 0 { sum as f64 / count as f64 } else { 0.0 };
            let vague_pct = if count > 0 { *vague_count as f64 / count as f64 * 100.0 } else { 0.0 };
            result.push(WeeklyQuality {
                week_start: week,
                commit_count: count,
                avg_score: avg,
                vague_count: *vague_count,
                vague_pct,
            });
        } else {
            result.push(WeeklyQuality {
                week_start: week,
                commit_count: 0,
                avg_score: 0.0,
                vague_count: 0,
                vague_pct: 0.0,
            });
        }
        week += Duration::days(7);
    }

    Ok(result)
}

// --- Revert correlation (US-016-05) ---

#[derive(Debug, Clone, serde::Serialize)]
pub struct RevertedCommit {
    pub original_hash: String,
    pub original_message: String,
    pub revert_hash: String,
    pub score: Option<u8>,
    pub repo_path: String,
}

/// Find commits that were reverted and correlate with their quality scores.
/// Detects reverts by message matching `^[Rr]evert .+` with body containing
/// `This reverts commit <hash>`.
pub fn find_reverted_commits(conn: &Connection) -> anyhow::Result<Vec<RevertedCommit>> {
    let mut stmt = conn.prepare(
        "SELECT repo_path, commit_hash, message
         FROM git_activity
         WHERE event_type = 'commit' AND message LIKE 'revert %'",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut results = Vec::new();
    for row in rows {
        let (repo_path, revert_hash, message) = row?;
        // Extract original hash: find first 40-char hex string in message
        let original_hash = match find_hex40(&message) {
            Some(h) => h,
            None => continue,
        };

        // Look up original message
        let original_message: String = conn
            .query_row(
                "SELECT COALESCE(message, '') FROM git_activity WHERE commit_hash = ?1 AND repo_path = ?2",
                rusqlite::params![&original_hash, &repo_path],
                |row| row.get(0),
            )
            .unwrap_or_default();

        // Look up score
        let score: Option<u8> = conn
            .query_row(
                "SELECT score FROM commit_quality WHERE commit_hash = ?1 AND repo_path = ?2",
                rusqlite::params![&original_hash, &repo_path],
                |row| row.get::<_, i64>(0).map(|s| s as u8),
            )
            .ok();

        results.push(RevertedCommit {
            original_hash,
            original_message,
            revert_hash,
            score,
            repo_path,
        });
    }

    Ok(results)
}

/// Find first contiguous 40-char lowercase hex string in `s`.
fn find_hex40(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 40 <= bytes.len() {
        if bytes[i].is_ascii_hexdigit() && bytes[i].is_ascii_lowercase() || bytes[i].is_ascii_digit() {
            let candidate = &s[i..i + 40];
            if candidate.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(candidate.to_string());
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;
    use tempfile::TempDir;

    /// Helper: convert local midnight NaiveDate to DateTime<Utc>
    fn local_midnight_utc(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        let nd = NaiveDate::from_ymd_opt(year, month, day).unwrap();
        Local
            .from_local_datetime(&nd.and_hms_opt(0, 0, 0).unwrap())
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Helper: convert local end-of-day NaiveDate to DateTime<Utc>
    fn local_eod_utc(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        let nd = NaiveDate::from_ymd_opt(year, month, day).unwrap();
        Local
            .from_local_datetime(&nd.and_hms_opt(23, 59, 59).unwrap())
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn test_query_daily_commit_counts() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let conn = open_db(&db_path).unwrap();

        // Use local noon timestamps so date(timestamp,'localtime') = expected date
        let day1 = NaiveDate::from_ymd_opt(2025, 1, 10).unwrap();
        let day3 = NaiveDate::from_ymd_opt(2025, 1, 12).unwrap();

        let noon1 = Local
            .from_local_datetime(&day1.and_hms_opt(12, 0, 0).unwrap())
            .unwrap()
            .with_timezone(&Utc)
            .to_rfc3339();
        let noon1b = Local
            .from_local_datetime(&day1.and_hms_opt(14, 0, 0).unwrap())
            .unwrap()
            .with_timezone(&Utc)
            .to_rfc3339();
        let noon3 = Local
            .from_local_datetime(&day3.and_hms_opt(10, 0, 0).unwrap())
            .unwrap()
            .with_timezone(&Utc)
            .to_rfc3339();
        let gap_ts = Local
            .from_local_datetime(&NaiveDate::from_ymd_opt(2025, 1, 11).unwrap().and_hms_opt(9, 0, 0).unwrap())
            .unwrap()
            .with_timezone(&Utc)
            .to_rfc3339();

        // Day 1: 2 commits
        conn.execute(
            "INSERT INTO git_activity (repo_path, event_type, branch, commit_hash, author, message, timestamp)
             VALUES ('repo1', 'commit', 'main', 'aaa', 'dev', 'msg1', ?1)",
            rusqlite::params![noon1],
        ).unwrap();
        conn.execute(
            "INSERT INTO git_activity (repo_path, event_type, branch, commit_hash, author, message, timestamp)
             VALUES ('repo1', 'commit', 'main', 'bbb', 'dev', 'msg2', ?1)",
            rusqlite::params![noon1b],
        ).unwrap();
        // Day 3: 1 commit
        conn.execute(
            "INSERT INTO git_activity (repo_path, event_type, branch, commit_hash, author, message, timestamp)
             VALUES ('repo1', 'commit', 'main', 'ccc', 'dev', 'msg3', ?1)",
            rusqlite::params![noon3],
        ).unwrap();
        // Non-commit event on gap day — should be excluded
        conn.execute(
            "INSERT INTO git_activity (repo_path, event_type, branch, timestamp)
             VALUES ('repo1', 'branch_switch', 'feature', ?1)",
            rusqlite::params![gap_ts],
        ).unwrap();

        let from = local_midnight_utc(2025, 1, 10);
        let to = local_eod_utc(2025, 1, 12);

        let result = query_daily_commit_counts(&conn, from, to).unwrap();

        assert_eq!(result.len(), 3);
        assert_eq!(result[&day1], 2);
        assert_eq!(result[&NaiveDate::from_ymd_opt(2025, 1, 11).unwrap()], 0);
        assert_eq!(result[&day3], 1);
    }

    #[test]
    fn test_query_daily_commit_counts_empty() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let conn = open_db(&db_path).unwrap();

        let from = local_midnight_utc(2025, 1, 1);
        let to = local_eod_utc(2025, 1, 3);

        let result = query_daily_commit_counts(&conn, from, to).unwrap();

        assert_eq!(result.len(), 3);
        for (_, count) in &result {
            assert_eq!(*count, 0);
        }
    }

    #[test]
    fn test_heatmap_range_starts_on_monday() {
        let (start, end) = heatmap_range(52);
        let start_local = start.with_timezone(&Local).date_naive();
        assert_eq!(start_local.weekday(), chrono::Weekday::Mon);
        // Range spans at most weeks*7+6 days
        let span = (end - start).num_days();
        assert!(span <= 52 * 7 + 6, "span {span} exceeds max");
        assert!(span >= 7, "span {span} too short");
    }

    #[test]
    fn test_heatmap_range_one_week() {
        let (start, end) = heatmap_range(1);
        let start_local = start.with_timezone(&Local).date_naive();
        assert_eq!(start_local.weekday(), chrono::Weekday::Mon);
        let span = (end - start).num_days();
        // weeks=1 → 7-13 days depending on weekday
        assert!(span >= 7 && span <= 13, "span {span} out of range for weeks=1");
    }

    #[test]
    fn test_heatmap_range_end_is_today() {
        let (_, end) = heatmap_range(4);
        let end_local = end.with_timezone(&Local).date_naive();
        let today = Local::now().date_naive();
        assert_eq!(end_local, today);
    }

    #[test]
    fn test_ai_session_effective_end_closed() {
        let now = Utc::now();
        let ended = now - chrono::Duration::hours(1);
        let s = AiSessionInfo {
            tool: "claude-code".into(),
            session_id: "t".into(),
            started_at: now - chrono::Duration::hours(2),
            ended_at: Some(ended),
            last_active_at: None,
            duration: chrono::Duration::hours(1),
            turns: None,
            segments: Vec::new(),
        };
        assert_eq!(s.effective_end(), ended);
    }

    #[test]
    fn test_ai_session_effective_end_open_no_heartbeat() {
        let now = Utc::now();
        let s = AiSessionInfo {
            tool: "claude-code".into(),
            session_id: "t".into(),
            started_at: now - chrono::Duration::hours(2),
            ended_at: None,
            last_active_at: None,
            duration: chrono::Duration::hours(2),
            turns: None,
            segments: Vec::new(),
        };
        // No heartbeat → legacy behavior (now)
        let end = s.effective_end();
        assert!((end - now).num_seconds().abs() < 2);
    }

    #[test]
    fn test_ai_session_effective_end_idle_capped() {
        let now = Utc::now();
        let last_active = now - chrono::Duration::hours(8);
        let s = AiSessionInfo {
            tool: "claude-code".into(),
            session_id: "t".into(),
            started_at: now - chrono::Duration::hours(10),
            ended_at: None,
            last_active_at: Some(last_active),
            duration: chrono::Duration::zero(),
            turns: None,
            segments: Vec::new(),
        };
        let end = s.effective_end();
        let expected = last_active + chrono::Duration::minutes(AI_IDLE_CAP_MINUTES);
        // Should be capped at last_active + 30min, not now
        assert!((end - expected).num_seconds().abs() < 2,
            "expected ~{expected}, got {end}");
        // And definitely NOT close to now
        assert!((now - end).num_hours() >= 7, "should be hours ago, not now");
    }

    #[test]
    fn test_ai_session_effective_end_recent_activity() {
        let now = Utc::now();
        let last_active = now - chrono::Duration::minutes(5);
        let s = AiSessionInfo {
            tool: "claude-code".into(),
            session_id: "t".into(),
            started_at: now - chrono::Duration::hours(1),
            ended_at: None,
            last_active_at: Some(last_active),
            duration: chrono::Duration::zero(),
            turns: None,
            segments: Vec::new(),
        };
        let end = s.effective_end();
        // last_active + 30min is in the future → capped at now
        assert!((end - now).num_seconds().abs() < 2);
    }

    #[test]
    fn test_idle_ai_session_excluded_from_time_estimate() {
        // An AI session with last_active_at 8 hours ago should NOT contribute 8 hours
        let now = Utc::now();
        let session_start = now - chrono::Duration::hours(10);
        let last_active = now - chrono::Duration::hours(8);

        let ai_intervals: Vec<TimeInterval> = {
            let s = AiSessionInfo {
                tool: "claude-code".into(),
                session_id: "t".into(),
                started_at: session_start,
                ended_at: None,
                last_active_at: Some(last_active),
                duration: chrono::Duration::zero(),
                turns: None,
                segments: Vec::new(),
            };
            let end = s.effective_end();
            if session_start < end {
                vec![TimeInterval { start: session_start, end }]
            } else {
                vec![]
            }
        };

        let (est, _) = estimate_time_v2(&[], &ai_intervals, &[], 120, 30);
        // Should be ~2h30m (10h start to 8h-ago + 30min cap), not 10h
        assert!(est.num_hours() < 4, "expected <4h, got {}h{}m", est.num_hours(), est.num_minutes() % 60);
    }

    #[test]
    fn test_effective_start_same_day_session() {
        let now = Utc::now();
        let s = AiSessionInfo {
            tool: "claude-code".into(),
            session_id: "t".into(),
            started_at: now - chrono::Duration::hours(2),
            ended_at: None,
            last_active_at: Some(now - chrono::Duration::minutes(5)),
            duration: chrono::Duration::zero(),
            turns: None,
            segments: Vec::new(),
        };
        let window_start = now - chrono::Duration::hours(12); // midnight-ish
        // started_at is after window_start → use started_at
        assert_eq!(s.effective_start(window_start), s.started_at);
    }

    #[test]
    fn test_effective_start_cross_day_session() {
        let now = Utc::now();
        let last_active = now - chrono::Duration::minutes(5);
        let s = AiSessionInfo {
            tool: "claude-code".into(),
            session_id: "t".into(),
            started_at: now - chrono::Duration::days(5),
            ended_at: None,
            last_active_at: Some(last_active),
            duration: chrono::Duration::zero(),
            turns: None,
            segments: Vec::new(),
        };
        let window_start = now - chrono::Duration::hours(12);
        // started_at is before window → use last_active_at
        assert_eq!(s.effective_start(window_start), last_active);
    }

    #[test]
    fn test_effective_start_cross_day_no_heartbeat() {
        let now = Utc::now();
        let s = AiSessionInfo {
            tool: "claude-code".into(),
            session_id: "t".into(),
            started_at: now - chrono::Duration::days(5),
            ended_at: None,
            last_active_at: None,
            duration: chrono::Duration::zero(),
            turns: None,
            segments: Vec::new(),
        };
        let window_start = now - chrono::Duration::hours(12);
        // No heartbeat, started before window → clamp to window_start
        assert_eq!(s.effective_start(window_start), window_start);
    }

    #[test]
    fn test_cross_day_session_interval_not_inflated() {
        // Session started 5 days ago, last_active 10 min ago.
        // effective_end = min(last_active + 30min, now) = now (since +30 is future)
        // Interval = [last_active, now] = ~10 min. NOT midnight-to-now.
        let now = Utc::now();
        let last_active = now - chrono::Duration::minutes(10);
        let s = AiSessionInfo {
            tool: "claude-code".into(),
            session_id: "t".into(),
            started_at: now - chrono::Duration::days(5),
            ended_at: None,
            last_active_at: Some(last_active),
            duration: chrono::Duration::zero(),
            turns: None,
            segments: Vec::new(),
        };
        let window_start = now - chrono::Duration::hours(12);
        let start = s.effective_start(window_start);
        let end = s.effective_end();
        let duration = end - start;
        // Should be ~10min (now - last_active), not 12h
        assert!(duration.num_minutes() <= 15,
            "expected <=15min, got {}min", duration.num_minutes());
        // And definitely not hours
        assert!(duration.num_hours() < 1,
            "expected <1h, got {}h", duration.num_hours());
    }
}
