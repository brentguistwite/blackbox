use std::path::Path;
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use crate::enrichment::GhPrDetail;

pub fn open_db(path: &Path) -> anyhow::Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;

    let migrations = Migrations::new(vec![
        M::up(
            "CREATE TABLE IF NOT EXISTS git_activity (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_path TEXT NOT NULL,
                event_type TEXT NOT NULL CHECK(event_type IN ('commit','branch_switch','merge')),
                branch TEXT,
                commit_hash TEXT,
                author TEXT,
                message TEXT,
                timestamp TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_git_activity_repo_ts ON git_activity(repo_path, timestamp);"
        ),
        M::up("ALTER TABLE git_activity ADD COLUMN source_branch TEXT;"),
        M::up(
            "CREATE TABLE IF NOT EXISTS directory_presence (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_path TEXT NOT NULL,
                entered_at TEXT NOT NULL,
                left_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_dir_presence_repo ON directory_presence(repo_path, entered_at);"
        ),
        M::up(
            "CREATE TABLE IF NOT EXISTS review_activity (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_path TEXT NOT NULL,
                pr_number INTEGER NOT NULL,
                pr_title TEXT NOT NULL,
                pr_url TEXT NOT NULL,
                review_action TEXT NOT NULL CHECK(review_action IN ('APPROVED','CHANGES_REQUESTED','COMMENTED')),
                reviewed_at TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_review_activity_repo_ts ON review_activity(repo_path, reviewed_at);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_review_activity_dedup ON review_activity(repo_path, pr_number, reviewed_at);"
        ),
        M::up(
            "CREATE TABLE IF NOT EXISTS ai_sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tool TEXT NOT NULL DEFAULT 'claude-code',
                repo_path TEXT NOT NULL,
                session_id TEXT NOT NULL UNIQUE,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                turns INTEGER,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_ai_sessions_repo_ts ON ai_sessions(repo_path, started_at);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_ai_sessions_dedup ON ai_sessions(session_id);"
        ),
        // US-018b: dedup existing rows, add partial unique index on (repo_path, commit_hash)
        M::up(
            "DELETE FROM git_activity WHERE commit_hash IS NOT NULL AND id NOT IN (
                SELECT MIN(id) FROM git_activity WHERE commit_hash IS NOT NULL GROUP BY repo_path, commit_hash
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_activity_repo_commit ON git_activity(repo_path, commit_hash) WHERE commit_hash IS NOT NULL;"
        ),
        // US-001: pr_snapshots table for PR cycle time metrics
        M::up(
            "CREATE TABLE IF NOT EXISTS pr_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_path TEXT NOT NULL,
                pr_number INTEGER NOT NULL,
                title TEXT NOT NULL,
                url TEXT NOT NULL,
                state TEXT NOT NULL,
                head_ref TEXT NOT NULL,
                base_ref TEXT NOT NULL,
                author_login TEXT,
                created_at_gh TEXT,
                merged_at TEXT,
                closed_at TEXT,
                first_review_at TEXT,
                additions INTEGER,
                deletions INTEGER,
                commits INTEGER,
                iteration_count INTEGER,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_pr_snapshots_repo_pr ON pr_snapshots(repo_path, pr_number);
            CREATE INDEX IF NOT EXISTS idx_pr_snapshots_repo_state ON pr_snapshots(repo_path, state);"
        ),
        // US-001: per-commit per-file line stats for churn detection
        M::up(
            "CREATE TABLE IF NOT EXISTS commit_line_stats (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_path TEXT NOT NULL,
                commit_hash TEXT NOT NULL,
                file_path TEXT NOT NULL,
                lines_added INTEGER NOT NULL,
                lines_deleted INTEGER NOT NULL,
                committed_at TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_commit_line_stats_dedup ON commit_line_stats(repo_path, commit_hash, file_path);"
        ),
        // US-002: churn events — raw evidence for churn rate computation
        M::up(
            "CREATE TABLE IF NOT EXISTS churn_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_path TEXT NOT NULL,
                original_commit_hash TEXT NOT NULL,
                churn_commit_hash TEXT NOT NULL,
                file_path TEXT NOT NULL,
                lines_churned INTEGER NOT NULL,
                churn_window_days INTEGER NOT NULL,
                detected_at TEXT NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_churn_events_dedup ON churn_events(repo_path, original_commit_hash, churn_commit_hash, file_path);"
        ),
        // richer-status US-001: daemon_state key-value table for heartbeat data
        M::up("CREATE TABLE IF NOT EXISTS daemon_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);"),
        // os-notifications US-004: notification rate-limiting log
        M::up("CREATE TABLE IF NOT EXISTS notification_log (date TEXT NOT NULL, notification_type TEXT NOT NULL, sent_at TEXT NOT NULL, PRIMARY KEY (date, notification_type));"),
        // US-016-03: commit message quality scores
        M::up(
            "CREATE TABLE IF NOT EXISTS commit_quality (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_path TEXT NOT NULL,
                commit_hash TEXT NOT NULL,
                score INTEGER NOT NULL,
                is_vague INTEGER NOT NULL DEFAULT 0,
                scored_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_commit_quality_repo_hash
                ON commit_quality(repo_path, commit_hash);"
        ),
        // idle-cap: track last activity timestamp for AI sessions
        M::up("ALTER TABLE ai_sessions ADD COLUMN last_active_at TEXT;"),
    ]);
    migrations.to_latest(&mut conn)?;

    Ok(conn)
}

