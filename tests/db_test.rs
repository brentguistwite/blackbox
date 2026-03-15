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
