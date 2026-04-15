use blackbox::db;
use blackbox::repo_deep_dive::{
    compute_branch_activity, compute_language_breakdown, compute_time_invested, compute_top_files,
    fetch_repo_pr_history, find_db_repo_path, query_repo_all_time, resolve_repo_path,
    RepoAllTimeData,
};
use tempfile::TempDir;

/// Helper: create a commit in a git2 repo with given files.
/// `files` is a slice of (relative_path, content_bytes).
fn commit_files(repo: &git2::Repository, files: &[(&str, &[u8])], message: &str) {
    let mut index = repo.index().unwrap();
    for (path, content) in files {
        let dir = std::path::Path::new(path).parent();
        if let Some(d) = dir {
            let full = repo.workdir().unwrap().join(d);
            std::fs::create_dir_all(full).ok();
        }
        let full_path = repo.workdir().unwrap().join(path);
        std::fs::write(&full_path, content).unwrap();
        index.add_path(std::path::Path::new(path)).unwrap();
    }
    index.write().unwrap();
    let tree_oid = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let sig = git2::Signature::now("test", "test@test.com").unwrap();
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .unwrap();
}

#[test]
fn resolve_repo_path_valid_git_repo() {
    let tmp = TempDir::new().unwrap();
    git2::Repository::init(tmp.path()).unwrap();
    let result = resolve_repo_path(tmp.path().to_str().unwrap());
    assert!(result.is_ok());
    let resolved = result.unwrap();
    assert!(resolved.is_absolute());
}

#[test]
fn resolve_repo_path_nonexistent_path() {
    let result = resolve_repo_path("/tmp/definitely-does-not-exist-xyz-123");
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("Path not found"), "got: {msg}");
}

#[test]
fn resolve_repo_path_non_git_dir() {
    let tmp = TempDir::new().unwrap();
    // no git init — just a plain dir
    let result = resolve_repo_path(tmp.path().to_str().unwrap());
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("Not a git repository"), "got: {msg}");
}

#[test]
fn find_db_repo_path_empty_db() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();
    let result = find_db_repo_path(&conn, std::path::Path::new("/some/repo")).unwrap();
    assert_eq!(result, None);
}

#[test]
fn find_db_repo_path_with_matching_row() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();
    db::insert_activity(
        &conn,
        "/home/user/myrepo",
        "commit",
        Some("main"),
        None,
        Some("abc123"),
        Some("author"),
        Some("msg"),
        "2024-01-01T00:00:00Z",
    )
    .unwrap();
    let result = find_db_repo_path(&conn, std::path::Path::new("/home/user/myrepo")).unwrap();
    assert_eq!(result, Some("/home/user/myrepo".to_string()));
}

// --- US-002: query_repo_all_time ---

#[test]
fn query_repo_all_time_empty_db() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();
    let data = query_repo_all_time(&conn, "/no/such/repo").unwrap();
    assert_eq!(data.repo_path, "/no/such/repo");
    assert!(data.events.is_empty());
    assert!(data.reviews.is_empty());
    assert!(data.ai_sessions.is_empty());
}

#[test]
fn query_repo_all_time_returns_events() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();
    let repo = "/home/user/myrepo";
    db::insert_activity(&conn, repo, "commit", Some("main"), None, Some("aaa"), Some("a"), Some("first"), "2024-01-01T00:00:00Z").unwrap();
    db::insert_activity(&conn, repo, "commit", Some("main"), None, Some("bbb"), Some("a"), Some("second"), "2024-01-02T00:00:00Z").unwrap();
    db::insert_activity(&conn, repo, "branch_switch", Some("feat"), None, None, None, None, "2024-01-03T00:00:00Z").unwrap();
    // different repo — should not appear
    db::insert_activity(&conn, "/other/repo", "commit", Some("main"), None, Some("ccc"), Some("a"), Some("other"), "2024-01-01T00:00:00Z").unwrap();

    let data = query_repo_all_time(&conn, repo).unwrap();
    assert_eq!(data.events.len(), 3);
    // ordered by timestamp ASC
    assert_eq!(data.events[0].commit_hash.as_deref(), Some("aaa"));
    assert_eq!(data.events[2].event_type, "branch_switch");
}

