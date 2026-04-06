use blackbox::churn::{compute_churn, diff_commit_stats};
use blackbox::db;
use blackbox::git_ops::{poll_repo, RepoState};
use chrono::{Duration, Utc};
use git2::{Repository, Signature};
use std::path::Path;
use tempfile::TempDir;

fn make_commit(repo: &Repository, message: &str, files: &[(&str, &str)]) -> git2::Oid {
    let sig = Signature::now("Test User", "test@example.com").unwrap();
    let mut index = repo.index().unwrap();
    for (name, content) in files {
        std::fs::write(repo.workdir().unwrap().join(name), content).unwrap();
        index.add_path(Path::new(name)).unwrap();
    }
    index.write().unwrap();
    let tree_oid = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let parents: Vec<git2::Commit> = repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_commit().ok())
        .map(|c| vec![c])
        .unwrap_or_default();
    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
        .unwrap()
}

fn init_repo(tmp: &TempDir) -> Repository {
    let repo = Repository::init(tmp.path()).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.email", "test@example.com").unwrap();
    config.set_str("user.name", "Test User").unwrap();
    repo
}

#[test]
fn test_single_file_add_5_lines() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    let content = "line1\nline2\nline3\nline4\nline5\n";
    let oid = make_commit(&repo, "add file", &[("hello.txt", content)]);
    let commit = repo.find_commit(oid).unwrap();

    let stats = diff_commit_stats(&repo, &commit).unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].file_path, "hello.txt");
    assert_eq!(stats[0].lines_added, 5);
    assert_eq!(stats[0].lines_deleted, 0);
}

#[test]
fn test_edit_existing_file() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    // Initial commit: 4 lines
    make_commit(&repo, "initial", &[("f.txt", "a\nb\nc\nd\n")]);
    // Edit: remove 2 lines (b, c), add 3 new lines
    let oid = make_commit(&repo, "edit", &[("f.txt", "a\nx\ny\nz\nd\n")]);
    let commit = repo.find_commit(oid).unwrap();

    let stats = diff_commit_stats(&repo, &commit).unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].file_path, "f.txt");
    assert_eq!(stats[0].lines_added, 3);
    assert_eq!(stats[0].lines_deleted, 2);
}

#[test]
fn test_merge_commit_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    let sig = Signature::now("Test User", "test@example.com").unwrap();

    // Create initial commit on main
    make_commit(&repo, "initial", &[("main.txt", "main\n")]);

    // Determine default branch name
    let default_branch = repo.head().unwrap().shorthand().unwrap().to_string();

    // Create branch and commit
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feature", &head, false).unwrap();
    repo.set_head("refs/heads/feature").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();
    let feat_oid = make_commit(&repo, "feature work", &[("feat.txt", "feat\n")]);

    // Switch back to default branch, add another commit
    repo.set_head(&format!("refs/heads/{}", default_branch))
        .unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();
    make_commit(&repo, "main work", &[("main2.txt", "main2\n")]);

    // Create merge commit (two parents)
    let main_head = repo.head().unwrap().peel_to_commit().unwrap();
    let feat_commit = repo.find_commit(feat_oid).unwrap();
    let mut index = repo
        .merge_commits(&main_head, &feat_commit, None)
        .unwrap();
    let tree_oid = index.write_tree_to(&repo).unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let merge_oid = repo
        .commit(
            Some("HEAD"),
            &sig,
            &sig,
            "merge",
            &tree,
            &[&main_head, &feat_commit],
        )
        .unwrap();
    let merge_commit = repo.find_commit(merge_oid).unwrap();

    let stats = diff_commit_stats(&repo, &merge_commit).unwrap();
    assert!(stats.is_empty(), "merge commits should return empty vec");
}

#[test]
fn test_initial_commit_no_parent() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    let oid = make_commit(
        &repo,
        "initial",
        &[("a.txt", "one\ntwo\nthree\n"), ("b.txt", "x\n")],
    );
    let commit = repo.find_commit(oid).unwrap();

    let stats = diff_commit_stats(&repo, &commit).unwrap();
    // Two files
    assert_eq!(stats.len(), 2);

    let a = stats.iter().find(|s| s.file_path == "a.txt").unwrap();
    assert_eq!(a.lines_added, 3);
    assert_eq!(a.lines_deleted, 0);

    let b = stats.iter().find(|s| s.file_path == "b.txt").unwrap();
    assert_eq!(b.lines_added, 1);
    assert_eq!(b.lines_deleted, 0);
}

