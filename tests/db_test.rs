use blackbox::db;
use chrono::Utc;
use rusqlite::Connection;
use tempfile::TempDir;

#[test]
fn test_open_db_creates_file() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    assert!(!db_path.exists());
    let _conn = db::open_db(&db_path).unwrap();
    assert!(db_path.exists());
}

#[test]
fn test_wal_mode_enabled() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();
    let mode: String = conn
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

#[test]
fn test_busy_timeout_set() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();
    let timeout: i64 = conn
        .pragma_query_value(None, "busy_timeout", |row| row.get(0))
        .unwrap();
    assert_eq!(timeout, 5000);
}

#[test]
fn test_git_activity_table_exists() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='git_activity'")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(tables, vec!["git_activity"]);

    // Verify expected columns exist
    let mut stmt = conn.prepare("PRAGMA table_info(git_activity)").unwrap();
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(columns.contains(&"repo_path".to_string()));
    assert!(columns.contains(&"event_type".to_string()));
    assert!(columns.contains(&"branch".to_string()));
    assert!(columns.contains(&"commit_hash".to_string()));
    assert!(columns.contains(&"author".to_string()));
    assert!(columns.contains(&"message".to_string()));
    assert!(columns.contains(&"timestamp".to_string()));
    assert!(columns.contains(&"created_at".to_string()));
}

#[test]
fn test_migrations_idempotent() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let _conn1 = db::open_db(&db_path).unwrap();
    drop(_conn1);
    // Second open should not error (no duplicate table)
    let _conn2 = db::open_db(&db_path).unwrap();
}

#[test]
fn test_source_branch_column_exists() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let mut stmt = conn.prepare("PRAGMA table_info(git_activity)").unwrap();
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(columns.contains(&"source_branch".to_string()));
}

#[test]
fn test_insert_activity_commit() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    db::insert_activity(
        &conn,
        "/tmp/repo",
        "commit",
        Some("main"),
        None,
        Some("abc123"),
        Some("Alice"),
        Some("fix bug"),
        "2026-03-14T12:00:00Z",
    )
    .unwrap();

    let (repo, etype, branch, hash, author, msg): (String, String, String, String, String, String) = conn
        .query_row(
            "SELECT repo_path, event_type, branch, commit_hash, author, message FROM git_activity WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .unwrap();

    assert_eq!(repo, "/tmp/repo");
    assert_eq!(etype, "commit");
    assert_eq!(branch, "main");
    assert_eq!(hash, "abc123");
    assert_eq!(author, "Alice");
    assert_eq!(msg, "fix bug");
}

#[test]
fn test_insert_activity_merge_with_source_branch() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    db::insert_activity(
        &conn,
        "/tmp/repo",
        "merge",
        Some("main"),
        Some("feature-x"),
        Some("def456"),
        Some("Bob"),
        Some("Merge feature-x into main"),
        "2026-03-14T13:00:00Z",
    )
    .unwrap();

    let source: String = conn
        .query_row(
            "SELECT source_branch FROM git_activity WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(source, "feature-x");
}

#[test]
fn test_review_activity_table_exists() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='review_activity'")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(tables, vec!["review_activity"]);

    let mut stmt = conn.prepare("PRAGMA table_info(review_activity)").unwrap();
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(columns.contains(&"repo_path".to_string()));
    assert!(columns.contains(&"pr_number".to_string()));
    assert!(columns.contains(&"pr_title".to_string()));
    assert!(columns.contains(&"pr_url".to_string()));
    assert!(columns.contains(&"review_action".to_string()));
    assert!(columns.contains(&"reviewed_at".to_string()));
    assert!(columns.contains(&"created_at".to_string()));
}