#[test]
fn query_repo_all_time_returns_reviews() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();
    let repo = "/home/user/myrepo";
    db::insert_review(&conn, repo, 1, "PR one", "https://gh/1", "APPROVED", "2024-02-01T00:00:00Z").unwrap();
    db::insert_review(&conn, repo, 2, "PR two", "https://gh/2", "COMMENTED", "2024-02-02T00:00:00Z").unwrap();
    // different repo
    db::insert_review(&conn, "/other/repo", 3, "PR three", "https://gh/3", "APPROVED", "2024-02-01T00:00:00Z").unwrap();

    let data = query_repo_all_time(&conn, repo).unwrap();
    assert_eq!(data.reviews.len(), 2);
    assert_eq!(data.reviews[0].pr_number, 1);
    assert_eq!(data.reviews[1].pr_title, "PR two");
}

#[test]
fn query_repo_all_time_returns_ai_sessions() {
    let tmp = TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("test.db")).unwrap();
    let repo = "/home/user/myrepo";
    db::insert_ai_session(&conn, "claude-code", repo, "sess-1", "2024-03-01T00:00:00Z").unwrap();
    db::update_session_ended(&conn, "sess-1", "2024-03-01T01:00:00Z", Some(5)).unwrap();
    db::insert_ai_session(&conn, "claude-code", repo, "sess-2", "2024-03-02T00:00:00Z").unwrap();
    // different repo
    db::insert_ai_session(&conn, "claude-code", "/other/repo", "sess-3", "2024-03-01T00:00:00Z").unwrap();

    let data = query_repo_all_time(&conn, repo).unwrap();
    assert_eq!(data.ai_sessions.len(), 2);
    assert_eq!(data.ai_sessions[0].session_id, "sess-1");
    assert!(data.ai_sessions[0].ended_at.is_some());
    assert!(data.ai_sessions[1].ended_at.is_none());
}

// --- US-003: compute_language_breakdown ---

#[test]
fn language_breakdown_single_rs_file() {
    let tmp = TempDir::new().unwrap();
    let repo = git2::Repository::init(tmp.path()).unwrap();
    commit_files(&repo, &[("main.rs", b"fn main() {\n    println!(\"hi\");\n}\n")], "init");

    let result = compute_language_breakdown(tmp.path()).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].language, "Rust");
    assert_eq!(result[0].file_count, 1);
    // 3 lines of content + split produces 4 segments, but "fn main...\nprintln...\n}\n" = 4 splits
    assert!(result[0].line_count >= 3);
    assert!((result[0].percent - 100.0).abs() < 0.01);
}

#[test]
fn language_breakdown_two_languages() {
    let tmp = TempDir::new().unwrap();
    let repo = git2::Repository::init(tmp.path()).unwrap();
    commit_files(
        &repo,
        &[
            ("main.rs", b"line1\nline2\n"),
            ("script.py", b"print('hi')\n"),
        ],
        "init",
    );

    let result = compute_language_breakdown(tmp.path()).unwrap();
    assert_eq!(result.len(), 2);
    let total_pct: f64 = result.iter().map(|r| r.percent).sum();
    assert!((total_pct - 100.0).abs() < 0.01);
}

#[test]
fn language_breakdown_skips_binary() {
    let tmp = TempDir::new().unwrap();
    let repo = git2::Repository::init(tmp.path()).unwrap();
    commit_files(
        &repo,
        &[
            ("main.rs", b"fn main() {}\n"),
            ("image.png", &[0x89, 0x50, 0x4E, 0x47, 0x00, 0x00, 0x00]),
        ],
        "init",
    );

    let result = compute_language_breakdown(tmp.path()).unwrap();
    // only Rust should appear, binary png skipped
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].language, "Rust");
}