/// Record a directory change: close previous open entry, insert new one.
pub fn record_directory_presence(conn: &Connection, repo_path: &str) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();

    // Close any open entry for this repo (set left_at)
    conn.execute(
        "UPDATE directory_presence SET left_at = ?1 WHERE repo_path = ?2 AND left_at IS NULL",
        rusqlite::params![now, repo_path],
    )?;

    // Insert new entry
    conn.execute(
        "INSERT INTO directory_presence (repo_path, entered_at) VALUES (?1, ?2)",
        rusqlite::params![repo_path, now],
    )?;

    Ok(())
}

/// Insert a review activity record. Returns Ok(false) if duplicate (already exists).
pub fn insert_review(
    conn: &Connection,
    repo_path: &str,
    pr_number: i64,
    pr_title: &str,
    pr_url: &str,
    review_action: &str,
    reviewed_at: &str,
) -> anyhow::Result<bool> {
    match conn.execute(
        "INSERT OR IGNORE INTO review_activity (repo_path, pr_number, pr_title, pr_url, review_action, reviewed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![repo_path, pr_number, pr_title, pr_url, review_action, reviewed_at],
    ) {
        Ok(0) => Ok(false), // duplicate, ignored
        Ok(_) => Ok(true),  // inserted
        Err(e) => Err(e.into()),
    }
}

/// Insert a new AI session. Returns Ok(false) if duplicate (session_id already exists).
pub fn insert_ai_session(
    conn: &Connection,
    tool: &str,
    repo_path: &str,
    session_id: &str,
    started_at: &str,
) -> anyhow::Result<bool> {
    match conn.execute(
        "INSERT OR IGNORE INTO ai_sessions (tool, repo_path, session_id, started_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![tool, repo_path, session_id, started_at],
    ) {
        Ok(0) => Ok(false),
        Ok(_) => Ok(true),
        Err(e) => Err(e.into()),
    }
}

/// Mark a session as ended with ended_at timestamp and optional turn count.
pub fn update_session_ended(
    conn: &Connection,
    session_id: &str,
    ended_at: &str,
    turns: Option<i64>,
) -> anyhow::Result<bool> {
    let rows = conn.execute(
        "UPDATE ai_sessions SET ended_at = ?1, turns = ?2 WHERE session_id = ?3 AND ended_at IS NULL",
        rusqlite::params![ended_at, turns, session_id],
    )?;
    Ok(rows > 0)
}

/// Update last_active_at for an AI session (heartbeat from file mtime etc).
pub fn update_session_last_active(
    conn: &Connection,
    session_id: &str,
    last_active_at: &str,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE ai_sessions SET last_active_at = ?1 WHERE session_id = ?2",
        rusqlite::params![last_active_at, session_id],
    )?;
    Ok(())
}

