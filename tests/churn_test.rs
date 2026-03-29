use blackbox::churn::diff_commit_stats;
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
