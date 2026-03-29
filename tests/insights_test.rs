use assert_cmd::Command;
use blackbox::llm::{build_insights_prompt, INSIGHTS_SYSTEM_PROMPT};
use blackbox::enrichment::PrInfo;
use blackbox::query::{aggregate_insights_data, ActivityEvent, InsightsData, RepoInsights, RepoSummary};
use chrono::{Datelike, Duration, Utc};
use std::fs;
use tempfile::TempDir;

/// Helper: build a RepoSummary with commit events at given timestamps+messages.
fn repo(
    name: &str,
    events: Vec<(&str, Option<&str>)>, // (RFC3339 timestamp, optional message)
) -> RepoSummary {
    let activity_events: Vec<ActivityEvent> = events
        .iter()
        .map(|(ts, msg)| ActivityEvent {
            event_type: "commit".to_string(),
            branch: Some("main".to_string()),
            commit_hash: Some("abc123".to_string()),
            message: msg.map(|m| m.to_string()),
            timestamp: chrono::DateTime::parse_from_rfc3339(ts)
                .unwrap()
                .with_timezone(&Utc),
        })
        .collect();
    let commits = activity_events.len();
    RepoSummary {
        repo_path: format!("/repo/{}", name),
        repo_name: name.to_string(),
        commits,
        branches: vec!["main".to_string()],
        estimated_time: Duration::minutes(30 * commits as i64),
        events: activity_events,
        pr_info: None,
        reviews: vec![],
        ai_sessions: vec![],
        presence_intervals: vec![],
        branch_switches: 0,
    }
}

#[test]
fn bugfix_classification() {
    let repos = vec![repo(
        "alpha",
        vec![
            ("2025-01-13T10:00:00Z", Some("fix: login bug")),
            ("2025-01-13T11:00:00Z", Some("feat: add button")),
            ("2025-01-13T12:00:00Z", Some("bug: crash on startup")),
        ],
    )];
    let data = aggregate_insights_data(&repos, "This Week");
    assert_eq!(data.bugfix_commits, 2);
    assert_eq!(data.total_commits_with_msg, 3);
    assert_eq!(data.total_commits, 3);
}

