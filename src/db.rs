use std::path::Path;
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};

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
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO git_activity (repo_path, event_type, branch, source_branch, commit_hash, author, message, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![repo_path, event_type, branch, source_branch, commit_hash, author, message, timestamp],
    )?;
    Ok(())
}
