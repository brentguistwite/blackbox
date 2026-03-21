use blackbox::query::{ActivityEvent, ActivitySummary, AiSessionInfo, RepoSummary, ReviewInfo};
use blackbox::output::{format_duration, render_summary_to_string, render_json, render_csv, render_standup, OutputFormat};
use blackbox::enrichment::PrInfo;
use chrono::{Duration, Utc};

#[test]
fn format_duration_hours_and_minutes() {
    let d = Duration::minutes(90);
    assert_eq!(format_duration(d), "~1h 30m");
}

#[test]
fn format_duration_minutes_only() {
    let d = Duration::minutes(45);
    assert_eq!(format_duration(d), "~45m");
}

#[test]
fn format_duration_zero() {
    let d = Duration::zero();
    assert_eq!(format_duration(d), "~0m");
}

#[test]
fn format_duration_exact_hour() {
    let d = Duration::hours(2);
    assert_eq!(format_duration(d), "~2h 0m");
}

#[test]
fn render_summary_with_repos() {
    colored::control::set_override(false);

    let summary = ActivitySummary {
        period_label: "Today".to_string(),
        total_commits: 5,
        total_reviews: 0,
        total_repos: 1,
        total_estimated_time: Duration::minutes(90),
        total_ai_session_time: Duration::zero(),
        repos: vec![RepoSummary {
            repo_path: "/home/user/code/myproject".to_string(),
            repo_name: "myproject".to_string(),
            commits: 5,
            branches: vec!["main".to_string(), "feature-x".to_string()],
            estimated_time: Duration::minutes(90),
            events: vec![
                ActivityEvent {
                    event_type: "commit".to_string(),
                    branch: Some("main".to_string()),
                    commit_hash: Some("abc1234def5678".to_string()),
                    message: Some("fix login bug".to_string()),
                    timestamp: Utc::now(),
                },
                ActivityEvent {
                    event_type: "branch_switch".to_string(),
                    branch: Some("feature-x".to_string()),
                    commit_hash: None,
                    message: None,
                    timestamp: Utc::now(),
                },
                ActivityEvent {
                    event_type: "commit".to_string(),
                    branch: Some("feature-x".to_string()),
                    commit_hash: Some("def5678abc1234".to_string()),
                    message: Some("add new feature".to_string()),
                    timestamp: Utc::now(),
                },
            ],
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
        }],
    };

    let output = render_summary_to_string(&summary);
    assert!(output.contains("Today"), "should contain period label");
    assert!(output.contains("5 commits across 1 repo"), "should contain summary stats");
    assert!(output.contains("~1h 30m"), "should contain approximate time");
    assert!(output.contains("myproject"), "should contain repo name");
    assert!(output.contains("main"), "should contain branch");
    assert!(output.contains("feature-x"), "should contain branch");
    assert!(output.contains("abc1234"), "should contain short commit hash");
    assert!(output.contains("fix login bug"), "should contain commit message");
    assert!(output.contains("branch_switch"), "should show branch switch");
}

#[test]
fn render_empty_summary() {
    colored::control::set_override(false);

    let summary = ActivitySummary {
        period_label: "This Week".to_string(),
        total_commits: 0,
        total_reviews: 0,
        total_repos: 0,
        total_estimated_time: Duration::zero(),
        total_ai_session_time: Duration::zero(),
        repos: vec![],
    };

    let output = render_summary_to_string(&summary);
    assert!(output.contains("No activity recorded for This Week"), "should show no activity message");
}

fn make_test_summary() -> ActivitySummary {
    ActivitySummary {
        period_label: "Today".to_string(),
        total_commits: 3,
        total_reviews: 0,
        total_repos: 1,
        total_estimated_time: Duration::minutes(45),
        total_ai_session_time: Duration::zero(),
        repos: vec![RepoSummary {
            repo_path: "/home/user/code/myproject".to_string(),
            repo_name: "myproject".to_string(),
            commits: 3,
            branches: vec!["main".to_string()],
            estimated_time: Duration::minutes(45),
            events: vec![
                ActivityEvent {
                    event_type: "commit".to_string(),
                    branch: Some("main".to_string()),
                    commit_hash: Some("abc1234".to_string()),
                    message: Some("fix bug".to_string()),
                    timestamp: Utc::now(),
                },
                ActivityEvent {
                    event_type: "branch_switch".to_string(),
                    branch: Some("dev".to_string()),
                    commit_hash: None,
                    message: None,
                    timestamp: Utc::now(),
                },
            ],
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
        }],
    }
}

