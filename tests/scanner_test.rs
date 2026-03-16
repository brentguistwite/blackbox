use blackbox::repo_scanner::{auto_scan_repos_from, discover_repos};
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