// --- US-007: compute_churn tests ---

fn setup_db() -> (tempfile::NamedTempFile, rusqlite::Connection) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let conn = db::open_db(tmp.path()).unwrap();
    (tmp, conn)
}

#[test]
fn test_churn_two_commits_same_file_within_window() {
    let (_tmp, conn) = setup_db();
    let repo_path = "/test/repo";

    let yesterday = (Utc::now() - Duration::days(1)).to_rfc3339();
    let today = Utc::now().to_rfc3339();

    // Commit A: adds 10 lines to file.txt
    db::insert_commit_line_stats(&conn, repo_path, "aaa", "file.txt", 10, 0, &yesterday).unwrap();
    // Commit B: deletes 4 lines from file.txt (within 14-day window)
    db::insert_commit_line_stats(&conn, repo_path, "bbb", "file.txt", 2, 4, &today).unwrap();

    let report = compute_churn(&conn, repo_path, 14).unwrap();
    // churned = min(A.lines_added=10, B.lines_deleted=4) = 4
    // total_written = 10 + 2 = 12
    // churn_rate = 4/12 * 100 ≈ 33.33
    assert_eq!(report.churned_lines, 4);
    assert_eq!(report.total_lines_written, 12);
    assert!(report.churn_rate_pct > 33.0 && report.churn_rate_pct < 34.0);
    assert_eq!(report.churn_event_count, 1);
    assert_eq!(report.commit_count, 2);
    assert_eq!(report.window_days, 14);
    assert_eq!(report.repo_path, repo_path);
}

#[test]
fn test_churn_two_commits_same_file_outside_window() {
    let (_tmp, conn) = setup_db();
    let repo_path = "/test/repo";

    // Commit A: 20 days ago — outside a 14-day query window
    let twenty_days_ago = (Utc::now() - Duration::days(20)).to_rfc3339();
    // Commit B: 1 day ago — inside window
    let one_day_ago = (Utc::now() - Duration::days(1)).to_rfc3339();

    db::insert_commit_line_stats(&conn, repo_path, "aaa", "file.txt", 10, 0, &twenty_days_ago).unwrap();
    db::insert_commit_line_stats(&conn, repo_path, "bbb", "file.txt", 2, 4, &one_day_ago).unwrap();

    // window=14: commit A filtered out by query, only B returned → no churn pair
    let report = compute_churn(&conn, repo_path, 14).unwrap();
    assert_eq!(report.churned_lines, 0);
    assert_eq!(report.churn_rate_pct, 0.0);
    assert_eq!(report.churn_event_count, 0);
}

#[test]
fn test_churn_two_commits_different_files() {
    let (_tmp, conn) = setup_db();
    let repo_path = "/test/repo";

    let yesterday = (Utc::now() - Duration::days(1)).to_rfc3339();
    let today = Utc::now().to_rfc3339();

    db::insert_commit_line_stats(&conn, repo_path, "aaa", "a.txt", 10, 0, &yesterday).unwrap();
    db::insert_commit_line_stats(&conn, repo_path, "bbb", "b.txt", 5, 3, &today).unwrap();

    let report = compute_churn(&conn, repo_path, 14).unwrap();
    // Different files → no churn pairs
    assert_eq!(report.churned_lines, 0);
    assert_eq!(report.churn_rate_pct, 0.0);
    assert_eq!(report.churn_event_count, 0);
    assert_eq!(report.total_lines_written, 15);
}

#[test]
fn test_churn_no_data_zero_rate() {
    let (_tmp, conn) = setup_db();
    let repo_path = "/test/empty";

    let report = compute_churn(&conn, repo_path, 14).unwrap();
    assert_eq!(report.total_lines_written, 0);
    assert_eq!(report.churned_lines, 0);
    assert_eq!(report.churn_rate_pct, 0.0);
    assert_eq!(report.commit_count, 0);
    assert_eq!(report.churn_event_count, 0);
}

// --- US-012: End-to-end integration tests ---