#[test]
fn test_insert_review() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let inserted = db::insert_review(
        &conn,
        "/tmp/repo",
        42,
        "Add feature X",
        "https://github.com/org/repo/pull/42",
        "APPROVED",
        "2026-03-15T10:00:00Z",
    )
    .unwrap();
    assert!(inserted);

    let (repo, pr_num, title, url, action, reviewed_at): (String, i64, String, String, String, String) = conn
        .query_row(
            "SELECT repo_path, pr_number, pr_title, pr_url, review_action, reviewed_at FROM review_activity WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .unwrap();

    assert_eq!(repo, "/tmp/repo");
    assert_eq!(pr_num, 42);
    assert_eq!(title, "Add feature X");
    assert_eq!(url, "https://github.com/org/repo/pull/42");
    assert_eq!(action, "APPROVED");
    assert_eq!(reviewed_at, "2026-03-15T10:00:00Z");
}

#[test]
fn test_insert_review_dedup() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    // First insert succeeds
    let first = db::insert_review(
        &conn, "/tmp/repo", 42, "Title", "http://url", "APPROVED", "2026-03-15T10:00:00Z",
    ).unwrap();
    assert!(first);

    // Duplicate returns false (same repo_path + pr_number + reviewed_at)
    let second = db::insert_review(
        &conn, "/tmp/repo", 42, "Title", "http://url", "APPROVED", "2026-03-15T10:00:00Z",
    ).unwrap();
    assert!(!second);

    // Different reviewed_at = not a duplicate
    let third = db::insert_review(
        &conn, "/tmp/repo", 42, "Title", "http://url", "CHANGES_REQUESTED", "2026-03-15T11:00:00Z",
    ).unwrap();
    assert!(third);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM review_activity", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn test_ai_sessions_table_exists() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='ai_sessions'")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(tables, vec!["ai_sessions"]);

    let mut stmt = conn.prepare("PRAGMA table_info(ai_sessions)").unwrap();
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(columns.contains(&"tool".to_string()));
    assert!(columns.contains(&"repo_path".to_string()));
    assert!(columns.contains(&"session_id".to_string()));
    assert!(columns.contains(&"started_at".to_string()));
    assert!(columns.contains(&"ended_at".to_string()));
    assert!(columns.contains(&"turns".to_string()));
    assert!(columns.contains(&"created_at".to_string()));
}

#[test]
fn test_insert_ai_session() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let inserted = db::insert_ai_session(
        &conn,
        "/tmp/repo",
        "session-abc-123",
        "2026-03-16T10:00:00Z",
    )
    .unwrap();
    assert!(inserted);

    let (repo, sid, tool): (String, String, String) = conn
        .query_row(
            "SELECT repo_path, session_id, tool FROM ai_sessions WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(repo, "/tmp/repo");
    assert_eq!(sid, "session-abc-123");
    assert_eq!(tool, "claude-code");
}

#[test]
fn test_insert_ai_session_dedup() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let first = db::insert_ai_session(&conn, "/tmp/repo", "dup-session", "2026-03-16T10:00:00Z").unwrap();
    assert!(first);

    let second = db::insert_ai_session(&conn, "/tmp/repo", "dup-session", "2026-03-16T10:00:00Z").unwrap();
    assert!(!second);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_update_session_ended() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    db::insert_ai_session(&conn, "/tmp/repo", "end-session", "2026-03-16T10:00:00Z").unwrap();

    let updated = db::update_session_ended(&conn, "end-session", "2026-03-16T11:00:00Z", Some(42)).unwrap();
    assert!(updated);

    let (ended, turns): (String, i64) = conn
        .query_row(
            "SELECT ended_at, turns FROM ai_sessions WHERE session_id = 'end-session'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(ended, "2026-03-16T11:00:00Z");
    assert_eq!(turns, 42);

    // Second update should be no-op (already ended)
    let second = db::update_session_ended(&conn, "end-session", "2026-03-16T12:00:00Z", Some(50)).unwrap();
    assert!(!second);
}

#[test]
fn test_get_active_sessions() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    db::insert_ai_session(&conn, "/tmp/repo", "active-1", "2026-03-16T10:00:00Z").unwrap();
    db::insert_ai_session(&conn, "/tmp/repo", "active-2", "2026-03-16T10:00:00Z").unwrap();
    db::insert_ai_session(&conn, "/tmp/repo", "ended-1", "2026-03-16T10:00:00Z").unwrap();
    db::update_session_ended(&conn, "ended-1", "2026-03-16T11:00:00Z", None).unwrap();

    let active = db::get_active_sessions(&conn).unwrap();
    assert_eq!(active.len(), 2);
    assert!(active.contains(&"active-1".to_string()));
    assert!(active.contains(&"active-2".to_string()));
}

