use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

// --- path_to_repo tests ---

#[test]
fn test_path_to_repo_head_file() {
    let repo = PathBuf::from("/home/user/projects/myrepo");
    let watched = vec![repo.clone()];
    let event_path = PathBuf::from("/home/user/projects/myrepo/.git/HEAD");
    assert_eq!(
        blackbox::watcher::path_to_repo(&event_path, &watched),
        Some(repo)
    );
}

#[test]
fn test_path_to_repo_refs_file() {
    let repo = PathBuf::from("/home/user/projects/myrepo");
    let watched = vec![repo.clone()];
    let event_path = PathBuf::from("/home/user/projects/myrepo/.git/refs/heads/main");
    assert_eq!(
        blackbox::watcher::path_to_repo(&event_path, &watched),
        Some(repo)
    );
}

#[test]
fn test_path_to_repo_unwatched_repo() {
    let watched = vec![PathBuf::from("/home/user/projects/other")];
    let event_path = PathBuf::from("/home/user/projects/myrepo/.git/HEAD");
    assert_eq!(blackbox::watcher::path_to_repo(&event_path, &watched), None);
}

#[test]
fn test_path_to_repo_no_git_dir() {
    let watched = vec![PathBuf::from("/home/user/projects/myrepo")];
    let event_path = PathBuf::from("/home/user/projects/myrepo/src/main.rs");
    assert_eq!(blackbox::watcher::path_to_repo(&event_path, &watched), None);
}

#[test]
fn test_path_to_repo_nested_refs() {
    let repo = PathBuf::from("/home/user/projects/myrepo");
    let watched = vec![repo.clone()];
    let event_path =
        PathBuf::from("/home/user/projects/myrepo/.git/refs/heads/feature/my-branch");
    assert_eq!(
        blackbox::watcher::path_to_repo(&event_path, &watched),
        Some(repo)
    );
}

// --- RepoWatcher tests ---

#[test]
fn test_watcher_creation_with_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path().join("myrepo");
    std::fs::create_dir_all(&repo_path).unwrap();
    git2::Repository::init(&repo_path).unwrap();

    let watcher = blackbox::watcher::RepoWatcher::new(&[repo_path.clone()]);
    assert!(watcher.is_ok(), "Watcher should be created for valid repos");
    let w = watcher.unwrap();
    assert_eq!(w.repos(), &[repo_path]);
}

#[test]
fn test_watcher_creation_empty_repos() {
    let watcher = blackbox::watcher::RepoWatcher::new(&[]);
    assert!(watcher.is_ok(), "Watcher should work with empty repo list");
    assert!(watcher.unwrap().repos().is_empty());
}

#[test]
fn test_watcher_creation_nonexistent_repo() {
    let repos = vec![PathBuf::from("/nonexistent/repo")];
    let watcher = blackbox::watcher::RepoWatcher::new(&repos);
    // Should still create watcher, just skip watches for missing paths
    assert!(watcher.is_ok());
}

#[test]
fn test_watcher_recv_timeout_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path().join("myrepo");
    std::fs::create_dir_all(&repo_path).unwrap();
    git2::Repository::init(&repo_path).unwrap();

    let watcher = blackbox::watcher::RepoWatcher::new(&[repo_path]).unwrap();
    let mut debounce = HashMap::new();

    // Short timeout — should return empty (no events)
    let changed = watcher.recv_events(&mut debounce, Duration::from_millis(50));
    assert!(changed.is_empty());
}

