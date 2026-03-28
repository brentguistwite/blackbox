use blackbox::db;
use blackbox::repo_deep_dive::{find_db_repo_path, query_repo_all_time, resolve_repo_path};
use tempfile::TempDir;

#[test]
fn resolve_repo_path_valid_git_repo() {
    let tmp = TempDir::new().unwrap();
    git2::Repository::init(tmp.path()).unwrap();
    let result = resolve_repo_path(tmp.path().to_str().unwrap());
    assert!(result.is_ok());
    let resolved = result.unwrap();
    assert!(resolved.is_absolute());
}

#[test]
fn resolve_repo_path_nonexistent_path() {
    let result = resolve_repo_path("/tmp/definitely-does-not-exist-xyz-123");
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("Path not found"), "got: {msg}");
}

#[test]
fn resolve_repo_path_non_git_dir() {
    let tmp = TempDir::new().unwrap();
    // no git init — just a plain dir
    let result = resolve_repo_path(tmp.path().to_str().unwrap());
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("Not a git repository"), "got: {msg}");
}

#[test]
fn find_db_repo_path_empty_db() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();
    let result = find_db_repo_path(&conn, std::path::Path::new("/some/repo")).unwrap();
    assert_eq!(result, None);
}

#[test]
fn find_db_repo_path_with_matching_row() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();
    db::insert_activity(
        &conn,
        "/home/user/myrepo",
        "commit",
        Some("main"),
        None,
        Some("abc123"),
        Some("author"),
        Some("msg"),
        "2024-01-01T00:00:00Z",
    )
    .unwrap();
    let result = find_db_repo_path(&conn, std::path::Path::new("/home/user/myrepo")).unwrap();
    assert_eq!(result, Some("/home/user/myrepo".to_string()));
}

// --- US-002: query_repo_all_time ---

#[test]
fn query_repo_all_time_empty_db() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();
    let data = query_repo_all_time(&conn, "/no/such/repo").unwrap();
    assert_eq!(data.repo_path, "/no/such/repo");
    assert!(data.events.is_empty());
    assert!(data.reviews.is_empty());
    assert!(data.ai_sessions.is_empty());
}

#[test]
fn query_repo_all_time_returns_events() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();
    let repo = "/home/user/myrepo";
    db::insert_activity(&conn, repo, "commit", Some("main"), None, Some("aaa"), Some("a"), Some("first"), "2024-01-01T00:00:00Z").unwrap();
    db::insert_activity(&conn, repo, "commit", Some("main"), None, Some("bbb"), Some("a"), Some("second"), "2024-01-02T00:00:00Z").unwrap();
    db::insert_activity(&conn, repo, "branch_switch", Some("feat"), None, None, None, None, "2024-01-03T00:00:00Z").unwrap();
    // different repo — should not appear
    db::insert_activity(&conn, "/other/repo", "commit", Some("main"), None, Some("ccc"), Some("a"), Some("other"), "2024-01-01T00:00:00Z").unwrap();

    let data = query_repo_all_time(&conn, repo).unwrap();
    assert_eq!(data.events.len(), 3);
    // ordered by timestamp ASC
    assert_eq!(data.events[0].commit_hash.as_deref(), Some("aaa"));
    assert_eq!(data.events[2].event_type, "branch_switch");
}

#[test]
fn query_repo_all_time_returns_reviews() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();
    let repo = "/home/user/myrepo";
    db::insert_review(&conn, repo, 1, "PR one", "https://gh/1", "APPROVED", "2024-02-01T00:00:00Z").unwrap();
    db::insert_review(&conn, repo, 2, "PR two", "https://gh/2", "COMMENTED", "2024-02-02T00:00:00Z").unwrap();
    // different repo
    db::insert_review(&conn, "/other/repo", 3, "PR three", "https://gh/3", "APPROVED", "2024-02-01T00:00:00Z").unwrap();

    let data = query_repo_all_time(&conn, repo).unwrap();
    assert_eq!(data.reviews.len(), 2);
    assert_eq!(data.reviews[0].pr_number, 1);
    assert_eq!(data.reviews[1].pr_title, "PR two");
}

#[test]
fn query_repo_all_time_returns_ai_sessions() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();
    let repo = "/home/user/myrepo";
    db::insert_ai_session(&conn, repo, "sess-1", "2024-03-01T00:00:00Z").unwrap();
    db::update_session_ended(&conn, "sess-1", "2024-03-01T01:00:00Z", Some(5)).unwrap();
    db::insert_ai_session(&conn, repo, "sess-2", "2024-03-02T00:00:00Z").unwrap();
    // different repo
    db::insert_ai_session(&conn, "/other/repo", "sess-3", "2024-03-01T00:00:00Z").unwrap();

    let data = query_repo_all_time(&conn, repo).unwrap();
    assert_eq!(data.ai_sessions.len(), 2);
    assert_eq!(data.ai_sessions[0].session_id, "sess-1");
    assert!(data.ai_sessions[0].ended_at.is_some());
    assert!(data.ai_sessions[1].ended_at.is_none());
}
