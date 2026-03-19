use blackbox::db;
use blackbox::git_ops::{poll_repo, RepoState};
use chrono::TimeZone;
use git2::{Repository, Signature};
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
fn test_first_poll_backfills_todays_commits() {
    let repo_tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();
    // create_test_repo uses Signature::now() → today's timestamp → should be backfilled
    let _repo = create_test_repo(&repo_tmp);
    let conn = open_test_db(&db_tmp);

    let mut state = RepoState::default();
    let path_str = repo_tmp.path().to_string_lossy();
    poll_repo(repo_tmp.path(), &path_str, &mut state, &conn).unwrap();

    // State should be seeded
    assert!(state.last_commit_oid.is_some());
    assert!(state.last_head_branch.is_some());

    // Backfill should have inserted the initial commit (today's timestamp)
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_activity WHERE event_type = 'commit'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(total, 1);
}

#[test]
fn test_new_commit_detected() {
    let repo_tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();
    let repo = create_test_repo(&repo_tmp);
    let conn = open_test_db(&db_tmp);

    let mut state = RepoState::default();
    let path_str = repo_tmp.path().to_string_lossy();
    // First poll: seed
    poll_repo(repo_tmp.path(), &path_str, &mut state, &conn).unwrap();

    // Add a commit
    add_commit(&repo, "second commit");

    // Second poll: should detect new commit
    poll_repo(repo_tmp.path(), &path_str, &mut state, &conn).unwrap();

    // 2 commits: backfilled initial + new "second commit"
    assert_eq!(count_events(&conn, "commit"), 2);

    let msg: String = conn
        .query_row(
            "SELECT message FROM git_activity WHERE event_type = 'commit' ORDER BY id DESC LIMIT 1",
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
    let path_str = repo_tmp.path().to_string_lossy();
    // First poll: seed on main/master
    poll_repo(repo_tmp.path(), &path_str, &mut state, &conn).unwrap();

    // Switch to new branch
    create_branch_and_switch(&repo, "feature-x");

    // Second poll: should detect branch switch
    poll_repo(repo_tmp.path(), &path_str, &mut state, &conn).unwrap();

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
    let path_str = repo_tmp.path().to_string_lossy();
    // First poll: seed
    poll_repo(repo_tmp.path(), &path_str, &mut state, &conn).unwrap();

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
    poll_repo(repo_tmp.path(), &path_str, &mut state, &conn).unwrap();

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
    let path_str = repo_tmp.path().to_string_lossy();
    poll_repo(repo_tmp.path(), &path_str, &mut state, &conn).unwrap();

    // Detach HEAD
    let head_oid = repo.head().unwrap().target().unwrap();
    repo.set_head_detached(head_oid).unwrap();

    // Should not panic
    poll_repo(repo_tmp.path(), &path_str, &mut state, &conn).unwrap();
    // Branch should now be None
    assert!(state.last_head_branch.is_none());
}

// --- Worktree-specific tests ---

/// Helper: create a main repo + worktree, returns (main_repo, worktree_path)
fn create_repo_with_worktree(tmp: &TempDir) -> (Repository, std::path::PathBuf) {
    let main_dir = tmp.path().join("main");
    std::fs::create_dir_all(&main_dir).unwrap();
    let repo = Repository::init(&main_dir).unwrap();
    let sig = Signature::now("Test", "test@test.com").unwrap();
    {
        let tree_id = {
            let mut index = repo.index().unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
    }

    // Create a branch for the worktree
    {
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("wt-branch", &head, false).unwrap();
    }

    // Create worktree
    let wt_path = tmp.path().join("wt1");
    {
        let wt_ref = repo.find_reference("refs/heads/wt-branch").unwrap();
        repo.worktree("wt1", &wt_path, Some(git2::WorktreeAddOptions::new().reference(Some(&wt_ref)))).unwrap();
    }

    (repo, wt_path)
}

#[test]
fn test_poll_repo_uses_db_repo_path_for_writes() {
    let repo_tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();
    let repo = create_test_repo(&repo_tmp);
    let conn = open_test_db(&db_tmp);

    let mut state = RepoState::default();
    let custom_db_path = "/custom/main/repo";
    // Seed
    poll_repo(repo_tmp.path(), custom_db_path, &mut state, &conn).unwrap();
    // Add commit
    add_commit(&repo, "test commit");
    // Poll
    poll_repo(repo_tmp.path(), custom_db_path, &mut state, &conn).unwrap();

    let repo_path: String = conn
        .query_row(
            "SELECT repo_path FROM git_activity WHERE event_type = 'commit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(repo_path, custom_db_path);
}

#[test]
fn test_worktree_commit_attributed_to_main_repo() {
    let tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();
    let (_main_repo, wt_path) = create_repo_with_worktree(&tmp);
    let conn = open_test_db(&db_tmp);

    // Resolve main repo path (simulating what poller does)
    let main_path = blackbox::repo_scanner::resolve_main_repo(&wt_path).unwrap();
    let db_repo_path = main_path.to_string_lossy().to_string();

    let mut state = RepoState {
        main_repo_path: main_path.clone(),
        ..Default::default()
    };
    // Seed
    poll_repo(&wt_path, &db_repo_path, &mut state, &conn).unwrap();

    // Add commit in worktree
    let wt_repo = Repository::open(&wt_path).unwrap();
    let sig = Signature::now("Test", "test@test.com").unwrap();
    let head = wt_repo.head().unwrap().peel_to_commit().unwrap();
    let tree = wt_repo.find_tree(head.tree_id()).unwrap();
    wt_repo
        .commit(Some("HEAD"), &sig, &sig, "worktree commit", &tree, &[&head])
        .unwrap();

    // Poll
    poll_repo(&wt_path, &db_repo_path, &mut state, &conn).unwrap();

    // DB should have commit under main repo path, not worktree path
    let recorded_path: String = conn
        .query_row(
            "SELECT repo_path FROM git_activity WHERE event_type = 'commit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(recorded_path, db_repo_path);
    assert!(!recorded_path.contains("wt1"));
}

#[test]
fn test_two_worktrees_tracked_independently() {
    let tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();
    let conn = open_test_db(&db_tmp);

    // Create main repo
    let main_dir = tmp.path().join("main");
    std::fs::create_dir_all(&main_dir).unwrap();
    let repo = Repository::init(&main_dir).unwrap();
    let sig = Signature::now("Test", "test@test.com").unwrap();
    let tree_id = {
        let mut index = repo.index().unwrap();
        index.write_tree().unwrap()
    };
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
        .unwrap();

    // Create two branches + worktrees
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("branch-a", &head, false).unwrap();
    repo.branch("branch-b", &head, false).unwrap();

    let wt_a = tmp.path().join("wt-a");
    let ref_a = repo.find_reference("refs/heads/branch-a").unwrap();
    repo.worktree(
        "wt-a",
        &wt_a,
        Some(git2::WorktreeAddOptions::new().reference(Some(&ref_a))),
    )
    .unwrap();

    let wt_b = tmp.path().join("wt-b");
    let ref_b = repo.find_reference("refs/heads/branch-b").unwrap();
    repo.worktree(
        "wt-b",
        &wt_b,
        Some(git2::WorktreeAddOptions::new().reference(Some(&ref_b))),
    )
    .unwrap();

    let main_path = main_dir.canonicalize().unwrap();
    let db_path = main_path.to_string_lossy().to_string();

    // Each worktree gets its own state
    let mut state_a = RepoState {
        main_repo_path: main_path.clone(),
        ..Default::default()
    };
    let mut state_b = RepoState {
        main_repo_path: main_path.clone(),
        ..Default::default()
    };

    // Seed both
    poll_repo(&wt_a, &db_path, &mut state_a, &conn).unwrap();
    poll_repo(&wt_b, &db_path, &mut state_b, &conn).unwrap();

    // Each should track its own branch
    assert_eq!(state_a.last_head_branch.as_deref(), Some("branch-a"));
    assert_eq!(state_b.last_head_branch.as_deref(), Some("branch-b"));

    // Commit in wt-a only
    let wt_a_repo = Repository::open(&wt_a).unwrap();
    let head_a = wt_a_repo.head().unwrap().peel_to_commit().unwrap();
    let tree_a = wt_a_repo.find_tree(head_a.tree_id()).unwrap();
    wt_a_repo
        .commit(Some("HEAD"), &sig, &sig, "commit in a", &tree_a, &[&head_a])
        .unwrap();

    poll_repo(&wt_a, &db_path, &mut state_a, &conn).unwrap();
    poll_repo(&wt_b, &db_path, &mut state_b, &conn).unwrap();

    // 2 commits: backfilled initial (deduped across worktrees) + "commit in a"
    assert_eq!(count_events(&conn, "commit"), 2);

    let msg: String = conn
        .query_row(
            "SELECT message FROM git_activity WHERE event_type = 'commit' ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(msg, "commit in a");
}

#[test]
fn test_regular_repo_main_repo_path_equals_key() {
    let repo_tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();
    let repo = create_test_repo(&repo_tmp);
    let conn = open_test_db(&db_tmp);

    let path = repo_tmp.path().to_path_buf();
    let path_str = path.to_string_lossy().to_string();

    // Simulating poller: is_worktree returns None, so main_repo_path == repo path
    assert!(blackbox::repo_scanner::is_worktree(&path).is_none());
    let mut state = RepoState {
        main_repo_path: path.clone(),
        ..Default::default()
    };

    poll_repo(&path, &path_str, &mut state, &conn).unwrap();
    add_commit(&repo, "regular commit");
    poll_repo(&path, &path_str, &mut state, &conn).unwrap();

    let recorded: String = conn
        .query_row(
            "SELECT repo_path FROM git_activity WHERE event_type = 'commit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(recorded, path_str);
}

/// Create a repo whose initial commit has a specific epoch timestamp.
fn create_test_repo_at(tmp: &TempDir, epoch: i64) -> Repository {
    let repo = Repository::init(tmp.path()).unwrap();
    let sig = Signature::new("Test", "test@test.com", &git2::Time::new(epoch, 0)).unwrap();
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

/// Add a commit with a specific epoch timestamp.
fn add_commit_at(repo: &Repository, message: &str, epoch: i64) -> git2::Oid {
    let sig = Signature::new("Test", "test@test.com", &git2::Time::new(epoch, 0)).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let tree = repo.find_tree(head.tree_id()).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head])
        .unwrap()
}

#[test]
fn test_first_poll_skips_old_commits() {
    let repo_tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();

    // Yesterday at noon
    let yesterday = chrono::Local::now().date_naive().pred_opt().unwrap();
    let yesterday_noon = yesterday.and_hms_opt(12, 0, 0).unwrap();
    let yesterday_epoch = chrono::Local
        .from_local_datetime(&yesterday_noon)
        .earliest()
        .unwrap()
        .timestamp();

    let _repo = create_test_repo_at(&repo_tmp, yesterday_epoch);
    let conn = open_test_db(&db_tmp);

    let mut state = RepoState::default();
    let path_str = repo_tmp.path().to_string_lossy();
    poll_repo(repo_tmp.path(), &path_str, &mut state, &conn).unwrap();

    // Old commit should NOT be backfilled
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM git_activity WHERE event_type = 'commit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(total, 0);
}

#[test]
fn test_first_poll_cap_at_50() {
    let repo_tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();

    // Use a timestamp firmly today
    let today_noon = chrono::Local::now()
        .date_naive()
        .and_hms_opt(12, 0, 0)
        .unwrap();
    let today_epoch = chrono::Local
        .from_local_datetime(&today_noon)
        .earliest()
        .unwrap()
        .timestamp();

    let repo = create_test_repo_at(&repo_tmp, today_epoch);
    // Add 54 more commits (55 total with initial)
    for i in 1..=54 {
        add_commit_at(&repo, &format!("commit {}", i), today_epoch + i);
    }

    let conn = open_test_db(&db_tmp);
    let mut state = RepoState::default();
    let path_str = repo_tmp.path().to_string_lossy();
    poll_repo(repo_tmp.path(), &path_str, &mut state, &conn).unwrap();

    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM git_activity WHERE event_type = 'commit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(total, 50);
}

#[test]
fn test_first_poll_dedup_existing() {
    let repo_tmp = TempDir::new().unwrap();
    let db_tmp = TempDir::new().unwrap();
    let _repo = create_test_repo(&repo_tmp);
    let conn = open_test_db(&db_tmp);

    // Manually pre-insert the commit that will be backfilled
    let repo = Repository::open(repo_tmp.path()).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let hash = head.id().to_string();
    let path_str = repo_tmp.path().to_string_lossy();
    db::insert_activity(
        &conn,
        &path_str,
        "commit",
        Some("main"),
        None,
        Some(&hash),
        Some("Test"),
        Some("initial commit"),
        &chrono::Utc::now().to_rfc3339(),
    )
    .unwrap();

    let mut state = RepoState::default();
    poll_repo(repo_tmp.path(), &path_str, &mut state, &conn).unwrap();

    // Should still be only 1 record (INSERT OR IGNORE dedup)
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM git_activity WHERE event_type = 'commit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(total, 1);
}
