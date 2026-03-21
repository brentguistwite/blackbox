use blackbox::db;
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
        &conn,
        "/tmp/repo",
        42,
        "Title",
        "http://url",
        "APPROVED",
        "2026-03-15T10:00:00Z",
    )
    .unwrap();
    assert!(first);

    // Duplicate returns false (same repo_path + pr_number + reviewed_at)
    let second = db::insert_review(
        &conn,
        "/tmp/repo",
        42,
        "Title",
        "http://url",
        "APPROVED",
        "2026-03-15T10:00:00Z",
    )
    .unwrap();
    assert!(!second);

    // Different reviewed_at = not a duplicate
    let third = db::insert_review(
        &conn,
        "/tmp/repo",
        42,
        "Title",
        "http://url",
        "CHANGES_REQUESTED",
        "2026-03-15T11:00:00Z",
    )
    .unwrap();
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

    let first =
        db::insert_ai_session(&conn, "/tmp/repo", "dup-session", "2026-03-16T10:00:00Z").unwrap();
    assert!(first);

    let second =
        db::insert_ai_session(&conn, "/tmp/repo", "dup-session", "2026-03-16T10:00:00Z").unwrap();
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

    let updated =
        db::update_session_ended(&conn, "end-session", "2026-03-16T11:00:00Z", Some(42)).unwrap();
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
    let second =
        db::update_session_ended(&conn, "end-session", "2026-03-16T12:00:00Z", Some(50)).unwrap();
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
        &conn,
        "/tmp/repo",
        "commit",
        Some("main"),
        None,
        Some("abc123"),
        Some("Alice"),
        Some("fix bug"),
        "2026-03-18T10:00:00Z",
    )
    .unwrap();
    assert!(first);

    // Same repo + same commit_hash → ignored
    let second = db::insert_activity(
        &conn,
        "/tmp/repo",
        "commit",
        Some("main"),
        None,
        Some("abc123"),
        Some("Alice"),
        Some("fix bug"),
        "2026-03-18T10:00:00Z",
    )
    .unwrap();
    assert!(!second);

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM git_activity WHERE commit_hash = 'abc123'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_insert_activity_commit_different_repos_both_kept() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    db::insert_activity(
        &conn,
        "/repo/a",
        "commit",
        Some("main"),
        None,
        Some("abc123"),
        Some("Alice"),
        Some("fix"),
        "2026-03-18T10:00:00Z",
    )
    .unwrap();

    db::insert_activity(
        &conn,
        "/repo/b",
        "commit",
        Some("main"),
        None,
        Some("abc123"),
        Some("Alice"),
        Some("fix"),
        "2026-03-18T10:00:00Z",
    )
    .unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM git_activity WHERE commit_hash = 'abc123'",
            [],
            |r| r.get(0),
        )
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
            &conn,
            "/tmp/repo",
            "branch_switch",
            Some("develop"),
            None,
            None,
            None,
            None,
            "2026-03-18T10:00:00Z",
        )
        .unwrap();
        assert!(inserted);
    }

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM git_activity WHERE event_type = 'branch_switch'",
            [],
            |r| r.get(0),
        )
        .unwrap();
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
fn test_file_changes_table_exists() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='file_changes'")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(tables, vec!["file_changes"]);

    let mut stmt = conn.prepare("PRAGMA table_info(file_changes)").unwrap();
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(columns.contains(&"repo_path".to_string()));
    assert!(columns.contains(&"commit_hash".to_string()));
    assert!(columns.contains(&"file_path".to_string()));
    assert!(columns.contains(&"lines_added".to_string()));
    assert!(columns.contains(&"lines_removed".to_string()));
    assert!(columns.contains(&"timestamp".to_string()));
}

