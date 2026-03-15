use blackbox::db;
use blackbox::git_ops::{poll_repo, RepoState};
use git2::{Repository, Signature};
use std::path::Path;
use tempfile::TempDir;

fn create_test_repo(tmp: &TempDir) -> Repository {
    let repo = Repository::init(tmp.path()).unwrap();
    let sig = Signature::now("Test", "test@test.com").unwrap();
    let tree_id = {
        let mut index = repo.index().unwrap();
        index.write_tree().unwrap()
    };
    {
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
            .unwrap();
    }
    repo
}

fn add_commit(repo: &Repository, message: &str) -> git2::Oid {
    let sig = Signature::now("Test", "test@test.com").unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let tree = repo.find_tree(head.tree_id()).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head])
        .unwrap()
}

fn create_branch_and_switch(repo: &Repository, name: &str) {
    let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch(name, &head_commit, false).unwrap();
    repo.set_head(&format!("refs/heads/{}", name)).unwrap();
}

fn open_test_db(tmp: &TempDir) -> rusqlite::Connection {
    let db_path = tmp.path().join("test.db");
    db::open_db(&db_path).unwrap()
}

fn count_events(conn: &rusqlite::Connection, event_type: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM git_activity WHERE event_type = ?1",
        [event_type],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn test_first_poll_seeds_state_no_records() {
    let repo_tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();
    let _repo = create_test_repo(&repo_tmp);
    let conn = open_test_db(&db_tmp);

    let mut state = RepoState::default();
    poll_repo(repo_tmp.path(), &mut state, &conn).unwrap();

    // State should be seeded
    assert!(state.last_commit_oid.is_some());
    assert!(state.last_head_branch.is_some());

    // No records should be inserted on first poll
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_activity", [], |row| row.get(0))
        .unwrap();
    assert_eq!(total, 0);
}

#[test]
fn test_new_commit_detected() {
    let repo_tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();
    let repo = create_test_repo(&repo_tmp);
    let conn = open_test_db(&db_tmp);

    let mut state = RepoState::default();
    // First poll: seed
    poll_repo(repo_tmp.path(), &mut state, &conn).unwrap();

    // Add a commit
    add_commit(&repo, "second commit");

    // Second poll: should detect new commit
    poll_repo(repo_tmp.path(), &mut state, &conn).unwrap();

    assert_eq!(count_events(&conn, "commit"), 1);

    let msg: String = conn
        .query_row(
            "SELECT message FROM git_activity WHERE event_type = 'commit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(msg, "second commit");
}

#[test]
fn test_branch_switch_detected() {
    let repo_tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();
    let repo = create_test_repo(&repo_tmp);
    let conn = open_test_db(&db_tmp);

    let mut state = RepoState::default();
    // First poll: seed on main/master
    poll_repo(repo_tmp.path(), &mut state, &conn).unwrap();

    // Switch to new branch
    create_branch_and_switch(&repo, "feature-x");

    // Second poll: should detect branch switch
    poll_repo(repo_tmp.path(), &mut state, &conn).unwrap();

    assert_eq!(count_events(&conn, "branch_switch"), 1);

    let branch: String = conn
        .query_row(
            "SELECT branch FROM git_activity WHERE event_type = 'branch_switch'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(branch, "feature-x");
}

#[test]
fn test_merge_commit_detected() {
    let repo_tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();
    let repo = create_test_repo(&repo_tmp);
    let conn = open_test_db(&db_tmp);

    let mut state = RepoState::default();
    // First poll: seed
    poll_repo(repo_tmp.path(), &mut state, &conn).unwrap();

    // Create a branch, add commit on it, switch back, merge
    let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feature-branch", &head_commit, false).unwrap();
    repo.set_head("refs/heads/feature-branch").unwrap();
    let feature_oid = add_commit(&repo, "feature work");

    // Switch back to main branch
    let default_branch = state.last_head_branch.clone().unwrap();
    repo.set_head(&format!("refs/heads/{}", default_branch))
        .unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();

    // Create merge commit
    let sig = Signature::now("Test", "test@test.com").unwrap();
    let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
    let feature_commit = repo.find_commit(feature_oid).unwrap();
    let tree = repo.find_tree(main_commit.tree_id()).unwrap();
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "Merge feature-branch into main",
        &tree,
        &[&main_commit, &feature_commit],
    )
    .unwrap();

    // Poll again -- detect merge + the feature commit
    // Reset state branch to avoid branch_switch noise from our manual set_head
    state.last_head_branch = Some(default_branch.clone());
    poll_repo(repo_tmp.path(), &mut state, &conn).unwrap();

    assert!(count_events(&conn, "merge") >= 1);

    let source: String = conn
        .query_row(
            "SELECT source_branch FROM git_activity WHERE event_type = 'merge'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // Should be feature-branch or a short hash fallback
    assert!(!source.is_empty());
}

#[test]
fn test_detached_head_handled() {
    let repo_tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();
    let repo = create_test_repo(&repo_tmp);
    let conn = open_test_db(&db_tmp);

    let mut state = RepoState::default();
    poll_repo(repo_tmp.path(), &mut state, &conn).unwrap();

    // Detach HEAD
    let head_oid = repo.head().unwrap().target().unwrap();
    repo.set_head_detached(head_oid).unwrap();

    // Should not panic
    poll_repo(repo_tmp.path(), &mut state, &conn).unwrap();
    // Branch should now be None
    assert!(state.last_head_branch.is_none());
}