fn make_empty_summary() -> ActivitySummary {
    ActivitySummary {
        period_label: "This Week".to_string(),
        total_commits: 0,
        total_reviews: 0,
        total_repos: 0,
        total_estimated_time: Duration::zero(),
        total_ai_session_time: Duration::zero(),
        repos: vec![],
    }
}

// --- JSON tests ---

#[test]
fn render_json_is_valid_json() {
    let summary = make_test_summary();
    let json_str = render_json(&summary);
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("should be valid JSON");
    assert!(parsed.is_object());
}

#[test]
fn render_json_has_top_level_fields() {
    let summary = make_test_summary();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(v["period_label"], "Today");
    assert_eq!(v["total_commits"], 3);
    assert_eq!(v["total_repos"], 1);
    assert_eq!(v["total_estimated_minutes"], 45);
    assert!(v["repos"].is_array());
    assert_eq!(v["repos"].as_array().unwrap().len(), 1);
}

#[test]
fn render_json_repo_fields() {
    let summary = make_test_summary();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let repo = &v["repos"][0];
    assert_eq!(repo["repo_name"], "myproject");
    assert_eq!(repo["repo_path"], "/home/user/code/myproject");
    assert_eq!(repo["commits"], 3);
    assert!(repo["branches"].is_array());
    assert_eq!(repo["estimated_minutes"], 45);
    assert!(repo["events"].is_array());
    assert_eq!(repo["events"].as_array().unwrap().len(), 2);
}

#[test]
fn render_json_event_fields() {
    let summary = make_test_summary();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let event = &v["repos"][0]["events"][0];
    assert_eq!(event["event_type"], "commit");
    assert_eq!(event["branch"], "main");
    assert_eq!(event["commit_hash"], "abc1234");
    assert_eq!(event["message"], "fix bug");
    assert!(event["timestamp"].is_string());
}

#[test]
fn render_json_empty_summary() {
    let summary = make_empty_summary();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).expect("empty should be valid JSON");
    assert_eq!(v["repos"].as_array().unwrap().len(), 0);
    assert_eq!(v["total_commits"], 0);
}

// --- CSV tests ---

#[test]
fn render_csv_has_header() {
    let summary = make_test_summary();
    let csv_str = render_csv(&summary);
    let first_line = csv_str.lines().next().unwrap();
    assert_eq!(
        first_line,
        "period,repo_name,event_type,branch,commit_hash,message,timestamp,repo_estimated_minutes,pr_number,pr_title"
    );
}

#[test]
fn render_csv_row_count() {
    let summary = make_test_summary();
    let csv_str = render_csv(&summary);
    let lines: Vec<&str> = csv_str.lines().collect();
    // header + 2 events = 3 lines
    assert_eq!(lines.len(), 3);
}

#[test]
fn render_csv_empty_summary_header_only() {
    let summary = make_empty_summary();
    let csv_str = render_csv(&summary);
    let lines: Vec<&str> = csv_str.lines().collect();
    assert_eq!(lines.len(), 1, "empty summary should have header only");
    assert!(lines[0].contains("period"));
}

#[test]
fn output_format_enum_exists() {
    // Just verify the enum variants exist and Default works
    let _pretty = OutputFormat::Pretty;
    let _json = OutputFormat::Json;
    let _csv = OutputFormat::Csv;
    let default = OutputFormat::default();
    assert!(matches!(default, OutputFormat::Pretty));
}

// --- Review output tests ---

fn make_summary_with_reviews() -> ActivitySummary {
    ActivitySummary {
        period_label: "Today".to_string(),
        total_commits: 2,
        total_reviews: 3,
        total_repos: 1,
        total_estimated_time: Duration::minutes(30),
        total_ai_session_time: Duration::zero(),
        repos: vec![RepoSummary {
            repo_path: "/home/user/code/myproject".to_string(),
            repo_name: "myproject".to_string(),
            commits: 2,
            branches: vec!["main".to_string()],
            estimated_time: Duration::minutes(30),
            events: vec![ActivityEvent {
                event_type: "commit".to_string(),
                branch: Some("main".to_string()),
                commit_hash: Some("abc1234".to_string()),
                message: Some("fix bug".to_string()),
                timestamp: Utc::now(),
            }],
            pr_info: None,
            reviews: vec![
                ReviewInfo {
                    pr_number: 42,
                    pr_title: "Add auth".to_string(),
                    action: "APPROVED".to_string(),
                    reviewed_at: Utc::now(),
                },
                ReviewInfo {
                    pr_number: 43,
                    pr_title: "Fix typo".to_string(),
                    action: "COMMENTED".to_string(),
                    reviewed_at: Utc::now(),
                },
                ReviewInfo {
                    pr_number: 44,
                    pr_title: "Refactor DB".to_string(),
                    action: "CHANGES_REQUESTED".to_string(),
                    reviewed_at: Utc::now(),
                },
            ],
            ai_sessions: vec![],
        }],
    }
}