#[test]
fn test_watcher_detects_head_change() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path().join("myrepo");
    std::fs::create_dir_all(&repo_path).unwrap();
    let repo = git2::Repository::init(&repo_path).unwrap();

    // Create initial commit so HEAD exists
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    let tree_id = {
        let mut index = repo.index().unwrap();
        std::fs::write(repo_path.join("README.md"), "# test").unwrap();
        index.add_path(Path::new("README.md")).unwrap();
        index.write().unwrap();
        index.write_tree().unwrap()
    };
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
        .unwrap();

    let watcher = blackbox::watcher::RepoWatcher::new(&[repo_path.clone()]).unwrap();
    let mut debounce = HashMap::new();

    // Make a new commit (changes .git/refs/heads/main or .git/HEAD)
    let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
    let tree_id = {
        let mut index = repo.index().unwrap();
        std::fs::write(repo_path.join("new.txt"), "new").unwrap();
        index.add_path(Path::new("new.txt")).unwrap();
        index.write().unwrap();
        index.write_tree().unwrap()
    };
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&head_commit])
        .unwrap();

    // Give FS events time to propagate (FSEvents on macOS can batch with delay)
    std::thread::sleep(Duration::from_secs(2));

    let changed = watcher.recv_events(&mut debounce, Duration::from_secs(1));
    assert!(
        changed.contains(&repo_path),
        "Should detect repo change after commit, got: {:?}",
        changed
    );
}

#[test]
fn test_watcher_debounce() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path().join("myrepo");
    std::fs::create_dir_all(&repo_path).unwrap();
    let repo = git2::Repository::init(&repo_path).unwrap();

    // Create initial commit
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    let tree_id = {
        let mut index = repo.index().unwrap();
        std::fs::write(repo_path.join("README.md"), "# test").unwrap();
        index.add_path(Path::new("README.md")).unwrap();
        index.write().unwrap();
        index.write_tree().unwrap()
    };
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
        .unwrap();

    let watcher = blackbox::watcher::RepoWatcher::new(&[repo_path.clone()]).unwrap();
    let mut debounce = HashMap::new();

    // Set debounce entry to "just now" — simulates recent poll
    debounce.insert(repo_path.clone(), Instant::now());

    // Make a commit
    let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
    let tree_id = {
        let mut index = repo.index().unwrap();
        std::fs::write(repo_path.join("new.txt"), "data").unwrap();
        index.add_path(Path::new("new.txt")).unwrap();
        index.write().unwrap();
        index.write_tree().unwrap()
    };
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&head_commit])
        .unwrap();

    std::thread::sleep(Duration::from_millis(500));

    // Should be debounced (last poll was < 1s ago)
    let changed = watcher.recv_events(&mut debounce, Duration::from_millis(200));
    assert!(
        changed.is_empty(),
        "Should debounce events within 1s, got: {:?}",
        changed
    );
}

// --- US-018d: Worktree watcher support ---

/// Helper: create a main repo + manual worktree (same pattern as scanner_test.rs)
fn create_repo_with_worktree(tmp: &Path, wt_name: &str) -> (PathBuf, PathBuf) {
    let main_path = tmp.join("main_repo");
    let repo = git2::Repository::init(&main_path).unwrap();
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();

    let wt_path = tmp.join(wt_name);
    std::fs::create_dir_all(&wt_path).unwrap();
    let wt_gitdir = main_path.join(format!(".git/worktrees/{}", wt_name));
    std::fs::create_dir_all(&wt_gitdir).unwrap();
    std::fs::write(wt_gitdir.join("HEAD"), "ref: refs/heads/feat\n").unwrap();
    std::fs::write(wt_gitdir.join("commondir"), "../..\n").unwrap();
    // Absolute gitdir pointer
    std::fs::write(
        wt_path.join(".git"),
        format!("gitdir: {}\n", wt_gitdir.display()),
    )
    .unwrap();

    (main_path, wt_path)
}

#[test]
fn test_watcher_regular_repo_watches_git_and_refs() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path().join("myrepo");
    std::fs::create_dir_all(&repo_path).unwrap();
    git2::Repository::init(&repo_path).unwrap();

    let watcher = blackbox::watcher::RepoWatcher::new(&[repo_path.clone()]).unwrap();
    let dirs = watcher.watched_dirs();
    let canon = std::fs::canonicalize(&repo_path).unwrap();

    // Regular repo: should watch .git/ and .git/refs/heads/
    let has_git_dir = dirs.iter().any(|d| *d == canon.join(".git"));
    let has_refs_dir = dirs.iter().any(|d| *d == canon.join(".git/refs/heads"));
    assert!(has_git_dir, "Regular repo should watch .git/, got: {:?}", dirs);
    assert!(has_refs_dir, "Regular repo should watch .git/refs/heads/, got: {:?}", dirs);
}