#[test]
fn test_activity_dedup_unique_index_exists() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let idx: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_activity_repo_commit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(idx.contains("UNIQUE"));
    assert!(idx.contains("commit_hash IS NOT NULL"));
}

#[test]
fn test_insert_activity_commit_dedup_same_repo() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let first = db::insert_activity(
        &conn, "/tmp/repo", "commit", Some("main"), None,
        Some("abc123"), Some("Alice"), Some("fix bug"), "2026-03-18T10:00:00Z",
    ).unwrap();
    assert!(first);

    // Same repo + same commit_hash → ignored
    let second = db::insert_activity(
        &conn, "/tmp/repo", "commit", Some("main"), None,
        Some("abc123"), Some("Alice"), Some("fix bug"), "2026-03-18T10:00:00Z",
    ).unwrap();
    assert!(!second);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_activity WHERE commit_hash = 'abc123'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_insert_activity_commit_different_repos_both_kept() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    db::insert_activity(
        &conn, "/repo/a", "commit", Some("main"), None,
        Some("abc123"), Some("Alice"), Some("fix"), "2026-03-18T10:00:00Z",
    ).unwrap();

    db::insert_activity(
        &conn, "/repo/b", "commit", Some("main"), None,
        Some("abc123"), Some("Alice"), Some("fix"), "2026-03-18T10:00:00Z",
    ).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_activity WHERE commit_hash = 'abc123'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn test_branch_switch_never_blocked() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    // Multiple branch_switch events (NULL commit_hash) should all insert
    for _ in 0..3 {
        let inserted = db::insert_activity(
            &conn, "/tmp/repo", "branch_switch", Some("develop"), None,
            None, None, None, "2026-03-18T10:00:00Z",
        ).unwrap();
        assert!(inserted);
    }

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM git_activity WHERE event_type = 'branch_switch'",
            [], |r| r.get(0),
        ).unwrap();
    assert_eq!(count, 3);
}

#[test]
fn test_migration_dedup_existing_rows() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");

    // Create DB with only the first 5 migrations (before dedup migration)
    {
        let mut conn = Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        let migrations = rusqlite_migration::Migrations::new(vec![
            rusqlite_migration::M::up(
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
            rusqlite_migration::M::up("ALTER TABLE git_activity ADD COLUMN source_branch TEXT;"),
            rusqlite_migration::M::up(
                "CREATE TABLE IF NOT EXISTS directory_presence (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    repo_path TEXT NOT NULL,
                    entered_at TEXT NOT NULL,
                    left_at TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_dir_presence_repo ON directory_presence(repo_path, entered_at);"
            ),
            rusqlite_migration::M::up(
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
            rusqlite_migration::M::up(
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
        ]);
        migrations.to_latest(&mut conn).unwrap();

        // Insert duplicate rows (same repo_path + commit_hash)
        conn.execute(
            "INSERT INTO git_activity (repo_path, event_type, branch, commit_hash, author, message, timestamp)
             VALUES ('/repo', 'commit', 'main', 'aaa', 'dev', 'msg', '2026-01-01T00:00:00Z')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO git_activity (repo_path, event_type, branch, commit_hash, author, message, timestamp)
             VALUES ('/repo', 'commit', 'main', 'aaa', 'dev', 'msg', '2026-01-01T00:00:00Z')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO git_activity (repo_path, event_type, branch, commit_hash, author, message, timestamp)
             VALUES ('/repo', 'commit', 'main', 'bbb', 'dev', 'msg2', '2026-01-01T01:00:00Z')",
            [],
        ).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM git_activity", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3); // 2 dupes of aaa + 1 bbb
    }

    // Now open with full migrations (includes dedup)
    let conn = db::open_db(&db_path).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_activity", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2); // dedup removed 1 duplicate aaa, kept bbb
}

#[test]
fn test_migration_idempotent_with_dedup() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let _conn1 = db::open_db(&db_path).unwrap();
    drop(_conn1);
    // Second open should not error (dedup migration + index creation are safe)
    let _conn2 = db::open_db(&db_path).unwrap();
}

#[test]
fn test_insert_activity_branch_switch() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    db::insert_activity(
        &conn,
        "/tmp/repo",
        "branch_switch",
        Some("develop"),
        None,
        None,
        None,
        None,
        "2026-03-14T14:00:00Z",
    )
    .unwrap();

    let etype: String = conn
        .query_row(
            "SELECT event_type FROM git_activity WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(etype, "branch_switch");
}

#[test]
fn test_commit_line_stats_table_exists() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='commit_line_stats'")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(tables, vec!["commit_line_stats"]);

    let mut stmt = conn.prepare("PRAGMA table_info(commit_line_stats)").unwrap();
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(columns.contains(&"id".to_string()));
    assert!(columns.contains(&"repo_path".to_string()));
    assert!(columns.contains(&"commit_hash".to_string()));
    assert!(columns.contains(&"file_path".to_string()));
    assert!(columns.contains(&"lines_added".to_string()));
    assert!(columns.contains(&"lines_deleted".to_string()));
    assert!(columns.contains(&"committed_at".to_string()));
    assert!(columns.contains(&"created_at".to_string()));
}