#[test]
fn commits_by_dow_bucketing() {
    // 2025-01-13 = Monday, 2025-01-15 = Wednesday
    let repos = vec![repo(
        "alpha",
        vec![
            ("2025-01-13T10:00:00Z", Some("mon commit 1")),
            ("2025-01-13T11:00:00Z", Some("mon commit 2")),
            ("2025-01-15T10:00:00Z", Some("wed commit")),
        ],
    )];
    let data = aggregate_insights_data(&repos, "This Week");

    // Compute expected DOW indices at runtime (handles timezone differences)
    let mon_local = chrono::DateTime::parse_from_rfc3339("2025-01-13T10:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Local);
    let wed_local = chrono::DateTime::parse_from_rfc3339("2025-01-15T10:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Local);
    let mon_idx = mon_local.weekday().num_days_from_monday() as usize;
    let wed_idx = wed_local.weekday().num_days_from_monday() as usize;

    assert_eq!(data.commits_by_dow[mon_idx], 2);
    assert_eq!(data.commits_by_dow[wed_idx], 1);
    let total_dow: u32 = data.commits_by_dow.iter().sum();
    assert_eq!(total_dow, 3);
    assert_eq!(data.total_repos, 1);
}

#[test]
fn commits_by_hour_bucketing() {
    let repos = vec![repo(
        "alpha",
        vec![
            ("2025-01-13T10:00:00Z", Some("morning")),
            ("2025-01-13T10:30:00Z", Some("morning 2")),
            ("2025-01-13T15:00:00Z", Some("afternoon")),
        ],
    )];
    let data = aggregate_insights_data(&repos, "This Week");
    let total_hour: u32 = data.commits_by_hour.iter().sum();
    assert_eq!(total_hour, 3);
}

#[test]
fn empty_repos_returns_zeroed_struct() {
    let repos: Vec<RepoSummary> = vec![];
    let data = aggregate_insights_data(&repos, "This Week");
    assert_eq!(data.total_commits, 0);
    assert_eq!(data.total_repos, 0);
    assert_eq!(data.commits_by_dow, [0u32; 7]);
    assert_eq!(data.commits_by_hour, [0u32; 24]);
    assert_eq!(data.avg_msg_len_by_dow, [0.0f64; 7]);
    assert_eq!(data.bugfix_commits, 0);
    assert_eq!(data.total_commits_with_msg, 0);
    assert!(data.pr_merge_times_hours.is_empty());
    assert!(data.per_repo.is_empty());
}

#[test]
fn avg_msg_len_by_dow() {
    // 2025-01-13 = Monday, 2025-01-17 = Friday
    let mon_msg = "twenty char message!"; // exactly 20 chars
    let fri_msg = "short!!!"; // exactly 8 chars
    assert_eq!(mon_msg.len(), 20);
    assert_eq!(fri_msg.len(), 8);

    let repos = vec![repo(
        "alpha",
        vec![
            ("2025-01-13T10:00:00Z", Some(mon_msg)),
            ("2025-01-17T10:00:00Z", Some(fri_msg)),
        ],
    )];
    let data = aggregate_insights_data(&repos, "This Week");

    // Compute expected DOW indices at runtime (handles timezone differences)
    let mon_local = chrono::DateTime::parse_from_rfc3339("2025-01-13T10:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Local);
    let fri_local = chrono::DateTime::parse_from_rfc3339("2025-01-17T10:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Local);
    let mon_idx = mon_local.weekday().num_days_from_monday() as usize;
    let fri_idx = fri_local.weekday().num_days_from_monday() as usize;

    assert!(
        (data.avg_msg_len_by_dow[mon_idx] - 20.0).abs() < 0.01,
        "Mon avg_msg_len expected ~20, got {}",
        data.avg_msg_len_by_dow[mon_idx]
    );
    assert!(
        (data.avg_msg_len_by_dow[fri_idx] - 8.0).abs() < 0.01,
        "Fri avg_msg_len expected ~8, got {}",
        data.avg_msg_len_by_dow[fri_idx]
    );
}

#[test]
fn per_repo_stats() {
    let repos = vec![
        repo(
            "alpha",
            vec![
                ("2025-01-13T10:00:00Z", Some("commit one")),
                ("2025-01-13T11:00:00Z", Some("commit two")),
            ],
        ),
        repo(
            "beta",
            vec![("2025-01-14T10:00:00Z", Some("only commit"))],
        ),
    ];
    let data = aggregate_insights_data(&repos, "This Week");
    assert_eq!(data.per_repo.len(), 2);
    let alpha = data.per_repo.iter().find(|r| r.repo_name == "alpha").unwrap();
    assert_eq!(alpha.commits, 2);
    assert_eq!(alpha.branches_touched, 1);
    assert!(!alpha.has_prs);
    assert!(alpha.avg_commit_msg_len > 0.0);
}

#[test]
fn no_message_commits_skipped_for_msg_stats() {
    let repos = vec![repo(
        "alpha",
        vec![
            ("2025-01-13T10:00:00Z", None),
            ("2025-01-13T11:00:00Z", Some("has a message")),
        ],
    )];
    let data = aggregate_insights_data(&repos, "This Week");
    assert_eq!(data.total_commits, 2);
    assert_eq!(data.total_commits_with_msg, 1);
    // bugfix should be 0 (None skipped, "has a message" not a bugfix)
    assert_eq!(data.bugfix_commits, 0);
}

#[test]
fn period_label_passthrough() {
    let repos: Vec<RepoSummary> = vec![];
    let data = aggregate_insights_data(&repos, "This Month");
    assert_eq!(data.period_label, "This Month");
}

// --- US-002: build_insights_prompt tests ---

fn make_insights_data(num_repos: usize) -> InsightsData {
    let mut commits_by_dow = [0u32; 7];
    commits_by_dow[0] = 8;  // Mon
    commits_by_dow[1] = 12; // Tue
    commits_by_dow[2] = 8;  // Wed
    commits_by_dow[3] = 10; // Thu
    commits_by_dow[4] = 9;  // Fri

    let mut commits_by_hour = [0u32; 24];
    commits_by_hour[10] = 9;
    commits_by_hour[14] = 7;
    commits_by_hour[15] = 6;
    commits_by_hour[9] = 3;

    let mut avg_msg_len_by_dow = [0.0f64; 7];
    avg_msg_len_by_dow[0] = 42.0;
    avg_msg_len_by_dow[1] = 38.0;
    avg_msg_len_by_dow[2] = 35.0;
    avg_msg_len_by_dow[3] = 31.0;
    avg_msg_len_by_dow[4] = 28.0;

    let per_repo: Vec<RepoInsights> = (0..num_repos)
        .map(|i| RepoInsights {
            repo_name: format!("repo-{}", i),
            commits: 20 - i, // descending
            estimated_minutes: 120 - (i as i64 * 5),
            branches_touched: 2,
            has_prs: i % 3 == 0,
            avg_commit_msg_len: 35.0,
        })
        .collect();

    let total_commits: usize = per_repo.iter().map(|r| r.commits).sum();

    InsightsData {
        period_label: "This Week".to_string(),
        total_commits,
        total_repos: num_repos,
        commits_by_dow,
        commits_by_hour,
        avg_msg_len_by_dow,
        bugfix_commits: 6,
        total_commits_with_msg: 45,
        pr_merge_times_hours: vec![2.5, 4.0, 8.0],
        per_repo,
    }
}

#[test]
fn prompt_contains_framing_line() {
    let data = make_insights_data(3);
    let prompt = build_insights_prompt(&data);
    assert!(prompt.contains("Analyze these developer activity patterns"));
}

#[test]
fn prompt_contains_period_and_totals() {
    let data = make_insights_data(3);
    let prompt = build_insights_prompt(&data);
    assert!(prompt.contains("This Week"));
    assert!(prompt.contains("across 3 repos"));
}

#[test]
fn prompt_contains_dow_distribution() {
    let data = make_insights_data(3);
    let prompt = build_insights_prompt(&data);
    // Should have day labels with percentages
    assert!(prompt.contains("Mon:"));
    assert!(prompt.contains("Tue:"));
    assert!(prompt.contains("%"));
}

#[test]
fn prompt_contains_peak_hour() {
    let data = make_insights_data(3);
    let prompt = build_insights_prompt(&data);
    assert!(prompt.contains("10:00"));
}

#[test]
fn prompt_contains_bugfix_ratio() {
    let data = make_insights_data(3);
    let prompt = build_insights_prompt(&data);
    assert!(prompt.contains("Bug-fix"));
    assert!(prompt.contains("6/"));
}

#[test]
fn prompt_contains_repo_breakdown() {
    let data = make_insights_data(3);
    let prompt = build_insights_prompt(&data);
    assert!(prompt.contains("repo-0"));
    assert!(prompt.contains("repo-1"));
    assert!(prompt.contains("repo-2"));
}

#[test]
fn prompt_contains_pr_merge_times() {
    let data = make_insights_data(3);
    let prompt = build_insights_prompt(&data);
    assert!(prompt.contains("PR merge times"));
    assert!(prompt.contains("3 PRs"));
}

#[test]
fn prompt_omits_pr_section_when_empty() {
    let mut data = make_insights_data(3);
    data.pr_merge_times_hours = vec![];
    let prompt = build_insights_prompt(&data);
    assert!(!prompt.contains("PR merge times"));
}

#[test]
fn prompt_no_commit_hashes() {
    let data = make_insights_data(5);
    let prompt = build_insights_prompt(&data);
    assert!(!prompt.contains("abc123"));
}

#[test]
fn prompt_truncates_repos_over_10() {
    let data = make_insights_data(15);
    let prompt = build_insights_prompt(&data);
    assert!(prompt.contains("showing top 10 of 15 repos"));
    // Should not contain repo-10 through repo-14
    assert!(!prompt.contains("repo-14"));
}

#[test]
fn prompt_is_plain_text_not_json() {
    let data = make_insights_data(3);
    let prompt = build_insights_prompt(&data);
    // Should not be parseable as JSON
    assert!(serde_json::from_str::<serde_json::Value>(&prompt).is_err());
}

#[test]
fn prompt_under_8000_chars_for_large_dataset() {
    let data = make_insights_data(15);
    let prompt = build_insights_prompt(&data);
    assert!(prompt.len() < 8000, "prompt len {} >= 8000", prompt.len());
}

#[test]
fn prompt_trims_repos_further_when_over_8000_chars() {
    // Create 10 repos with very long names to push prompt past 8000 chars
    let per_repo: Vec<RepoInsights> = (0..10)
        .map(|i| RepoInsights {
            repo_name: format!("repo-{}-{}", i, "x".repeat(750)),
            commits: 50 - i,
            estimated_minutes: 600,
            branches_touched: 5,
            has_prs: true,
            avg_commit_msg_len: 45.0,
        })
        .collect();
    let total_commits: usize = per_repo.iter().map(|r| r.commits).sum();
    let data = InsightsData {
        period_label: "This Week".to_string(),
        total_commits,
        total_repos: 10,
        commits_by_dow: [10, 12, 8, 10, 9, 0, 0],
        commits_by_hour: [0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 9, 3, 2, 1, 7, 6, 0, 0, 0, 0, 0, 0, 0, 0],
        avg_msg_len_by_dow: [42.0, 38.0, 35.0, 31.0, 28.0, 0.0, 0.0],
        bugfix_commits: 4,
        total_commits_with_msg: 40,
        pr_merge_times_hours: vec![2.0, 5.0],
        per_repo,
    };
    let prompt = build_insights_prompt(&data);
    assert!(prompt.len() < 8000, "prompt len {} >= 8000 after trimming", prompt.len());
    // Should have fewer than 10 repos shown due to trimming
    assert!(prompt.contains("showing top"), "should have truncation note");
}

#[test]
fn prompt_has_prs_marker_on_repos() {
    let data = make_insights_data(3);
    let prompt = build_insights_prompt(&data);
    // repo-0 has has_prs=true (i%3==0)
    assert!(prompt.contains("[has PRs]"));
}

// --- US-006: InsightsData JSON serialization ---

#[test]
fn render_insights_json_valid_pretty_json() {
    let data = make_insights_data(3);
    let json_str = blackbox::output::render_insights_json(&data);

    // Must be valid JSON
    let val: serde_json::Value = serde_json::from_str(&json_str).expect("should be valid JSON");

    // Pretty-printed = contains newlines
    assert!(json_str.contains('\n'), "should be pretty-printed");

    // Snake_case keys
    let obj = val.as_object().unwrap();
    assert!(obj.contains_key("period_label"));
    assert!(obj.contains_key("total_commits"));
    assert!(obj.contains_key("commits_by_dow"));
    assert!(obj.contains_key("commits_by_hour"));
    assert!(obj.contains_key("avg_msg_len_by_dow"));
    assert!(obj.contains_key("bugfix_commits"));
    assert!(obj.contains_key("pr_merge_times_hours"));
    assert!(obj.contains_key("per_repo"));
}

#[test]
fn render_insights_json_array_lengths() {
    let data = make_insights_data(3);
    let json_str = blackbox::output::render_insights_json(&data);
    let val: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // commits_by_dow: 7 elements
    let dow = val["commits_by_dow"].as_array().unwrap();
    assert_eq!(dow.len(), 7);

    // commits_by_hour: 24 elements
    let hour = val["commits_by_hour"].as_array().unwrap();
    assert_eq!(hour.len(), 24);

    // pr_merge_times_hours: 3 f64s from test data
    let prs = val["pr_merge_times_hours"].as_array().unwrap();
    assert_eq!(prs.len(), 3);
    assert_eq!(prs[0].as_f64().unwrap(), 2.5);
    assert_eq!(prs[1].as_f64().unwrap(), 4.0);
    assert_eq!(prs[2].as_f64().unwrap(), 8.0);
}

#[test]
fn render_insights_json_per_repo_fields() {
    let data = make_insights_data(2);
    let json_str = blackbox::output::render_insights_json(&data);
    let val: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    let repos = val["per_repo"].as_array().unwrap();
    assert_eq!(repos.len(), 2);

    let r0 = &repos[0];
    assert!(r0["repo_name"].is_string());
    assert!(r0["commits"].is_number());
    assert!(r0["estimated_minutes"].is_number());
    assert!(r0["branches_touched"].is_number());
    assert!(r0["has_prs"].is_boolean());
    assert!(r0["avg_commit_msg_len"].is_number());
}

#[test]
fn render_insights_json_empty_data() {
    let data = InsightsData {
        period_label: "This Week".to_string(),
        total_commits: 0,
        total_repos: 0,
        commits_by_dow: [0u32; 7],
        commits_by_hour: [0u32; 24],
        avg_msg_len_by_dow: [0.0f64; 7],
        bugfix_commits: 0,
        total_commits_with_msg: 0,
        pr_merge_times_hours: vec![],
        per_repo: vec![],
    };
    let json_str = blackbox::output::render_insights_json(&data);
    let val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(val["total_commits"], 0);
    assert!(val["per_repo"].as_array().unwrap().is_empty());
    assert!(val["pr_merge_times_hours"].as_array().unwrap().is_empty());
}

// --- US-016: CLI integration test: blackbox insights --format json ---

/// Helper: init config + open DB + insert sample commits for CLI tests.
fn setup_insights_cli_env() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    // Init config (no llm_api_key — json path shouldn't need it)
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
        .assert()
        .success();

    // Populate DB with commits
    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let conn = blackbox::db::open_db(&db_dir.join("blackbox.db")).unwrap();
    let now = chrono::Utc::now();
    for i in 0..5 {
        let ts = now - chrono::Duration::hours(i);
        conn.execute(
            "INSERT INTO git_activity (repo_path, event_type, branch, commit_hash, author, message, timestamp)
             VALUES (?1, 'commit', 'main', ?2, 'tester', 'test commit', ?3)",
            rusqlite::params!["/tmp/repos/foo", format!("abc{i}"), ts.to_rfc3339()],
        )
        .unwrap();
    }

    (tmp, config_dir, data_dir)
}

#[test]
fn cli_insights_json_exits_zero() {
    let (_tmp, config_dir, data_dir) = setup_insights_cli_env();

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["insights", "--format", "json"])
        .assert()
        .success();
}

#[test]
fn cli_insights_json_is_valid_json_with_total_commits() {
    let (_tmp, config_dir, data_dir) = setup_insights_cli_env();

    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["insights", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let val: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");
    assert!(
        val.get("total_commits").is_some(),
        "JSON must contain total_commits field"
    );
    assert!(
        val["total_commits"].as_u64().unwrap() >= 5,
        "should have at least the 5 inserted commits"
    );
}

#[test]
fn cli_insights_json_no_llm_key_required() {
    // Verify --format json works without any llm_api_key configured
    let (_tmp, config_dir, data_dir) = setup_insights_cli_env();

    // Confirm config has no llm_api_key
    let config_path = config_dir.join("blackbox/config.toml");
    let content = fs::read_to_string(&config_path).unwrap();
    assert!(
        !content.contains("llm_api_key"),
        "test config should not have llm_api_key"
    );

    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["insights", "--format", "json"])
        .assert()
        .success();
}

// --- US-017: empty activity short-circuit ---

/// Helper: init config + open DB but insert NO commits.
fn setup_empty_insights_cli_env() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    // Init config (no llm_api_key)
    Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["init", "--watch-dirs", "/tmp/repos", "--poll-interval", "300"])
        .assert()
        .success();

    // Ensure DB exists but is empty
    let db_dir = data_dir.join("blackbox");
    fs::create_dir_all(&db_dir).unwrap();
    let _conn = blackbox::db::open_db(&db_dir.join("blackbox.db")).unwrap();

    (tmp, config_dir, data_dir)
}

