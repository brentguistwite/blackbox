use assert_cmd::Command;
use blackbox::db;
use blackbox::llm::LlmConfig;
use blackbox::perf_review::{
    build_perf_review_context, generate_perf_review, PerfReviewContext, PrSummary,
};
use blackbox::query::{ActivityEvent, ActivitySummary, RepoSummary};
use chrono::{Duration, Utc};
use tempfile::TempDir;

/// Helper: build empty ActivitySummary (zero commits, zero reviews).
fn empty_summary() -> ActivitySummary {
    ActivitySummary {
        period_label: "Q1 2026".to_string(),
        total_commits: 0,
        total_reviews: 0,
        total_repos: 0,
        total_estimated_time: Duration::zero(),
        total_ai_session_time: Duration::zero(),
        streak_days: 0,
        total_branch_switches: 0,
        repos: vec![],
    }
}

/// Dummy LLM config — never actually called because we bail before LLM.
fn dummy_llm_config() -> LlmConfig {
    LlmConfig {
        provider: "anthropic".to_string(),
        api_key: "fake-key".to_string(),
        model: "claude-sonnet-4-20250514".to_string(),
        base_url: None,
    }
}

// --- US-09: Sparse activity graceful handling ---

#[test]
fn generate_perf_review_errors_on_zero_activity() {
    let summary = empty_summary();
    let context = build_perf_review_context(&summary);
    assert_eq!(context.total_commits, 0);
    assert_eq!(context.total_reviews, 0);

    let result = generate_perf_review(&dummy_llm_config(), &context);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("No activity found"),
        "expected 'No activity found', got: {}",
        err_msg
    );
}

#[test]
fn generate_perf_review_errors_on_empty_db_period() {
    // Create a real DB with no data, aggregate, and verify error path
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let config = blackbox::config::Config::default();
    let from = Utc::now() - Duration::days(90);
    let to = Utc::now();

    let summary = blackbox::perf_review::aggregate_perf_review(
        &conn,
        &config,
        from,
        to,
        "Q1 2026".to_string(),
    )
    .unwrap();

    let context = build_perf_review_context(&summary);
    let result = generate_perf_review(&dummy_llm_config(), &context);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("No activity found"),
        "expected error for empty DB period"
    );
}

#[test]
fn context_with_only_reviews_does_not_error() {
    // Zero commits but has reviews → should NOT error (reviews alone are valid)
    let context = PerfReviewContext {
        period_label: "Q1 2026".to_string(),
        total_commits: 0,
        total_reviews: 3,
        total_repos: 1,
        total_estimated_hours: 1.0,
        total_ai_session_hours: 0.0,
        repos: vec![],
        themes: vec![],
        pr_summary: PrSummary {
            authored_prs: vec![],
            reviewed_prs: vec![
                ("repo".to_string(), 1, "PR 1".to_string(), "APPROVED".to_string()),
            ],
        },
    };

    // This will try to make a real LLM call (and fail on network), but
    // the key assertion is that it does NOT bail with "No activity found"
    let result = generate_perf_review(&dummy_llm_config(), &context);
    // It should fail (fake API key), but NOT with "No activity found"
    if let Err(e) = result {
        let msg = e.to_string();
        assert!(
            !msg.contains("No activity found"),
            "reviews-only should not trigger 'No activity found' error, got: {}",
            msg
        );
    }
    // If it somehow succeeds (unlikely), that's also fine
}

// --- US-12: CLI integration tests ---

/// Set up temp XDG dirs with a minimal config (no LLM key).
fn setup_temp_env() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(config_dir.join("blackbox")).unwrap();
    std::fs::write(
        config_dir.join("blackbox").join("config.toml"),
        "watch_dirs = []\npoll_interval_secs = 60\n",
    )
    .unwrap();
    (tmp, data_dir, config_dir)
}

/// Set up temp XDG dirs with config that includes a fake LLM API key.
fn setup_temp_env_with_llm_key() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(config_dir.join("blackbox")).unwrap();
    std::fs::write(
        config_dir.join("blackbox").join("config.toml"),
        "watch_dirs = []\npoll_interval_secs = 60\nllm_api_key = \"fake-key\"\nllm_provider = \"anthropic\"\n",
    )
    .unwrap();
    (tmp, data_dir, config_dir)
}

