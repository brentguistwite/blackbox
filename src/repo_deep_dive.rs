use std::path::{Path, PathBuf};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use crate::query::{ActivityEvent, AiSessionInfo, ReviewInfo};

/// Canonicalize path, verify it's a git repo.
pub fn resolve_repo_path(input: &str) -> anyhow::Result<PathBuf> {
    let expanded = if input.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(input.replacen('~', &home, 1))
    } else {
        PathBuf::from(input)
    };

    let canonical = std::fs::canonicalize(&expanded)
        .map_err(|_| anyhow::anyhow!("Path not found: {input}"))?;

    git2::Repository::open(&canonical)
        .map_err(|_| anyhow::anyhow!("Not a git repository: {}", canonical.display()))?;

    Ok(canonical)
}

#[derive(Debug)]
pub struct RepoAllTimeData {
    pub repo_path: String,
    pub events: Vec<ActivityEvent>,
    pub reviews: Vec<ReviewInfo>,
    pub ai_sessions: Vec<AiSessionInfo>,
}

/// Query all git_activity, review_activity, ai_sessions for a repo (no time filter).
pub fn query_repo_all_time(conn: &Connection, repo_path: &str) -> anyhow::Result<RepoAllTimeData> {
    // git_activity
    let mut stmt = conn.prepare(
        "SELECT event_type, branch, commit_hash, message, timestamp
         FROM git_activity WHERE repo_path = ?1
         ORDER BY timestamp ASC",
    )?;
    let events: Vec<ActivityEvent> = stmt
        .query_map(rusqlite::params![repo_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(event_type, branch, commit_hash, message, ts_str)| {
            let timestamp = DateTime::parse_from_rfc3339(&ts_str).ok()?.with_timezone(&Utc);
            Some(ActivityEvent { event_type, branch, commit_hash, message, timestamp })
        })
        .collect();

    // review_activity
    let mut stmt = conn.prepare(
        "SELECT pr_number, pr_title, review_action, reviewed_at
         FROM review_activity WHERE repo_path = ?1
         ORDER BY reviewed_at ASC",
    )?;
    let reviews: Vec<ReviewInfo> = stmt
        .query_map(rusqlite::params![repo_path], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(pr_number, pr_title, action, reviewed_at_str)| {
            let reviewed_at = DateTime::parse_from_rfc3339(&reviewed_at_str).ok()?.with_timezone(&Utc);
            Some(ReviewInfo { pr_number, pr_title, action, reviewed_at })
        })
        .collect();

    // ai_sessions
    let now = Utc::now();
    let mut stmt = conn.prepare(
        "SELECT session_id, started_at, ended_at, turns
         FROM ai_sessions WHERE repo_path = ?1
         ORDER BY started_at ASC",
    )?;
    let ai_sessions: Vec<AiSessionInfo> = stmt
        .query_map(rusqlite::params![repo_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(session_id, started_str, ended_str, turns)| {
            let started_at = DateTime::parse_from_rfc3339(&started_str).ok()?.with_timezone(&Utc);
            let ended_at = ended_str.as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc));
            let end = ended_at.unwrap_or(now);
            let duration = end - started_at;
            Some(AiSessionInfo { session_id, started_at, ended_at, duration, turns })
        })
        .collect();

    Ok(RepoAllTimeData {
        repo_path: repo_path.to_string(),
        events,
        reviews,
        ai_sessions,
    })
}

/// Look up repo_path in DB via exact or prefix match.
pub fn find_db_repo_path(conn: &Connection, canonical: &Path) -> anyhow::Result<Option<String>> {
    let path_str = canonical.to_string_lossy();
    let result: rusqlite::Result<String> = conn.query_row(
        "SELECT DISTINCT repo_path FROM git_activity WHERE repo_path = ?1 OR repo_path LIKE ?1 || '/%' LIMIT 1",
        rusqlite::params![path_str],
        |row| row.get(0),
    );
    match result {
        Ok(s) => Ok(Some(s)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
