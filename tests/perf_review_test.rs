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