#[test]
fn language_breakdown_empty_repo() {
    let tmp = TempDir::new().unwrap();
    git2::Repository::init(tmp.path()).unwrap();
    let result = compute_language_breakdown(tmp.path()).unwrap();
    assert!(result.is_empty());
}

#[test]
fn language_breakdown_skips_no_extension() {
    let tmp = TempDir::new().unwrap();
    let repo = git2::Repository::init(tmp.path()).unwrap();
    commit_files(
        &repo,
        &[
            ("Makefile", b"all:\n\techo hi\n"),
            ("main.rs", b"fn main() {}\n"),
        ],
        "init",
    );

    let result = compute_language_breakdown(tmp.path()).unwrap();
    // only Rust — Makefile has no extension, skipped
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].language, "Rust");
}

// --- US-004: compute_top_files ---

#[test]
fn top_files_same_file_multiple_commits() {
    let tmp = TempDir::new().unwrap();
    let repo = git2::Repository::init(tmp.path()).unwrap();
    commit_files(&repo, &[("file.rs", b"v1\n")], "first");
    commit_files(&repo, &[("file.rs", b"v2\n")], "second");
    commit_files(&repo, &[("file.rs", b"v3\n")], "third");

    let result = compute_top_files(tmp.path(), 10).unwrap();
    assert!(!result.is_empty());
    assert_eq!(result[0].path, "file.rs");
    assert_eq!(result[0].change_count, 3);
}

#[test]
fn top_files_empty_repo() {
    let tmp = TempDir::new().unwrap();
    git2::Repository::init(tmp.path()).unwrap();
    let result = compute_top_files(tmp.path(), 10).unwrap();
    assert!(result.is_empty());
}

// --- US-005: compute_time_invested ---

#[test]
fn time_invested_empty_data() {
    let data = RepoAllTimeData {
        repo_path: "/test".to_string(),
        events: vec![],
        reviews: vec![],
        ai_sessions: vec![],
    };
    let dur = compute_time_invested(&data, 120, 30);
    assert_eq!(dur.num_seconds(), 0);
}

#[test]
fn time_invested_with_commits() {
    use blackbox::query::ActivityEvent;
    use chrono::{Duration, TimeZone, Utc};

    let t1 = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
    let t2 = t1 + Duration::minutes(10);
    let t3 = t2 + Duration::minutes(15);

    let data = RepoAllTimeData {
        repo_path: "/test".to_string(),
        events: vec![
            ActivityEvent { event_type: "commit".into(), branch: Some("main".into()), commit_hash: Some("aaa".into()), message: Some("a".into()), timestamp: t1 },
            ActivityEvent { event_type: "commit".into(), branch: Some("main".into()), commit_hash: Some("bbb".into()), message: Some("b".into()), timestamp: t2 },
            ActivityEvent { event_type: "commit".into(), branch: Some("main".into()), commit_hash: Some("ccc".into()), message: Some("c".into()), timestamp: t3 },
        ],
        reviews: vec![],
        ai_sessions: vec![],
    };
    let dur = compute_time_invested(&data, 120, 30);
    // estimate_time_v2 with 3 commits 10+15 min apart: adaptive gap from median
    // Should be > 0
    assert!(dur.num_minutes() > 0, "expected positive duration, got {}", dur.num_minutes());
}

#[test]
fn time_invested_with_ai_sessions() {
    use blackbox::query::{ActivityEvent, AiSessionInfo};
    use chrono::{Duration, TimeZone, Utc};

    let t1 = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
    let t2 = t1 + Duration::minutes(30);

    let data = RepoAllTimeData {
        repo_path: "/test".to_string(),
        events: vec![
            ActivityEvent { event_type: "commit".into(), branch: Some("main".into()), commit_hash: Some("aaa".into()), message: Some("a".into()), timestamp: t1 },
        ],
        reviews: vec![],
        ai_sessions: vec![
            AiSessionInfo {
                tool: "claude-code".into(),
                session_id: "s1".into(),
                started_at: t1,
                ended_at: Some(t2),
                last_active_at: None,
                duration: t2 - t1,
                turns: Some(3),
            },
        ],
    };
    let dur = compute_time_invested(&data, 120, 30);
    // Should include AI session time
    assert!(dur.num_minutes() >= 30, "expected >= 30min with AI session, got {}", dur.num_minutes());
}