/// Check if a session already exists (by session_id).
pub fn session_exists(conn: &Connection, session_id: &str) -> anyhow::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM ai_sessions WHERE session_id = ?1",
        rusqlite::params![session_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Get session IDs that are still active (no ended_at).
pub fn get_active_sessions(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT session_id FROM ai_sessions WHERE ended_at IS NULL")?;
    let ids = stmt.query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

/// Get active session IDs filtered by tool name.
pub fn get_active_sessions_by_tool(conn: &Connection, tool: &str) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT session_id FROM ai_sessions WHERE ended_at IS NULL AND tool = ?1",
    )?;
    let ids = stmt.query_map(rusqlite::params![tool], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

/// Get all active sessions as (session_id, tool) pairs.
pub fn get_active_sessions_all(conn: &Connection) -> anyhow::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, tool FROM ai_sessions WHERE ended_at IS NULL",
    )?;
    let pairs = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(pairs)
}

/// Insert a git activity record. Uses INSERT OR IGNORE for events with commit_hash
/// (commits, merges) to leverage the partial unique index. Branch switch events
/// (NULL commit_hash) use regular INSERT. Returns true if a row was inserted.
#[allow(clippy::too_many_arguments)]
pub fn insert_activity(
    conn: &Connection,
    repo_path: &str,
    event_type: &str,
    branch: Option<&str>,
    source_branch: Option<&str>,
    commit_hash: Option<&str>,
    author: Option<&str>,
    message: Option<&str>,
    timestamp: &str,
) -> anyhow::Result<bool> {
    let sql = if commit_hash.is_some() {
        "INSERT OR IGNORE INTO git_activity (repo_path, event_type, branch, source_branch, commit_hash, author, message, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
    } else {
        "INSERT INTO git_activity (repo_path, event_type, branch, source_branch, commit_hash, author, message, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
    };
    let rows = conn.execute(
        sql,
        rusqlite::params![repo_path, event_type, branch, source_branch, commit_hash, author, message, timestamp],
    )?;
    Ok(rows > 0)
}

/// Upsert a PR snapshot row. Computes first_review_at and iteration_count from reviews vec.
/// Uses INSERT OR REPLACE — the unique index on (repo_path, pr_number) triggers replacement.
pub fn upsert_pr_snapshot(
    conn: &Connection,
    repo_path: &str,
    pr: &GhPrDetail,
) -> anyhow::Result<()> {
    // Earliest non-PENDING review
    let first_review_at: Option<String> = pr
        .reviews
        .iter()
        .filter(|r| r.state != "PENDING")
        .map(|r| r.submitted_at.clone())
        .min();

    // Number of CHANGES_REQUESTED reviews
    let iteration_count: i64 = pr
        .reviews
        .iter()
        .filter(|r| r.state == "CHANGES_REQUESTED")
        .count() as i64;

    conn.execute(
        "INSERT OR REPLACE INTO pr_snapshots
         (repo_path, pr_number, title, url, state, head_ref, base_ref,
          author_login, created_at_gh, merged_at, closed_at, first_review_at,
          additions, deletions, commits, iteration_count)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        rusqlite::params![
            repo_path,
            pr.number as i64,
            &pr.title,
            &pr.url,
            &pr.state,
            &pr.head_ref_name,
            &pr.base_ref_name,
            pr.author.as_ref().map(|a| &a.login),
            &pr.created_at,
            &pr.merged_at,
            &pr.closed_at,
            first_review_at,
            pr.additions,
            pr.deletions,
            pr.commits.len() as i64,
            iteration_count,
        ],
    )?;
    Ok(())
}

/// Insert a churn event. Returns true if inserted, false if duplicate.
#[allow(clippy::too_many_arguments)]
pub fn insert_churn_event(
    conn: &Connection,
    repo_path: &str,
    original_commit_hash: &str,
    churn_commit_hash: &str,
    file_path: &str,
    lines_churned: i64,
    churn_window_days: i64,
    detected_at: &str,
) -> anyhow::Result<bool> {
    match conn.execute(
        "INSERT OR IGNORE INTO churn_events (repo_path, original_commit_hash, churn_commit_hash, file_path, lines_churned, churn_window_days, detected_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![repo_path, original_commit_hash, churn_commit_hash, file_path, lines_churned, churn_window_days, detected_at],
    ) {
        Ok(0) => Ok(false),
        Ok(_) => Ok(true),
        Err(e) => Err(e.into()),
    }
}

/// Per-file line stats from a single commit.
#[derive(Debug, Clone)]
pub struct CommitLineStat {
    pub repo_path: String,
    pub commit_hash: String,
    pub file_path: String,
    pub lines_added: i64,
    pub lines_deleted: i64,
    pub committed_at: chrono::DateTime<chrono::Utc>,
}