#[test]
fn render_pretty_shows_review_count_in_summary() {
    colored::control::set_override(false);
    let summary = make_summary_with_reviews();
    let output = render_summary_to_string(&summary);
    assert!(output.contains("3 reviews"), "should show total reviews in summary line");
}

#[test]
fn render_pretty_shows_reviewed_prs() {
    colored::control::set_override(false);
    let summary = make_summary_with_reviews();
    let output = render_summary_to_string(&summary);
    assert!(output.contains("Reviewed 3 PRs"), "should show reviewed N PRs");
    assert!(output.contains("PR #42: Add auth"), "should list PR titles");
    assert!(output.contains("PR #44: Refactor DB"), "should list all reviewed PRs");
}

#[test]
fn render_json_includes_reviews() {
    let summary = make_summary_with_reviews();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(v["total_reviews"], 3);
    let reviews = v["repos"][0]["reviews"].as_array().unwrap();
    assert_eq!(reviews.len(), 3);
    assert_eq!(reviews[0]["pr_number"], 42);
    assert_eq!(reviews[0]["action"], "APPROVED");
    assert_eq!(reviews[1]["pr_title"], "Fix typo");
}

#[test]
fn render_csv_includes_review_rows() {
    let summary = make_summary_with_reviews();
    let csv_str = render_csv(&summary);
    let lines: Vec<&str> = csv_str.lines().collect();
    // header + 1 commit event + 3 review rows = 5
    assert_eq!(lines.len(), 5);
    assert!(csv_str.contains("review_approved"), "should have review_approved event_type");
    assert!(csv_str.contains("review_commented"), "should have review_commented event_type");
    assert!(csv_str.contains("review_changes_requested"), "should have review_changes_requested");
}

#[test]
fn render_json_no_reviews_omits_field() {
    let summary = make_test_summary();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    // reviews field should be absent when empty (skip_serializing_if)
    assert!(v["repos"][0].get("reviews").is_none(), "empty reviews should be omitted from JSON");
}

// --- AI Session output tests ---

fn make_summary_with_ai_sessions() -> ActivitySummary {
    let started = Utc::now() - Duration::minutes(72);
    let ended = Utc::now() - Duration::minutes(10);
    ActivitySummary {
        period_label: "Today".to_string(),
        total_commits: 2,
        total_reviews: 0,
        total_repos: 1,
        total_estimated_time: Duration::minutes(30),
        total_ai_session_time: Duration::minutes(62) + Duration::minutes(0), // ended session only
        repos: vec![RepoSummary {
            repo_path: "/home/user/code/myproject".to_string(),
            repo_name: "myproject".to_string(),
            commits: 2,
            branches: vec!["main".to_string()],
            estimated_time: Duration::minutes(30),
            events: vec![ActivityEvent {
                event_type: "commit".to_string(),
                branch: Some("main".to_string()),
                commit_hash: Some("abc1234".to_string()),
                message: Some("fix bug".to_string()),
                timestamp: Utc::now(),
            }],
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![
                AiSessionInfo {
                    session_id: "sess-001".to_string(),
                    started_at: started,
                    ended_at: Some(ended),
                    duration: ended - started,
                    turns: Some(15),
                },
                AiSessionInfo {
                    session_id: "sess-002".to_string(),
                    started_at: Utc::now() - Duration::minutes(5),
                    ended_at: None,
                    duration: Duration::minutes(5),
                    turns: None,
                },
            ],
        }],
    }
}

#[test]
fn render_pretty_shows_ai_session_count() {
    colored::control::set_override(false);
    let summary = make_summary_with_ai_sessions();
    let output = render_summary_to_string(&summary);
    assert!(output.contains("2 Claude Code sessions"), "should show session count per repo");
}

#[test]
fn render_pretty_shows_ai_session_summary_line() {
    colored::control::set_override(false);
    let summary = make_summary_with_ai_sessions();
    let output = render_summary_to_string(&summary);
    assert!(output.contains("AI sessions:"), "should show AI session time in summary line");
}