// --- US-007: compute_branch_activity ---

#[test]
fn branch_activity_commits_on_two_branches() {
    use blackbox::query::ActivityEvent;
    use chrono::{Duration, TimeZone, Utc};

    let t1 = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
    let t2 = t1 + Duration::minutes(10);
    let t3 = t2 + Duration::minutes(20);

    let data = RepoAllTimeData {
        repo_path: "/test".to_string(),
        events: vec![
            ActivityEvent { event_type: "commit".into(), branch: Some("main".into()), commit_hash: Some("aaa".into()), message: Some("a".into()), timestamp: t1 },
            ActivityEvent { event_type: "commit".into(), branch: Some("main".into()), commit_hash: Some("bbb".into()), message: Some("b".into()), timestamp: t2 },
            ActivityEvent { event_type: "commit".into(), branch: Some("feature/x".into()), commit_hash: Some("ccc".into()), message: Some("c".into()), timestamp: t3 },
        ],
        reviews: vec![],
        ai_sessions: vec![],
    };
    let branches = compute_branch_activity(&data);
    assert_eq!(branches.len(), 2);
    // sorted by last_active desc — feature/x (t3) first
    assert_eq!(branches[0].name, "feature/x");
    assert_eq!(branches[0].commit_count, 1);
    assert_eq!(branches[1].name, "main");
    assert_eq!(branches[1].commit_count, 2);
}

#[test]
fn branch_activity_includes_branch_switch_with_zero_commits() {
    use blackbox::query::ActivityEvent;
    use chrono::{TimeZone, Utc};

    let t1 = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2024, 1, 1, 11, 0, 0).unwrap();

    let data = RepoAllTimeData {
        repo_path: "/test".to_string(),
        events: vec![
            ActivityEvent { event_type: "commit".into(), branch: Some("main".into()), commit_hash: Some("aaa".into()), message: Some("a".into()), timestamp: t1 },
            ActivityEvent { event_type: "branch_switch".into(), branch: Some("feature/y".into()), commit_hash: None, message: None, timestamp: t2 },
        ],
        reviews: vec![],
        ai_sessions: vec![],
    };
    let branches = compute_branch_activity(&data);
    assert_eq!(branches.len(), 2);
    // feature/y last_active=t2 > main last_active=t1, so feature/y first
    assert_eq!(branches[0].name, "feature/y");
    assert_eq!(branches[0].commit_count, 0);
    assert_eq!(branches[1].name, "main");
    assert_eq!(branches[1].commit_count, 1);
}

#[test]
fn branch_activity_empty_events() {
    let data = RepoAllTimeData {
        repo_path: "/test".to_string(),
        events: vec![],
        reviews: vec![],
        ai_sessions: vec![],
    };
    let branches = compute_branch_activity(&data);
    assert!(branches.is_empty());
}

// --- US-006: fetch_repo_pr_history ---

#[test]
fn pr_history_empty_data_no_gh() {
    let data = RepoAllTimeData {
        repo_path: "/test".to_string(),
        events: vec![],
        reviews: vec![],
        ai_sessions: vec![],
    };
    let prs = fetch_repo_pr_history("/nonexistent/repo", &data);
    assert!(prs.is_empty());
}

