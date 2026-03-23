use std::path::Path;
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};

/// A file that was modified multiple times in a given period (code churn).
#[derive(Debug)]
pub struct ChurnEntry {
    pub file_path: String,
    pub change_count: i64,
    pub repo_path: String,
}

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
        // US-016: file_changes table for code churn tracking
        M::up(
            "CREATE TABLE IF NOT EXISTS file_changes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_path TEXT NOT NULL,
                commit_hash TEXT NOT NULL,
                file_path TEXT NOT NULL,
                lines_added INTEGER NOT NULL DEFAULT 0,
                lines_removed INTEGER NOT NULL DEFAULT 0,
                timestamp TEXT NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_file_changes_dedup ON file_changes(repo_path, commit_hash, file_path);"
        ),
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
    repo_path: &str,
    session_id: &str,
    started_at: &str,
) -> anyhow::Result<bool> {
    match conn.execute(
        "INSERT OR IGNORE INTO ai_sessions (repo_path, session_id, started_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![repo_path, session_id, started_at],
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

/// Insert a file change record. Returns true if inserted, false if duplicate.
pub fn insert_file_change(
    conn: &Connection,
    repo_path: &str,
    commit_hash: &str,
    file_path: &str,
    timestamp: &str,
) -> anyhow::Result<bool> {
    match conn.execute(
        "INSERT OR IGNORE INTO file_changes (repo_path, commit_hash, file_path, timestamp)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![repo_path, commit_hash, file_path, timestamp],
    ) {
        Ok(0) => Ok(false),
        Ok(_) => Ok(true),
        Err(e) => Err(e.into()),
    }
}

/// Insert a git activity record. Uses INSERT OR IGNORE for events with commit_hash
/// (commits, merges) to leverage the partial unique index. Branch switch events
/// (NULL commit_hash) use regular INSERT. Returns true if a row was inserted.
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

/// Query files with high churn (modified >= threshold times) in a time range.
/// Returns up to 20 results ordered by change count descending.
pub fn query_churn(
    conn: &Connection,
    from: &str,
    to: &str,
    threshold: i64,
) -> anyhow::Result<Vec<ChurnEntry>> {
    let mut stmt = conn.prepare(
        "SELECT file_path, repo_path, COUNT(*) as change_count
         FROM file_changes
         WHERE timestamp >= ?1 AND timestamp < ?2
         GROUP BY repo_path, file_path
         HAVING COUNT(*) >= ?3
         ORDER BY change_count DESC
         LIMIT 20"
    )?;
    let entries = stmt.query_map(rusqlite::params![from, to, threshold], |row| {
        Ok(ChurnEntry {
            file_path: row.get(0)?,
            repo_path: row.get(1)?,
            change_count: row.get(2)?,
        })
    })?
    .filter_map(|r| r.ok())
    .collect();
    Ok(entries)
}
