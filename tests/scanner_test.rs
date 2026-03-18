use blackbox::repo_scanner::{
    auto_scan_repos_from, discover_repos, is_valid_gitdir_file, is_worktree, resolve_main_repo,
    scan_directory,
};
use std::path::PathBuf;
use tempfile::TempDir;

fn init_repo(path: &std::path::Path) {
    git2::Repository::init(path).unwrap();
}

// --- existing discover_repos tests ---

#[test]
fn test_discover_finds_git_repos() {
    let tmp = TempDir::new().unwrap();
    init_repo(&tmp.path().join("repo_a"));
    init_repo(&tmp.path().join("repo_b"));

    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 2);
    assert!(repos.contains(&tmp.path().join("repo_a")));
    assert!(repos.contains(&tmp.path().join("repo_b")));
}

#[test]
fn test_discover_finds_nested_repos() {
    let tmp = TempDir::new().unwrap();
    init_repo(&tmp.path().join("deep/nested/repo"));

    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], tmp.path().join("deep/nested/repo"));
}

#[test]
fn test_discover_skips_node_modules() {
    let tmp = TempDir::new().unwrap();
    init_repo(&tmp.path().join("node_modules/pkg"));
    init_repo(&tmp.path().join("real_repo"));

    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], tmp.path().join("real_repo"));
}

#[test]
fn test_discover_skips_target_and_vendor() {
    let tmp = TempDir::new().unwrap();
    init_repo(&tmp.path().join("target/debug"));
    init_repo(&tmp.path().join("vendor/dep"));
    init_repo(&tmp.path().join(".build/out"));
    init_repo(&tmp.path().join("good"));

    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], tmp.path().join("good"));
}

#[test]
fn test_discover_skips_bare_repos() {
    let tmp = TempDir::new().unwrap();
    init_repo(&tmp.path().join("normal"));
    git2::Repository::init_bare(tmp.path().join("bare")).unwrap();

    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], tmp.path().join("normal"));
}

#[test]
fn test_discover_empty_dirs() {
    let tmp = TempDir::new().unwrap();
    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert!(repos.is_empty());
}

#[test]
fn test_discover_nonexistent_dir() {
    let repos = discover_repos(&[PathBuf::from("/nonexistent/path/xyz")]);
    assert!(repos.is_empty());
}

#[test]
fn test_discover_multiple_watch_dirs() {
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    init_repo(&tmp1.path().join("repo1"));
    init_repo(&tmp2.path().join("repo2"));

    let repos = discover_repos(&[tmp1.path().to_path_buf(), tmp2.path().to_path_buf()]);
    assert_eq!(repos.len(), 2);
}

// --- auto_scan_repos tests ---

#[test]
fn test_auto_scan_finds_repos_in_well_known_dirs() {
    let home = TempDir::new().unwrap();
    // Create repos in well-known dirs
    init_repo(&home.path().join("Documents/project1"));
    init_repo(&home.path().join("code/myapp"));

    let results = auto_scan_repos_from(home.path());
    assert_eq!(results.len(), 2);

    let doc_entry = results.iter().find(|(p, _)| p.ends_with("Documents")).unwrap();
    assert_eq!(doc_entry.1.len(), 1);
    assert!(doc_entry.1[0].ends_with("project1"));

    let code_entry = results.iter().find(|(p, _)| p.ends_with("code")).unwrap();
    assert_eq!(code_entry.1.len(), 1);
    assert!(code_entry.1[0].ends_with("myapp"));
}

#[test]
fn test_auto_scan_skips_nonexistent_well_known_dirs() {
    let home = TempDir::new().unwrap();
    // Only create one well-known dir
    init_repo(&home.path().join("projects/repo1"));

    let results = auto_scan_repos_from(home.path());
    assert_eq!(results.len(), 1);
    assert!(results[0].0.ends_with("projects"));
}