#[test]
fn test_commit_line_stats_unique_index() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let idx: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_commit_line_stats_dedup'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(idx.contains("UNIQUE"));
    assert!(idx.contains("repo_path"));
    assert!(idx.contains("commit_hash"));
    assert!(idx.contains("file_path"));
}

#[test]
fn test_commit_line_stats_insert_or_ignore() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    // First insert succeeds
    conn.execute(
        "INSERT INTO commit_line_stats (repo_path, commit_hash, file_path, lines_added, lines_deleted, committed_at)
         VALUES ('/repo', 'abc123', 'src/main.rs', 10, 5, '2026-03-20T10:00:00Z')",
        [],
    ).unwrap();

    // Duplicate insert via OR IGNORE should not error
    conn.execute(
        "INSERT OR IGNORE INTO commit_line_stats (repo_path, commit_hash, file_path, lines_added, lines_deleted, committed_at)
         VALUES ('/repo', 'abc123', 'src/main.rs', 20, 15, '2026-03-20T10:00:00Z')",
        [],
    ).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM commit_line_stats", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);

    // Original values preserved (not replaced)
    let added: i64 = conn
        .query_row(
            "SELECT lines_added FROM commit_line_stats WHERE commit_hash = 'abc123'",
            [], |r| r.get(0),
        ).unwrap();
    assert_eq!(added, 10);
}

#[test]
fn test_churn_events_table_exists() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='churn_events'")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(tables, vec!["churn_events"]);

    let mut stmt = conn.prepare("PRAGMA table_info(churn_events)").unwrap();
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(columns.contains(&"id".to_string()));
    assert!(columns.contains(&"repo_path".to_string()));
    assert!(columns.contains(&"original_commit_hash".to_string()));
    assert!(columns.contains(&"churn_commit_hash".to_string()));
    assert!(columns.contains(&"file_path".to_string()));
    assert!(columns.contains(&"lines_churned".to_string()));
    assert!(columns.contains(&"churn_window_days".to_string()));
    assert!(columns.contains(&"detected_at".to_string()));
}

#[test]
fn test_churn_events_unique_index() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let idx: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_churn_events_dedup'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(idx.contains("UNIQUE"));
    assert!(idx.contains("repo_path"));
    assert!(idx.contains("original_commit_hash"));
    assert!(idx.contains("churn_commit_hash"));
    assert!(idx.contains("file_path"));
}