fn make_commit_at(repo: &Repository, message: &str, files: &[(&str, &str)], epoch: i64) -> git2::Oid {
    let time = git2::Time::new(epoch, 0);
    let sig = Signature::new("Test User", "test@example.com", &time).unwrap();
    let mut index = repo.index().unwrap();
    for (name, content) in files {
        std::fs::write(repo.workdir().unwrap().join(name), content).unwrap();
        index.add_path(Path::new(name)).unwrap();
    }
    index.write().unwrap();
    let tree_oid = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let parents: Vec<git2::Commit> = repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_commit().ok())
        .map(|c| vec![c])
        .unwrap_or_default();
    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
        .unwrap()
}

/// Create 3-commit test repo, poll it, return (db_file, conn, repo_dir, repo_path_str).
fn setup_three_commit_repo() -> (tempfile::NamedTempFile, rusqlite::Connection, TempDir, String) {
    let tmp_repo = TempDir::new().unwrap();
    let repo = init_repo(&tmp_repo);
    let (tmp_db, conn) = setup_db();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // Commit A: adds 10 lines to file.txt
    make_commit_at(
        &repo,
        "add file.txt",
        &[("file.txt", "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\n")],
        now - 100,
    );
    // Commit B: removes 4 lines from file.txt (within window)
    make_commit_at(
        &repo,
        "trim file.txt",
        &[("file.txt", "line1\nline2\nline3\nline4\nline5\nline6\n")],
        now - 50,
    );
    // Commit C: adds new file with 5 lines
    make_commit_at(
        &repo,
        "add new.txt",
        &[("new.txt", "new1\nnew2\nnew3\nnew4\nnew5\n")],
        now,
    );

    let repo_path_str = tmp_repo.path().to_str().unwrap().to_string();
    let mut state = RepoState::default();
    poll_repo(tmp_repo.path(), &repo_path_str, &mut state, &conn).unwrap();

    (tmp_db, conn, tmp_repo, repo_path_str)
}

#[test]
fn test_e2e_three_commit_churn() {
    let (_db, conn, _repo_dir, repo_path) = setup_three_commit_repo();

    let report = compute_churn(&conn, &repo_path, 14).unwrap();

    // churned = min(A.added=10, B.deleted=4) = 4
    // total_written = 10 + 0 + 5 = 15
    // churn_rate = 4/15 * 100 ≈ 26.67
    assert_eq!(report.total_lines_written, 15);
    assert_eq!(report.churned_lines, 4);
    assert!(
        (report.churn_rate_pct - 26.67).abs() < 1.0,
        "expected ≈26.7%, got {:.2}%",
        report.churn_rate_pct
    );
}

#[test]
fn test_e2e_window_zero_no_churn() {
    let (_db, conn, _repo_dir, repo_path) = setup_three_commit_repo();

    // window=0 → since=now → no stats returned → 0 churn
    let report = compute_churn(&conn, &repo_path, 0).unwrap();
    assert_eq!(report.churn_rate_pct, 0.0);
    assert_eq!(report.total_lines_written, 0);
}

#[test]
fn test_e2e_single_commit_no_churn() {
    let tmp_repo = TempDir::new().unwrap();
    let repo = init_repo(&tmp_repo);
    let (_tmp_db, conn) = setup_db();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    make_commit_at(&repo, "initial", &[("file.txt", "a\nb\nc\nd\ne\n")], now);

    let repo_path_str = tmp_repo.path().to_str().unwrap();
    let mut state = RepoState::default();
    poll_repo(tmp_repo.path(), repo_path_str, &mut state, &conn).unwrap();

    let report = compute_churn(&conn, repo_path_str, 14).unwrap();
    assert_eq!(report.churn_rate_pct, 0.0);
    assert_eq!(report.churned_lines, 0);
    assert!(report.total_lines_written > 0, "should have recorded line stats");
}

#[test]
fn test_e2e_no_commit_line_stats() {
    let (_tmp_db, conn) = setup_db();

    let report = compute_churn(&conn, "/nonexistent/repo", 14).unwrap();
    assert_eq!(report.total_lines_written, 0);
    assert_eq!(report.churn_rate_pct, 0.0);
    assert_eq!(report.churned_lines, 0);
    assert_eq!(report.commit_count, 0);
    assert_eq!(report.churn_event_count, 0);
}