#[test]
fn test_auto_scan_finds_repos_in_home_children() {
    let home = TempDir::new().unwrap();
    // Create a non-well-known child dir with a repo
    init_repo(&home.path().join("custom_dir/myrepo"));

    let results = auto_scan_repos_from(home.path());
    assert_eq!(results.len(), 1);
    assert!(results[0].0.ends_with("custom_dir"));
    assert!(results[0].1[0].ends_with("myrepo"));
}

#[test]
fn test_auto_scan_skips_hidden_dirs() {
    let home = TempDir::new().unwrap();
    init_repo(&home.path().join(".hidden/secret_repo"));
    init_repo(&home.path().join("visible/repo"));

    let results = auto_scan_repos_from(home.path());
    assert_eq!(results.len(), 1);
    assert!(results[0].0.ends_with("visible"));
}

#[test]
fn test_auto_scan_allows_config_and_local() {
    let home = TempDir::new().unwrap();
    init_repo(&home.path().join(".config/some_repo"));
    init_repo(&home.path().join(".local/another_repo"));

    let results = auto_scan_repos_from(home.path());
    assert_eq!(results.len(), 2);
    let names: Vec<String> = results.iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string()).collect();
    assert!(names.contains(&".config".to_string()));
    assert!(names.contains(&".local".to_string()));
}

#[test]
fn test_auto_scan_skips_skip_dirs() {
    let home = TempDir::new().unwrap();
    init_repo(&home.path().join("code/myapp"));
    init_repo(&home.path().join("code/node_modules/dep"));
    init_repo(&home.path().join("code/target/build"));

    let results = auto_scan_repos_from(home.path());
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].1.len(), 1);
    assert!(results[0].1[0].ends_with("myapp"));
}

#[test]
fn test_auto_scan_empty_home() {
    let home = TempDir::new().unwrap();
    let results = auto_scan_repos_from(home.path());
    assert!(results.is_empty());
}

#[test]
fn test_auto_scan_groups_by_parent() {
    let home = TempDir::new().unwrap();
    init_repo(&home.path().join("Documents/repo1"));
    init_repo(&home.path().join("Documents/repo2"));
    init_repo(&home.path().join("Documents/sub/repo3"));

    let results = auto_scan_repos_from(home.path());
    assert_eq!(results.len(), 1);
    let (parent, repos) = &results[0];
    assert!(parent.ends_with("Documents"));
    assert_eq!(repos.len(), 3);
}

#[test]
fn test_auto_scan_handles_permission_errors() {
    // Non-existent paths should be skipped gracefully
    let home = TempDir::new().unwrap();
    let results = auto_scan_repos_from(&home.path().join("nonexistent"));
    assert!(results.is_empty());
}

// --- US-015: fast path + worktree .git file detection ---

#[test]
fn test_fast_path_git_dir() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path()); // .git dir at root level
    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], tmp.path().to_path_buf());
}

#[test]
fn test_fast_path_git_file_worktree() {
    let tmp = TempDir::new().unwrap();
    // Create a .git file (worktree pointer) — fast path doesn't validate the target
    std::fs::write(
        tmp.path().join(".git"),
        "gitdir: /some/repo/.git/worktrees/feat\n",
    )
    .unwrap();
    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], tmp.path().to_path_buf());
}

#[test]
fn test_fast_path_malformed_git_file_falls_through() {
    let tmp = TempDir::new().unwrap();
    // Malformed .git file (no 'gitdir:' prefix) — falls through to WalkDir
    std::fs::write(tmp.path().join(".git"), "not a valid gitdir pointer\n").unwrap();
    // Real repo in subdirectory should be found by WalkDir
    init_repo(&tmp.path().join("real_repo"));
    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], tmp.path().join("real_repo"));
}

