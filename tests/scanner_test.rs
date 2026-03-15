use blackbox::repo_scanner::discover_repos;
use std::path::PathBuf;
use tempfile::TempDir;

fn init_repo(path: &std::path::Path) {
    git2::Repository::init(path).unwrap();
}

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
