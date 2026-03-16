use blackbox::db;
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