#[test]
fn test_walkdir_discovers_worktree_git_files() {
    let tmp = TempDir::new().unwrap();

    // Create main repo with initial commit
    let main_path = tmp.path().join("parent/main_repo");
    let repo = git2::Repository::init(&main_path).unwrap();
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();

    // Manually create worktree structure
    let wt_path = tmp.path().join("parent/my_worktree");
    std::fs::create_dir_all(&wt_path).unwrap();
    let wt_gitdir = main_path.join(".git/worktrees/my_worktree");
    std::fs::create_dir_all(&wt_gitdir).unwrap();
    std::fs::write(
        wt_path.join(".git"),
        format!("gitdir: {}\n", wt_gitdir.display()),
    )
    .unwrap();
    std::fs::write(wt_gitdir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(wt_gitdir.join("commondir"), "../..\n").unwrap();

    let repos = discover_repos(&[tmp.path().join("parent")]);
    assert!(repos.contains(&main_path), "should find main repo");
    assert!(repos.contains(&wt_path), "should find worktree");
    assert_eq!(repos.len(), 2);
}

#[test]
fn test_walkdir_skips_malformed_git_files() {
    let tmp = TempDir::new().unwrap();

    // Real repo
    init_repo(&tmp.path().join("parent/real_repo"));

    // Malformed .git file in a subdir — should be skipped
    let bad_path = tmp.path().join("parent/bad_worktree");
    std::fs::create_dir_all(&bad_path).unwrap();
    std::fs::write(bad_path.join(".git"), "garbage content\n").unwrap();

    let repos = discover_repos(&[tmp.path().join("parent")]);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], tmp.path().join("parent/real_repo"));
}

#[test]
fn test_is_valid_gitdir_file() {
    let tmp = TempDir::new().unwrap();

    let valid = tmp.path().join("valid");
    std::fs::write(&valid, "gitdir: /some/path\n").unwrap();
    assert!(is_valid_gitdir_file(&valid));

    let invalid = tmp.path().join("invalid");
    std::fs::write(&invalid, "not valid\n").unwrap();
    assert!(!is_valid_gitdir_file(&invalid));

    let nonexistent = tmp.path().join("nonexistent");
    assert!(!is_valid_gitdir_file(&nonexistent));
}

#[test]
fn test_fast_path_discover_repos_limited_via_auto_scan() {
    // discover_repos_limited is used by auto_scan_repos_from — test via that API
    let home = TempDir::new().unwrap();
    // Put a repo root directly as a well-known dir child
    let repo_path = home.path().join("code");
    init_repo(&repo_path); // code/ is itself a repo
    let results = auto_scan_repos_from(home.path());
    // Should fast-path: code/.git is a dir → return code as a repo
    let code_entry = results.iter().find(|(p, _)| p.ends_with("code"));
    assert!(code_entry.is_some(), "should find code dir");
    assert_eq!(code_entry.unwrap().1.len(), 1);
    assert_eq!(code_entry.unwrap().1[0], repo_path);
}

// --- US-018a: resolve_main_repo + is_worktree ---