#[test]
fn pr_history_from_reviews_only() {
    use blackbox::query::ReviewInfo;
    use chrono::{TimeZone, Utc};

    let t1 = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2024, 2, 2, 0, 0, 0).unwrap();

    let data = RepoAllTimeData {
        repo_path: "/test".to_string(),
        events: vec![],
        reviews: vec![
            ReviewInfo { pr_number: 42, pr_title: "Add feature X".into(), action: "APPROVED".into(), reviewed_at: t1 },
            ReviewInfo { pr_number: 42, pr_title: "Add feature X".into(), action: "COMMENTED".into(), reviewed_at: t2 },
            ReviewInfo { pr_number: 10, pr_title: "Fix bug Y".into(), action: "APPROVED".into(), reviewed_at: t1 },
        ],
        ai_sessions: vec![],
    };
    // With a nonexistent path, gh will fail — should fall back to review-only entries
    let prs = fetch_repo_pr_history("/nonexistent/repo", &data);
    assert_eq!(prs.len(), 2);
    // sorted by number desc
    assert_eq!(prs[0].number, 42);
    assert_eq!(prs[0].title, "Add feature X");
    assert_eq!(prs[0].state, "REVIEWED");
    assert_eq!(prs[1].number, 10);
}

// --- US-015: CLI integration tests ---

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

/// Helper: set up isolated XDG dirs with config + DB, return (config_dir, data_dir)
fn setup_cli_env(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    // Create config via init
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
        .assert()
        .success();

    // Ensure DB exists
    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = db::open_db(&db_dir.join("blackbox.db")).unwrap();

    (config_dir, data_dir)
}

#[test]
fn cli_repo_pretty_exits_0_shows_repo_name() {
    let tmp = TempDir::new().unwrap();
    let (config_dir, data_dir) = setup_cli_env(&tmp);

    // Create a git repo with a commit
    let repo_dir = tmp.path().join("myrepo");
    fs::create_dir_all(&repo_dir).unwrap();
    let repo = git2::Repository::init(&repo_dir).unwrap();
    commit_files(&repo, &[("main.rs", b"fn main() {}\n")], "init");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["repo", repo_dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("myrepo"));
}

#[test]
fn cli_repo_json_exits_0_valid_json_keys() {
    let tmp = TempDir::new().unwrap();
    let (config_dir, data_dir) = setup_cli_env(&tmp);

    let repo_dir = tmp.path().join("jsonrepo");
    fs::create_dir_all(&repo_dir).unwrap();
    let repo = git2::Repository::init(&repo_dir).unwrap();
    commit_files(&repo, &[("lib.rs", b"pub fn hello() {}\n")], "init");

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["repo", repo_dir.to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "exit 0 expected");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout: {stdout}"));
    assert!(parsed.get("repo_name").is_some(), "missing repo_name");
    assert!(parsed.get("total_commits").is_some(), "missing total_commits");
    assert!(parsed.get("total_estimated_minutes").is_some(), "missing total_estimated_minutes");
    assert!(parsed.get("languages").is_some(), "missing languages");
    assert!(parsed.get("top_files").is_some(), "missing top_files");
    assert!(parsed.get("branches").is_some(), "missing branches");
}

#[test]
fn cli_repo_invalid_path_exits_nonzero() {
    let tmp = TempDir::new().unwrap();
    let (config_dir, data_dir) = setup_cli_env(&tmp);

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["repo", "/tmp/definitely-not-a-repo-xyz-999"])
        .output()
        .unwrap();

    assert!(!output.status.success(), "should exit non-zero");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Not a git repository") || combined.contains("Path not found"),
        "expected error message, got: {combined}"
    );
}

#[test]
fn cli_repo_untracked_shows_indicator() {
    let tmp = TempDir::new().unwrap();
    let (config_dir, data_dir) = setup_cli_env(&tmp);

    // Repo with commits but not in DB — should show untracked
    let repo_dir = tmp.path().join("untracked-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    let repo = git2::Repository::init(&repo_dir).unwrap();
    commit_files(&repo, &[("app.py", b"print('hi')\n")], "init");

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["repo", repo_dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("untracked"));
}
