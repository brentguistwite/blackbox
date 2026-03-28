use blackbox::db;
use blackbox::repo_deep_dive::{find_db_repo_path, resolve_repo_path};
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