#[test]
fn test_churn_events_insert_or_ignore() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    conn.execute(
        "INSERT INTO churn_events (repo_path, original_commit_hash, churn_commit_hash, file_path, lines_churned, churn_window_days, detected_at)
         VALUES ('/repo', 'aaa', 'bbb', 'src/main.rs', 5, 14, '2026-03-20T10:00:00Z')",
        [],
    ).unwrap();

    conn.execute(
        "INSERT OR IGNORE INTO churn_events (repo_path, original_commit_hash, churn_commit_hash, file_path, lines_churned, churn_window_days, detected_at)
         VALUES ('/repo', 'aaa', 'bbb', 'src/main.rs', 99, 14, '2026-03-20T12:00:00Z')",
        [],
    ).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM churn_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);

    let churned: i64 = conn
        .query_row(
            "SELECT lines_churned FROM churn_events WHERE original_commit_hash = 'aaa'",
            [], |r| r.get(0),
        ).unwrap();
    assert_eq!(churned, 5);
}

#[test]
fn test_insert_commit_line_stats() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let inserted = db::insert_commit_line_stats(
        &conn, "/repo", "abc123", "src/main.rs", 10, 5, "2026-03-20T10:00:00Z",
    ).unwrap();
    assert!(inserted);

    let (rp, ch, fp, la, ld): (String, String, String, i64, i64) = conn
        .query_row(
            "SELECT repo_path, commit_hash, file_path, lines_added, lines_deleted FROM commit_line_stats WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).unwrap();
    assert_eq!(rp, "/repo");
    assert_eq!(ch, "abc123");
    assert_eq!(fp, "src/main.rs");
    assert_eq!(la, 10);
    assert_eq!(ld, 5);
}

#[test]
fn test_insert_commit_line_stats_dedup() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let first = db::insert_commit_line_stats(
        &conn, "/repo", "abc123", "src/main.rs", 10, 5, "2026-03-20T10:00:00Z",
    ).unwrap();
    assert!(first);

    // Same commit+file → ignored, returns false
    let second = db::insert_commit_line_stats(
        &conn, "/repo", "abc123", "src/main.rs", 99, 99, "2026-03-20T10:00:00Z",
    ).unwrap();
    assert!(!second);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM commit_line_stats", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_query_commit_line_stats_for_repo() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    // Insert 3 rows with different timestamps
    db::insert_commit_line_stats(&conn, "/repo", "ccc", "f.rs", 3, 0, "2026-03-20T12:00:00Z").unwrap();
    db::insert_commit_line_stats(&conn, "/repo", "aaa", "f.rs", 1, 0, "2026-03-20T10:00:00Z").unwrap();
    db::insert_commit_line_stats(&conn, "/repo", "bbb", "f.rs", 2, 0, "2026-03-20T11:00:00Z").unwrap();
    // Different repo — should not appear
    db::insert_commit_line_stats(&conn, "/other", "ddd", "f.rs", 9, 9, "2026-03-20T10:00:00Z").unwrap();

    let since = chrono::DateTime::parse_from_rfc3339("2026-03-20T00:00:00Z").unwrap().with_timezone(&Utc);
    let stats = db::query_commit_line_stats_for_repo(&conn, "/repo", since).unwrap();

    assert_eq!(stats.len(), 3);
    // Must be ASC by committed_at
    assert_eq!(stats[0].commit_hash, "aaa");
    assert_eq!(stats[1].commit_hash, "bbb");
    assert_eq!(stats[2].commit_hash, "ccc");
    assert_eq!(stats[0].lines_added, 1);
    assert_eq!(stats[1].lines_added, 2);
    assert_eq!(stats[2].lines_added, 3);
}

#[test]
fn test_query_commit_line_stats_since_filter() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    db::insert_commit_line_stats(&conn, "/repo", "old", "f.rs", 5, 0, "2026-03-01T10:00:00Z").unwrap();
    db::insert_commit_line_stats(&conn, "/repo", "new", "f.rs", 7, 0, "2026-03-20T10:00:00Z").unwrap();

    let since = chrono::DateTime::parse_from_rfc3339("2026-03-15T00:00:00Z").unwrap().with_timezone(&Utc);
    let stats = db::query_commit_line_stats_for_repo(&conn, "/repo", since).unwrap();

    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].commit_hash, "new");
}
