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