/// Helper: create a main repo with an initial commit and a manually-crafted worktree.
/// Returns (main_repo_path, worktree_path).
fn create_repo_with_worktree(tmp: &std::path::Path, wt_name: &str) -> (PathBuf, PathBuf) {
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
fn test_resolve_main_repo_absolute_gitdir() {
    let tmp = TempDir::new().unwrap();
    let (main_path, wt_path) = create_repo_with_worktree(tmp.path(), "feat");
    let resolved = resolve_main_repo(&wt_path).unwrap();
    assert_eq!(
        resolved.canonicalize().unwrap(),
        main_path.canonicalize().unwrap()
    );
}

#[test]
fn test_resolve_main_repo_relative_gitdir() {
    let tmp = TempDir::new().unwrap();
    let (main_path, wt_path) = create_repo_with_worktree(tmp.path(), "feat");
    // Overwrite .git file with relative path (wt is sibling of main_repo)
    std::fs::write(
        wt_path.join(".git"),
        "gitdir: ../main_repo/.git/worktrees/feat\n",
    )
    .unwrap();
    let resolved = resolve_main_repo(&wt_path).unwrap();
    assert_eq!(
        resolved.canonicalize().unwrap(),
        main_path.canonicalize().unwrap()
    );
}

#[test]
fn test_resolve_main_repo_nonexistent_gitdir() {
    let tmp = TempDir::new().unwrap();
    let wt_path = tmp.path().join("bad_wt");
    std::fs::create_dir_all(&wt_path).unwrap();
    std::fs::write(
        wt_path.join(".git"),
        "gitdir: /nonexistent/path/.git/worktrees/x\n",
    )
    .unwrap();
    assert!(resolve_main_repo(&wt_path).is_err());
}

#[test]
fn test_resolve_main_repo_malformed_git_file() {
    let tmp = TempDir::new().unwrap();
    let wt_path = tmp.path().join("bad_wt");
    std::fs::create_dir_all(&wt_path).unwrap();
    std::fs::write(wt_path.join(".git"), "garbage content\n").unwrap();
    assert!(resolve_main_repo(&wt_path).is_err());
}

#[test]
fn test_resolve_main_repo_validation_failure() {
    let tmp = TempDir::new().unwrap();
    // Create a gitdir structure but no .git dir at the resolved "main repo" root
    let fake_root = tmp.path().join("not_a_repo");
    let wt_gitdir = fake_root.join(".git/worktrees/feat");
    std::fs::create_dir_all(&wt_gitdir).unwrap();
    // Create a .git FILE (not dir) at fake_root so canonicalize succeeds but validation fails
    // Actually .git is already a dir since we created .git/worktrees/feat... so this validates.
    // Instead, put the worktree gitdir somewhere that doesn't resolve to a real repo.
    let dangling_gitdir = tmp.path().join("dangling/nested/deep");
    std::fs::create_dir_all(&dangling_gitdir).unwrap();
    let wt_path = tmp.path().join("wt");
    std::fs::create_dir_all(&wt_path).unwrap();
    std::fs::write(
        wt_path.join(".git"),
        format!("gitdir: {}\n", dangling_gitdir.display()),
    )
    .unwrap();
    // 3x .parent() from dangling/nested/deep → tmp root, which has no .git dir
    assert!(resolve_main_repo(&wt_path).is_err());
}

#[test]
fn test_is_worktree_regular_repo() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    assert!(is_worktree(tmp.path()).is_none());
}

#[test]
fn test_is_worktree_actual_worktree() {
    let tmp = TempDir::new().unwrap();
    let (main_path, wt_path) = create_repo_with_worktree(tmp.path(), "feat");
    let resolved = is_worktree(&wt_path);
    assert!(resolved.is_some());
    let gitdir = resolved.unwrap();
    // Should point to main_repo/.git/worktrees/feat
    assert_eq!(
        gitdir.canonicalize().unwrap(),
        main_path
            .join(".git/worktrees/feat")
            .canonicalize()
            .unwrap()
    );
}

// --- US-015b: scan_directory ---

#[test]
fn test_scan_directory_finds_repos() {
    let tmp = TempDir::new().unwrap();
    init_repo(&tmp.path().join("repo_a"));
    init_repo(&tmp.path().join("repo_b"));
    let repos = scan_directory(tmp.path());
    assert_eq!(repos.len(), 2);
}

#[test]
fn test_scan_directory_finds_nested_repos() {
    let tmp = TempDir::new().unwrap();
    init_repo(&tmp.path().join("deep/nested/repo"));
    let repos = scan_directory(tmp.path());
    assert_eq!(repos.len(), 1);
}

#[test]
fn test_scan_directory_empty() {
    let tmp = TempDir::new().unwrap();
    let repos = scan_directory(tmp.path());
    assert!(repos.is_empty());
}

#[test]
fn test_scan_directory_is_repo_root() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let repos = scan_directory(tmp.path());
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], tmp.path().to_path_buf());
}