#[test]
fn cli_insights_empty_db_exits_zero_with_no_activity_msg() {
    let (_tmp, config_dir, data_dir) = setup_empty_insights_cli_env();

    // No API key, no commits — should short-circuit before LLM check
    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["insights"])
        .output()
        .unwrap();

    assert!(output.status.success(), "should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No activity"),
        "stdout should contain 'No activity', got: {stdout}"
    );
    // Must NOT be an API key error
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains("llm_api_key") && !stderr.contains("llm_api_key"),
        "should not hit API key error path"
    );
}

// --- US-018: missing API key error path ---

#[test]
fn cli_insights_pretty_missing_api_key_exits_nonzero() {
    let (_tmp, config_dir, data_dir) = setup_insights_cli_env();

    // Pretty mode (default) with populated DB but no llm_api_key → non-zero exit
    let output = Command::cargo_bin("blackbox")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .args(["insights"])
        .output()
        .unwrap();

    assert!(!output.status.success(), "should exit non-zero without API key");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("llm_api_key"),
        "stderr should mention 'llm_api_key', got: {stderr}"
    );
    // Error should include config snippet example
    assert!(
        stderr.contains("config.toml"),
        "stderr should include config.toml example, got: {stderr}"
    );
}

// --- US-015: INSIGHTS_SYSTEM_PROMPT content assertions ---