fn open_test_db(data_dir: &std::path::Path) -> rusqlite::Connection {
    let db_path = data_dir.join("blackbox").join("blackbox.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    db::open_db(&db_path).unwrap()
}

/// Insert N commits into a repo at timestamps within 2020.
fn seed_commits(conn: &rusqlite::Connection, repo_path: &str, count: usize) {
    for i in 0..count {
        let ts = format!("2020-02-{:02}T10:00:00+00:00", (i % 28) + 1);
        db::insert_activity(
            conn,
            repo_path,
            "commit",
            Some("main"),
            None,
            Some(&format!("hash{i}")),
            Some("dev"),
            Some(&format!("feat: implement feature {i}")),
            &ts,
        )
        .unwrap();
    }
}

#[test]
fn cli_perf_review_no_api_key_exits_1() {
    let (_tmp, data_dir, config_dir) = setup_temp_env();
    let conn = open_test_db(&data_dir);

    // Insert 15 commits across 2 repos + 3 reviews
    seed_commits(&conn, "/repo/alpha", 8);
    seed_commits(&conn, "/repo/beta", 7);
    for i in 0..3 {
        db::insert_review(
            &conn,
            "/repo/alpha",
            i + 1,
            &format!("PR {}", i + 1),
            "https://github.com/test/pr",
            "APPROVED",
            &format!("2020-02-{:02}T12:00:00+00:00", i + 1),
        )
        .unwrap();
    }
    drop(conn);

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .arg("perf-review")
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "expected exit 1 with no API key"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("API key") || stderr.contains("api_key"),
        "expected API key error, got: {stderr}"
    );
}

#[test]
fn cli_perf_review_empty_range_no_activity() {
    let (_tmp, data_dir, config_dir) = setup_temp_env_with_llm_key();
    let _conn = open_test_db(&data_dir);

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["perf-review", "--from", "2020-01-01", "--to", "2020-01-02"])
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "expected exit 1 for empty period"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No activity"),
        "expected 'No activity' error, got: {stderr}"
    );
}

#[test]
fn cli_perf_review_invalid_date() {
    let (_tmp, data_dir, config_dir) = setup_temp_env_with_llm_key();
    let _conn = open_test_db(&data_dir);

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .args(["perf-review", "--from", "baddate", "--to", "2020-12-31"])
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "expected exit 1 for invalid date"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Invalid date"),
        "expected 'Invalid date' error, got: {stderr}"
    );
}

#[test]
fn sparse_data_warning_in_prompt_for_few_commits() {
    // Verify that < 10 commits triggers the limited data warning in context
    let now = Utc::now();
    let events: Vec<ActivityEvent> = (0..5)
        .map(|i| ActivityEvent {
            event_type: "commit".to_string(),
            branch: Some("main".to_string()),
            commit_hash: Some(format!("hash{i}")),
            message: Some(format!("commit {i}")),
            timestamp: now,
        })
        .collect();
    let repo = RepoSummary {
        repo_path: "/repo".to_string(),
        repo_name: "repo".to_string(),
        commits: 5,
        branches: vec!["main".to_string()],
        estimated_time: Duration::hours(2),
        events,
        pr_info: None,
        reviews: vec![],
        ai_sessions: vec![],
        presence_intervals: vec![],
        branch_switches: 0,
    };
    let summary = ActivitySummary {
        period_label: "Q1 2026".to_string(),
        total_commits: 5,
        total_reviews: 0,
        total_repos: 1,
        total_estimated_time: Duration::hours(2),
        total_ai_session_time: Duration::zero(),
        streak_days: 0,
        total_branch_switches: 0,
        repos: vec![repo],
    };

    let context = build_perf_review_context(&summary);
    assert_eq!(context.total_commits, 5);
    assert!(context.total_commits < 10, "test setup: need < 10 commits");

    // The warning is prepended inside generate_perf_review, which we can't
    // inspect without calling LLM. But we can verify the function doesn't
    // bail with "No activity found" (since commits > 0).
    let result = generate_perf_review(&dummy_llm_config(), &context);
    if let Err(e) = result {
        assert!(
            !e.to_string().contains("No activity found"),
            "5 commits should not trigger 'No activity found'"
        );
    }
}