#[test]
fn test_insert_file_change() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let inserted = db::insert_file_change(
        &conn,
        "/tmp/repo",
        "abc123",
        "src/main.rs",
        "2026-03-21T12:00:00Z",
    )
    .unwrap();
    assert!(inserted);

    let (repo, hash, fpath, ts): (String, String, String, String) = conn
        .query_row(
            "SELECT repo_path, commit_hash, file_path, timestamp FROM file_changes WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();

    assert_eq!(repo, "/tmp/repo");
    assert_eq!(hash, "abc123");
    assert_eq!(fpath, "src/main.rs");
    assert_eq!(ts, "2026-03-21T12:00:00Z");
}

#[test]
fn test_insert_file_change_dedup() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let first = db::insert_file_change(
        &conn,
        "/tmp/repo",
        "abc123",
        "src/main.rs",
        "2026-03-21T12:00:00Z",
    )
    .unwrap();
    assert!(first);

    // Same (repo_path, commit_hash, file_path) → ignored
    let second = db::insert_file_change(
        &conn,
        "/tmp/repo",
        "abc123",
        "src/main.rs",
        "2026-03-21T12:00:00Z",
    )
    .unwrap();
    assert!(!second);

    // Different file_path → not a duplicate
    let third = db::insert_file_change(
        &conn,
        "/tmp/repo",
        "abc123",
        "src/lib.rs",
        "2026-03-21T12:00:00Z",
    )
    .unwrap();
    assert!(third);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM file_changes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn test_query_churn_returns_files_above_threshold() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    // src/main.rs modified in 3 commits, src/lib.rs in 1
    for hash in ["aaa", "bbb", "ccc"] {
        db::insert_file_change(&conn, "/repo", hash, "src/main.rs", "2026-03-01T10:00:00Z")
            .unwrap();
    }
    db::insert_file_change(&conn, "/repo", "aaa", "src/lib.rs", "2026-03-01T10:00:00Z").unwrap();

    let results =
        db::query_churn(&conn, "2026-03-01T00:00:00Z", "2026-03-02T00:00:00Z", 3).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, "src/main.rs");
    assert_eq!(results[0].change_count, 3);
    assert_eq!(results[0].repo_path, "/repo");
}

#[test]
fn test_query_churn_ordered_by_count_desc_limited_to_20() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    // Create 22 files with decreasing change counts (22 down to 1)
    for i in 1..=22 {
        let fname = format!("file_{:02}.rs", i);
        for j in 0..i {
            let hash = format!("h{}_{}", i, j);
            db::insert_file_change(&conn, "/repo", &hash, &fname, "2026-03-01T10:00:00Z").unwrap();
        }
    }

    let results =
        db::query_churn(&conn, "2026-03-01T00:00:00Z", "2026-03-02T00:00:00Z", 1).unwrap();
    // Should be limited to 20
    assert_eq!(results.len(), 20);
    // First result should be file with most changes (22)
    assert_eq!(results[0].file_path, "file_22.rs");
    assert_eq!(results[0].change_count, 22);
    // Last result should be file_03.rs (3 changes; file_02 and file_01 are cut off)
    assert_eq!(results[19].file_path, "file_03.rs");
    assert_eq!(results[19].change_count, 3);
}

#[test]
fn test_query_churn_respects_time_range() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    // 3 changes in range
    for hash in ["a1", "a2", "a3"] {
        db::insert_file_change(&conn, "/repo", hash, "src/hot.rs", "2026-03-01T10:00:00Z").unwrap();
    }
    // 2 changes outside range
    for hash in ["b1", "b2"] {
        db::insert_file_change(&conn, "/repo", hash, "src/hot.rs", "2026-02-28T10:00:00Z").unwrap();
    }

    // Threshold 4: only 3 in range, should return empty
    let results =
        db::query_churn(&conn, "2026-03-01T00:00:00Z", "2026-03-02T00:00:00Z", 4).unwrap();
    assert!(results.is_empty());

    // Threshold 3: exactly 3 in range, should return 1
    let results =
        db::query_churn(&conn, "2026-03-01T00:00:00Z", "2026-03-02T00:00:00Z", 3).unwrap();
    assert_eq!(results.len(), 1);
}
