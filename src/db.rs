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
    ]);
    migrations.to_latest(&mut conn)?;

    Ok(conn)
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