#[test]
fn render_pretty_shows_active_session() {
    colored::control::set_override(false);
    let summary = make_summary_with_ai_sessions();
    let output = render_summary_to_string(&summary);
    assert!(output.contains("active"), "should show 'active' for ongoing sessions");
}

#[test]
fn render_pretty_shows_turns() {
    colored::control::set_override(false);
    let summary = make_summary_with_ai_sessions();
    let output = render_summary_to_string(&summary);
    assert!(output.contains("15 turns"), "should show turn count for ended sessions");
}

#[test]
fn render_json_includes_ai_sessions() {
    let summary = make_summary_with_ai_sessions();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert!(v["total_ai_session_minutes"].as_i64().unwrap() > 0);
    let sessions = v["repos"][0]["ai_sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0]["session_id"], "sess-001");
    assert!(sessions[0]["turns"].as_i64().is_some());
    assert!(sessions[1]["ended_at"].is_null());
}

#[test]
fn render_json_no_ai_sessions_omits_field() {
    let summary = make_test_summary();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert!(v["repos"][0].get("ai_sessions").is_none(), "empty ai_sessions should be omitted");
}

#[test]
fn render_csv_includes_ai_session_rows() {
    let summary = make_summary_with_ai_sessions();
    let csv_str = render_csv(&summary);
    let lines: Vec<&str> = csv_str.lines().collect();
    // header + 1 commit event + 2 ai_session rows = 4
    assert_eq!(lines.len(), 4);
    assert!(csv_str.contains("ai_session"), "should have ai_session event_type");
    assert!(csv_str.contains("Claude Code session"), "should have session description");
}

// --- Standup output tests ---

#[test]
fn standup_empty_shows_no_activity() {
    let summary = make_empty_summary();
    let output = render_standup(&summary);
    assert!(output.contains("No activity recorded"), "empty standup should say no activity");
}

#[test]
fn standup_has_markdown_bold_header() {
    let summary = make_test_summary();
    let output = render_standup(&summary);
    assert!(output.starts_with("**Today"), "header should start with markdown bold");
    assert!(output.contains(")**"), "header should close bold");
}

#[test]
fn standup_groups_commits_by_branch() {
    let summary = ActivitySummary {
        period_label: "Today".to_string(),
        total_commits: 5,
        total_reviews: 0,
        total_repos: 1,
        total_estimated_time: Duration::minutes(60),
        total_ai_session_time: Duration::zero(),
        repos: vec![RepoSummary {
            repo_path: "/code/myproject".to_string(),
            repo_name: "myproject".to_string(),
            commits: 5,
            branches: vec!["feat/auth".to_string(), "main".to_string()],
            estimated_time: Duration::minutes(60),
            events: vec![
                ActivityEvent { event_type: "commit".to_string(), branch: Some("feat/auth".to_string()), commit_hash: Some("a1".to_string()), message: Some("wip".to_string()), timestamp: Utc::now() },
                ActivityEvent { event_type: "commit".to_string(), branch: Some("feat/auth".to_string()), commit_hash: Some("a2".to_string()), message: Some("wip2".to_string()), timestamp: Utc::now() },
                ActivityEvent { event_type: "commit".to_string(), branch: Some("main".to_string()), commit_hash: Some("b1".to_string()), message: Some("merge".to_string()), timestamp: Utc::now() },
            ],
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
        }],
    };
    let output = render_standup(&summary);
    assert!(output.contains("feat/auth: 2 commits"), "should group feat/auth commits");
    assert!(output.contains("main: 1 commit"), "should show single commit for main");
}

#[test]
fn standup_shows_repo_with_bullet() {
    let summary = make_test_summary();
    let output = render_standup(&summary);
    assert!(output.contains("\u{2022} myproject"), "should use bullet for repo name");
}

#[test]
fn standup_shows_time_estimate() {
    let summary = make_test_summary();
    let output = render_standup(&summary);
    assert!(output.contains("~45m"), "should show time estimate on repo line");
}

#[test]
fn standup_shows_total_line() {
    let summary = make_test_summary();
    let output = render_standup(&summary);
    assert!(output.contains("Total:"), "should have total line");
    assert!(output.contains("1 repo"), "should show repo count");
}

#[test]
fn standup_no_ansi_codes() {
    let summary = make_test_summary();
    let output = render_standup(&summary);
    assert!(!output.contains("\x1b["), "standup output should have no ANSI escape codes");
}

#[test]
fn standup_includes_pr_info() {
    let summary = ActivitySummary {
        period_label: "Today".to_string(),
        total_commits: 1,
        total_reviews: 0,
        total_repos: 1,
        total_estimated_time: Duration::minutes(30),
        total_ai_session_time: Duration::zero(),
        repos: vec![RepoSummary {
            repo_path: "/code/proj".to_string(),
            repo_name: "proj".to_string(),
            commits: 1,
            branches: vec!["main".to_string()],
            estimated_time: Duration::minutes(30),
            events: vec![ActivityEvent { event_type: "commit".to_string(), branch: Some("main".to_string()), commit_hash: Some("x".to_string()), message: Some("m".to_string()), timestamp: Utc::now() }],
            pr_info: Some(vec![PrInfo { number: 472, title: "Auth refactor".to_string(), state: "MERGED".to_string(), head_ref_name: "feat/auth".to_string() }]),
            reviews: vec![],
            ai_sessions: vec![],
        }],
    };
    let output = render_standup(&summary);
    assert!(output.contains("Merged PR #472"), "should show merged PR");
}

#[test]
fn standup_includes_reviews() {
    let summary = make_summary_with_reviews();
    let output = render_standup(&summary);
    assert!(output.contains("Reviewed PR #42"), "should list reviewed PRs");
    assert!(output.contains("PR #43"), "should list all reviewed PRs");
}

#[test]
fn standup_includes_ai_sessions() {
    let summary = make_summary_with_ai_sessions();
    let output = render_standup(&summary);
    assert!(output.contains("Claude Code session"), "should show AI sessions");
}

#[test]
// --- US-007: Rhythms bar chart tests ---

#[test]
fn render_rhythms_empty_shows_no_activity() {
    colored::control::set_override(false);
    let hourly = [0usize; 24];
    let weekly = [0usize; 7];
    let output = blackbox::output::render_rhythms(&hourly, &weekly);
    assert!(output.contains("No activity"), "empty data should say no activity");
}

#[test]
fn render_rhythms_has_24_hour_labels() {
    colored::control::set_override(false);
    let mut hourly = [0usize; 24];
    hourly[10] = 5;
    let weekly = [0usize; 7];
    let output = blackbox::output::render_rhythms(&hourly, &weekly);
    assert!(output.contains("00"), "should have midnight label");
    assert!(output.contains("23"), "should have 23h label");
    assert!(output.contains("10"), "should have 10h label");
}

#[test]
fn render_rhythms_has_7_day_labels() {
    colored::control::set_override(false);
    let hourly = [0usize; 24];
    let mut weekly = [0usize; 7];
    weekly[0] = 3;
    let output = blackbox::output::render_rhythms(&hourly, &weekly);
    assert!(output.contains("Mon"), "should have Monday label");
    assert!(output.contains("Sun"), "should have Sunday label");
}

#[test]
fn render_rhythms_bars_scale_to_max() {
    colored::control::set_override(false);
    let mut hourly = [0usize; 24];
    hourly[9] = 10;  // max
    hourly[14] = 5;  // half
    let weekly = [0usize; 7];
    let output = blackbox::output::render_rhythms(&hourly, &weekly);
    // The max value row should have a longer bar than the half value row
    let lines: Vec<&str> = output.lines().collect();
    let line_9 = lines.iter().find(|l| l.contains("09")).unwrap();
    let line_14 = lines.iter().find(|l| l.contains("14")).unwrap();
    // Count bar chars (█)
    let bars_9: usize = line_9.chars().filter(|c| *c == '█').count();
    let bars_14: usize = line_14.chars().filter(|c| *c == '█').count();
    assert!(bars_9 > bars_14, "max value should have more bar chars: {} vs {}", bars_9, bars_14);
}

#[test]
fn standup_week_header() {
    let summary = ActivitySummary {
        period_label: "This Week".to_string(),
        total_commits: 1,
        total_reviews: 0,
        total_repos: 1,
        total_estimated_time: Duration::minutes(10),
        total_ai_session_time: Duration::zero(),
        repos: vec![RepoSummary {
            repo_path: "/code/p".to_string(),
            repo_name: "p".to_string(),
            commits: 1,
            branches: vec!["main".to_string()],
            estimated_time: Duration::minutes(10),
            events: vec![ActivityEvent { event_type: "commit".to_string(), branch: Some("main".to_string()), commit_hash: Some("z".to_string()), message: Some("x".to_string()), timestamp: Utc::now() }],
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
        }],
    };
    let output = render_standup(&summary);
    assert!(output.starts_with("**This Week"), "week standup should start with week header");
    assert!(output.contains(" - "), "week header should have date range with dash");
}