/// Insert per-file line stats for a commit. Returns true if inserted, false if duplicate.
pub fn insert_commit_line_stats(
    conn: &Connection,
    repo_path: &str,
    commit_hash: &str,
    file_path: &str,
    lines_added: i64,
    lines_deleted: i64,
    committed_at: &str,
) -> anyhow::Result<bool> {
    match conn.execute(
        "INSERT OR IGNORE INTO commit_line_stats (repo_path, commit_hash, file_path, lines_added, lines_deleted, committed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![repo_path, commit_hash, file_path, lines_added, lines_deleted, committed_at],
    ) {
        Ok(0) => Ok(false),
        Ok(_) => Ok(true),
        Err(e) => Err(e.into()),
    }
}

/// Query commit_line_stats for a repo since a given time, ordered by committed_at ASC.
pub fn query_commit_line_stats_for_repo(
    conn: &Connection,
    repo_path: &str,
    since: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Vec<CommitLineStat>> {
    let since_str = since.to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT repo_path, commit_hash, file_path, lines_added, lines_deleted, committed_at
         FROM commit_line_stats
         WHERE repo_path = ?1 AND committed_at >= ?2
         ORDER BY committed_at ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![repo_path, since_str], |row| {
        let committed_at_str: String = row.get(5)?;
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            committed_at_str,
        ))
    })?;
    let mut stats = Vec::new();
    for r in rows {
        let (repo_path, commit_hash, file_path, lines_added, lines_deleted, committed_at_str) = r?;
        let committed_at = chrono::DateTime::parse_from_rfc3339(&committed_at_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());
        stats.push(CommitLineStat {
            repo_path,
            commit_hash,
            file_path,
            lines_added,
            lines_deleted,
            committed_at,
        });
    }
    Ok(stats)
}

/// Return distinct repo_paths that have commit_line_stats data.
pub fn repos_with_line_stats(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT repo_path FROM commit_line_stats ORDER BY repo_path",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut paths = Vec::new();
    for r in rows {
        paths.push(r?);
    }
    Ok(paths)
}

/// Set a key-value pair in the daemon_state table (INSERT OR REPLACE).
pub fn set_daemon_state(conn: &Connection, key: &str, value: &str) -> anyhow::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO daemon_state (key, value) VALUES (?1, ?2)",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Get a value from the daemon_state table. Returns None if key absent.
pub fn get_daemon_state(conn: &Connection, key: &str) -> anyhow::Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM daemon_state WHERE key = ?1")?;
    let mut rows = stmt.query(rusqlite::params![key])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

/// Check if a notification was already sent for a given date and type.
pub fn notification_was_sent(
    conn: &Connection,
    date: &str,
    notification_type: &str,
) -> anyhow::Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM notification_log WHERE date = ?1 AND notification_type = ?2)",
        rusqlite::params![date, notification_type],
        |row| row.get(0),
    )?;
    Ok(exists)
}

/// Record that a notification was sent. Idempotent via INSERT OR IGNORE.
pub fn record_notification_sent(
    conn: &Connection,
    date: &str,
    notification_type: &str,
) -> anyhow::Result<()> {
    let sent_at = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO notification_log (date, notification_type, sent_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![date, notification_type, sent_at],
    )?;
    Ok(())
}

/// Insert a commit quality score. Returns Ok(false) if duplicate (already scored).
pub fn insert_commit_quality(
    conn: &Connection,
    repo_path: &str,
    commit_hash: &str,
    score: u8,
    is_vague: bool,
) -> anyhow::Result<bool> {
    match conn.execute(
        "INSERT OR IGNORE INTO commit_quality (repo_path, commit_hash, score, is_vague)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![repo_path, commit_hash, score as i64, is_vague as i64],
    ) {
        Ok(0) => Ok(false),
        Ok(_) => Ok(true),
        Err(e) => Err(e.into()),
    }
}

/// Count git_activity events from today (UTC midnight onwards).
pub fn count_events_today(conn: &Connection) -> anyhow::Result<u64> {
    let today_start = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .to_rfc3339();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM git_activity WHERE timestamp >= ?1",
        rusqlite::params![today_start],
        |row| row.get(0),
    )?;
    Ok(count as u64)
}