#[test]
fn system_prompt_instructs_quantitative_claims() {
    assert!(
        INSIGHTS_SYSTEM_PROMPT.contains("quantitative") || INSIGHTS_SYSTEM_PROMPT.contains("specific"),
        "INSIGHTS_SYSTEM_PROMPT should instruct numeric/quantitative claims"
    );
}

#[test]
fn system_prompt_prohibits_filler_language() {
    assert!(!INSIGHTS_SYSTEM_PROMPT.contains("Great job"), "must not contain 'Great job'");
    assert!(!INSIGHTS_SYSTEM_PROMPT.contains("Consider"), "must not contain 'Consider'");
}

#[test]
fn system_prompt_is_real_not_placeholder() {
    assert!(
        INSIGHTS_SYSTEM_PROMPT.len() > 100,
        "INSIGHTS_SYSTEM_PROMPT too short ({}), likely placeholder",
        INSIGHTS_SYSTEM_PROMPT.len()
    );
}

// --- US-011: PR merge time enrichment ---

#[test]
fn pr_merge_times_computed_from_merged_prs() {
    let mut r = repo("alpha", vec![
        ("2025-01-13T10:00:00Z", Some("commit")),
    ]);
    // Attach PR info with one MERGED PR (created->merged = 24h)
    r.pr_info = Some(vec![
        PrInfo {
            number: 1,
            title: "feat: something".to_string(),
            state: "MERGED".to_string(),
            head_ref_name: "main".to_string(),
            created_at: Some("2025-01-13T10:00:00Z".to_string()),
            merged_at: Some("2025-01-14T10:00:00Z".to_string()),
        },
        // OPEN PR — should be excluded
        PrInfo {
            number: 2,
            title: "wip".to_string(),
            state: "OPEN".to_string(),
            head_ref_name: "main".to_string(),
            created_at: Some("2025-01-13T10:00:00Z".to_string()),
            merged_at: None,
        },
        // MERGED but missing created_at — should be excluded
        PrInfo {
            number: 3,
            title: "old".to_string(),
            state: "MERGED".to_string(),
            head_ref_name: "main".to_string(),
            created_at: None,
            merged_at: Some("2025-01-14T10:00:00Z".to_string()),
        },
    ]);

    let data = aggregate_insights_data(&[r], "This Week");
    assert_eq!(data.pr_merge_times_hours.len(), 1);
    assert!((data.pr_merge_times_hours[0] - 24.0).abs() < 0.01);
}

#[test]
fn pr_merge_times_empty_when_no_pr_info() {
    let r = repo("alpha", vec![
        ("2025-01-13T10:00:00Z", Some("commit")),
    ]);
    let data = aggregate_insights_data(&[r], "This Week");
    assert!(data.pr_merge_times_hours.is_empty());
}

#[test]
fn pr_merge_times_empty_when_no_merged_prs() {
    let mut r = repo("alpha", vec![
        ("2025-01-13T10:00:00Z", Some("commit")),
    ]);
    r.pr_info = Some(vec![
        PrInfo {
            number: 1,
            title: "open pr".to_string(),
            state: "OPEN".to_string(),
            head_ref_name: "main".to_string(),
            created_at: Some("2025-01-13T10:00:00Z".to_string()),
            merged_at: None,
        },
    ]);
    let data = aggregate_insights_data(&[r], "This Week");
    assert!(data.pr_merge_times_hours.is_empty());
}