#[test]
fn test_watcher_worktree_watches_head_only() {
    let tmp = TempDir::new().unwrap();
    let (main_path, wt_path) = create_repo_with_worktree(tmp.path(), "feat");

    let watcher = blackbox::watcher::RepoWatcher::new(&[wt_path.clone()]).unwrap();
    let dirs = watcher.watched_dirs();

    // Worktree: should watch resolved_gitdir/ only (the .git/worktrees/<name> dir)
    let canon_gitdir = std::fs::canonicalize(main_path.join(".git/worktrees/feat")).unwrap();
    assert_eq!(dirs.len(), 1, "Worktree should watch exactly 1 dir, got: {:?}", dirs);
    assert_eq!(
        dirs[0],
        canon_gitdir.as_path(),
        "Worktree should watch resolved gitdir"
    );
}

#[test]
fn test_watcher_mixed_regular_and_worktree() {
    let tmp = TempDir::new().unwrap();
    let (main_path, wt_path) = create_repo_with_worktree(tmp.path(), "feat");

    // Watch both main repo and worktree
    let watcher = blackbox::watcher::RepoWatcher::new(&[main_path.clone(), wt_path.clone()]).unwrap();
    let dirs = watcher.watched_dirs();

    // Main repo: .git/ + .git/refs/heads/ = 2 entries
    // Worktree: resolved_gitdir/ = 1 entry
    // Total: 3
    assert_eq!(dirs.len(), 3, "Should have 3 watched dirs, got: {:?}", dirs);

    let canon_main = std::fs::canonicalize(&main_path).unwrap();
    let has_main_git = dirs.iter().any(|d| *d == canon_main.join(".git"));
    let has_main_refs = dirs.iter().any(|d| *d == canon_main.join(".git/refs/heads"));
    assert!(has_main_git, "Should watch main repo .git/");
    assert!(has_main_refs, "Should watch main repo .git/refs/heads/");
}

#[test]
fn test_stale_worktree_removed_from_repo_states() {
    let tmp = TempDir::new().unwrap();
    let (main_path, wt_path) = create_repo_with_worktree(tmp.path(), "feat");

    // Simulate poller state: main repo + worktree entries
    let mut repo_states: HashMap<PathBuf, blackbox::git_ops::RepoState> = HashMap::new();
    let canon_main = std::fs::canonicalize(&main_path).unwrap();
    repo_states.insert(
        main_path.clone(),
        blackbox::git_ops::RepoState {
            main_repo_path: main_path.clone(),
            ..Default::default()
        },
    );
    repo_states.insert(
        wt_path.clone(),
        blackbox::git_ops::RepoState {
            main_repo_path: canon_main,
            ..Default::default()
        },
    );
    assert_eq!(repo_states.len(), 2);

    // Delete the worktree's .git file (simulates worktree deletion)
    std::fs::remove_file(wt_path.join(".git")).unwrap();

    // Stale check should remove the worktree entry
    let stale = blackbox::poller::remove_stale_worktrees(&mut repo_states);
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0], wt_path);
    assert_eq!(repo_states.len(), 1);
    assert!(repo_states.contains_key(&main_path));
}

#[test]
fn test_stale_check_ignores_regular_repos() {
    let tmp = TempDir::new().unwrap();
    let repo_path = tmp.path().join("myrepo");
    std::fs::create_dir_all(&repo_path).unwrap();
    git2::Repository::init(&repo_path).unwrap();

    let mut repo_states: HashMap<PathBuf, blackbox::git_ops::RepoState> = HashMap::new();
    repo_states.insert(
        repo_path.clone(),
        blackbox::git_ops::RepoState {
            main_repo_path: repo_path.clone(),
            ..Default::default()
        },
    );

    // Regular repo: main_repo_path == key, should not be flagged as stale
    let stale = blackbox::poller::remove_stale_worktrees(&mut repo_states);
    assert!(stale.is_empty());
    assert_eq!(repo_states.len(), 1);
}
