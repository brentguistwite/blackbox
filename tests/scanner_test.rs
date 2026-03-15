use blackbox::repo_scanner::discover_repos;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_discover_finds_git_repos() {
    let tmp = TempDir::new().unwrap();
    // Create two repos
    fs::create_dir_all(tmp.path().join("repo_a/.git")).unwrap();
    fs::create_dir_all(tmp.path().join("repo_b/.git")).unwrap();

    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 2);
    assert!(repos.contains(&tmp.path().join("repo_a")));
    assert!(repos.contains(&tmp.path().join("repo_b")));
}

#[test]
fn test_discover_finds_nested_repos() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("deep/nested/repo/.git")).unwrap();

    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], tmp.path().join("deep/nested/repo"));
}

#[test]
fn test_discover_skips_node_modules() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("node_modules/pkg/.git")).unwrap();
    fs::create_dir_all(tmp.path().join("real_repo/.git")).unwrap();

    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], tmp.path().join("real_repo"));
}

#[test]
fn test_discover_skips_target_and_vendor() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("target/debug/.git")).unwrap();
    fs::create_dir_all(tmp.path().join("vendor/dep/.git")).unwrap();
    fs::create_dir_all(tmp.path().join(".build/out/.git")).unwrap();
    fs::create_dir_all(tmp.path().join("good/.git")).unwrap();

    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], tmp.path().join("good"));
}

#[test]
fn test_discover_skips_bare_repos() {
    let tmp = TempDir::new().unwrap();
    // Create a proper non-bare repo via git2
    let repo_path = tmp.path().join("normal");
    git2::Repository::init(&repo_path).unwrap();

    // Create a bare repo
    let bare_path = tmp.path().join("bare");
    git2::Repository::init_bare(&bare_path).unwrap();

    let repos = discover_repos(&[tmp.path().to_path_buf()]);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], repo_path);
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
    fs::create_dir_all(tmp1.path().join("repo1/.git")).unwrap();
    fs::create_dir_all(tmp2.path().join("repo2/.git")).unwrap();

    let repos = discover_repos(&[tmp1.path().to_path_buf(), tmp2.path().to_path_buf()]);
    assert_eq!(repos.len(), 2);
}
