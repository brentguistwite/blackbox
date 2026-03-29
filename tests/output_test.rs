use blackbox::churn::ChurnReport;
use blackbox::query::{ActivityEvent, ActivitySummary, AiSessionInfo, RepoSummary, ReviewInfo};
use blackbox::output::{format_duration, render_summary_to_string, render_json, render_csv, render_standup, render_focus_report, render_churn_pretty, render_churn_json, render_churn_csv, is_tty, resolve_format, OutputFormat};
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
        streak_days: 0,
        total_branch_switches: 0,
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
            presence_intervals: vec![],
            branch_switches: 0,
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
        streak_days: 0,
        total_branch_switches: 0,
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
        streak_days: 0,
        total_branch_switches: 0,
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
            presence_intervals: vec![],
            branch_switches: 0,
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
        streak_days: 0,
        total_branch_switches: 0,
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
        streak_days: 0,
        total_branch_switches: 0,
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
            presence_intervals: vec![],
            branch_switches: 0,
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
        streak_days: 0,
        total_branch_switches: 0,
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
                    tool: "claude-code".to_string(),
                    session_id: "sess-001".to_string(),
                    started_at: started,
                    ended_at: Some(ended),
                    duration: ended - started,
                    turns: Some(15),
                },
                AiSessionInfo {
                    tool: "claude-code".to_string(),
                    session_id: "sess-002".to_string(),
                    started_at: Utc::now() - Duration::minutes(5),
                    ended_at: None,
                    duration: Duration::minutes(5),
                    turns: None,
                },
            ],
            presence_intervals: vec![],
            branch_switches: 0,
        }],
    }
}

#[test]
fn render_pretty_shows_ai_session_count() {
    colored::control::set_override(false);
    let summary = make_summary_with_ai_sessions();
    let output = render_summary_to_string(&summary);
    assert!(output.contains("2 AI sessions"), "should show session count per repo");
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
    assert!(csv_str.contains("claude-code session"), "should have session description");
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
        streak_days: 0,
        total_branch_switches: 0,
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
            presence_intervals: vec![],
            branch_switches: 0,
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
        streak_days: 0,
        total_branch_switches: 0,
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
            presence_intervals: vec![],
            branch_switches: 0,
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
    assert!(output.contains("AI session"), "should show AI sessions");
}

// --- JSON streak tests (US-004) ---

#[test]
fn render_json_includes_streak_days() {
    let summary = make_today_summary_with_streak(3);
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(v["streak_days"], 3, "JSON should contain streak_days field with value 3");
}

#[test]
fn render_json_streak_zero_still_present() {
    let summary = make_today_summary_with_streak(0);
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(v["streak_days"], 0, "streak_days=0 should still be present in JSON (not omitted)");
}

// --- Streak display tests (US-003) ---

fn make_today_summary_with_streak(streak_days: u32) -> ActivitySummary {
    ActivitySummary {
        period_label: "Today".to_string(),
        total_commits: 3,
        total_reviews: 0,
        total_repos: 1,
        total_estimated_time: Duration::minutes(45),
        total_ai_session_time: Duration::zero(),
        streak_days,
        total_branch_switches: 0,
        repos: vec![RepoSummary {
            repo_path: "/code/proj".to_string(),
            repo_name: "proj".to_string(),
            commits: 3,
            branches: vec!["main".to_string()],
            estimated_time: Duration::minutes(45),
            events: vec![ActivityEvent {
                event_type: "commit".to_string(),
                branch: Some("main".to_string()),
                commit_hash: Some("abc1234".to_string()),
                message: Some("fix bug".to_string()),
                timestamp: Utc::now(),
            }],
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        }],
    }
}

#[test]
fn streak_shown_in_today_pretty_output() {
    colored::control::set_override(false);
    let summary = make_today_summary_with_streak(5);
    let output = render_summary_to_string(&summary);
    assert!(output.contains("5-day streak"), "today with streak=5 should show '5-day streak'");
}

#[test]
fn streak_zero_not_shown_in_pretty_output() {
    colored::control::set_override(false);
    let summary = make_today_summary_with_streak(0);
    let output = render_summary_to_string(&summary);
    assert!(!output.contains("streak"), "streak=0 should show no streak text");
}

#[test]
fn streak_not_shown_for_week_period() {
    colored::control::set_override(false);
    let mut summary = make_today_summary_with_streak(5);
    summary.period_label = "This Week".to_string();
    let output = render_summary_to_string(&summary);
    assert!(!output.contains("streak"), "week period should not show streak text");
}

#[test]
fn streak_one_day_singular_format() {
    colored::control::set_override(false);
    let summary = make_today_summary_with_streak(1);
    let output = render_summary_to_string(&summary);
    assert!(output.contains("1-day streak"), "streak=1 should show '1-day streak'");
}

#[test]
fn is_tty_returns_bool() {
    // Cannot assert specific TTY state in CI/test (stdout is a pipe),
    // but function must compile, be callable, and return a bool.
    let result: bool = is_tty();
    // In test harness, stdout is piped → expect false
    assert!(!result, "stdout should not be a TTY when running under cargo test");
}

// --- US-002 + US-003: resolve_format tests ---

#[test]
fn resolve_format_no_flags_tty_returns_pretty() {
    // TTY + no flags → Pretty (unchanged default behavior)
    assert!(matches!(resolve_format(OutputFormat::Pretty, false, false, true), OutputFormat::Pretty));
}

#[test]
fn resolve_format_no_flags_tty_preserves_explicit_format() {
    // TTY + explicit --format json/csv → preserved
    assert!(matches!(resolve_format(OutputFormat::Json, false, false, true), OutputFormat::Json));
    assert!(matches!(resolve_format(OutputFormat::Csv, false, false, true), OutputFormat::Csv));
}

#[test]
fn resolve_format_json_flag_wins() {
    // --json overrides any provided format, regardless of TTY
    assert!(matches!(resolve_format(OutputFormat::Pretty, true, false, true), OutputFormat::Json));
    assert!(matches!(resolve_format(OutputFormat::Csv, true, false, false), OutputFormat::Json));
    assert!(matches!(resolve_format(OutputFormat::Json, true, false, true), OutputFormat::Json));
}

#[test]
fn resolve_format_csv_flag_wins() {
    // --csv overrides any provided format (when --json is false), regardless of TTY
    assert!(matches!(resolve_format(OutputFormat::Pretty, false, true, true), OutputFormat::Csv));
    assert!(matches!(resolve_format(OutputFormat::Json, false, true, false), OutputFormat::Csv));
    assert!(matches!(resolve_format(OutputFormat::Csv, false, true, true), OutputFormat::Csv));
}

#[test]
fn resolve_format_json_takes_priority_over_csv() {
    // If both somehow true, json wins (clap prevents this, but function is defensive)
    assert!(matches!(resolve_format(OutputFormat::Pretty, true, true, true), OutputFormat::Json));
}

// --- US-003: TTY auto-detection ---

#[test]
fn resolve_format_non_tty_no_flags_returns_json() {
    // Non-TTY + no flags + default Pretty → auto-detect to Json
    assert!(matches!(resolve_format(OutputFormat::Pretty, false, false, false), OutputFormat::Json));
}

#[test]
fn resolve_format_non_tty_explicit_json_format_preserved() {
    // Non-TTY + --format json (no shorthand flags) → Json
    assert!(matches!(resolve_format(OutputFormat::Json, false, false, false), OutputFormat::Json));
}

#[test]
fn resolve_format_non_tty_explicit_csv_format_preserved() {
    // Non-TTY + --format csv (no shorthand flags) → Csv
    assert!(matches!(resolve_format(OutputFormat::Csv, false, false, false), OutputFormat::Csv));
}

#[test]
fn resolve_format_non_tty_json_flag() {
    // Non-TTY + --json → Json (flag takes priority)
    assert!(matches!(resolve_format(OutputFormat::Pretty, true, false, false), OutputFormat::Json));
}

#[test]
fn resolve_format_non_tty_csv_flag() {
    // Non-TTY + --csv → Csv (flag takes priority over auto-detect)
    assert!(matches!(resolve_format(OutputFormat::Pretty, false, true, false), OutputFormat::Csv));
}

#[test]
fn resolve_format_tty_csv_flag() {
    // TTY + --csv → Csv
    assert!(matches!(resolve_format(OutputFormat::Pretty, false, true, true), OutputFormat::Csv));
}

// --- Churn pretty output tests (US-010) ---

fn make_churn_report(repo_path: &str, written: u64, churned: u64, window: u32) -> ChurnReport {
    let rate = if written == 0 {
        0.0
    } else {
        churned as f64 / written as f64 * 100.0
    };
    ChurnReport {
        repo_path: repo_path.to_string(),
        window_days: window,
        total_lines_written: written,
        churned_lines: churned,
        churn_rate_pct: rate,
        commit_count: 5,
        churn_event_count: 2,
    }
}

#[test]
fn render_churn_pretty_empty_reports() {
    colored::control::set_override(false);
    let output = render_churn_pretty(&[]);
    assert!(output.contains("No churn data yet."), "empty reports should show no data message");
}

#[test]
fn render_churn_pretty_header_contains_window_days() {
    colored::control::set_override(false);
    let reports = vec![make_churn_report("/home/user/myproject", 100, 5, 14)];
    let output = render_churn_pretty(&reports);
    assert!(output.contains("=== Code Churn (last 14 days) ==="), "header should contain window days");
}

#[test]
fn render_churn_pretty_shows_repo_name() {
    colored::control::set_override(false);
    let reports = vec![make_churn_report("/home/user/myproject", 100, 5, 14)];
    let output = render_churn_pretty(&reports);
    assert!(output.contains("myproject"), "should show repo name extracted from path");
}

#[test]
fn render_churn_pretty_shows_correct_percentage() {
    colored::control::set_override(false);
    // 20 churned / 200 written = 10.0%
    let reports = vec![make_churn_report("/repos/alpha", 200, 20, 7)];
    let output = render_churn_pretty(&reports);
    assert!(output.contains("10.0%"), "should show correct churn percentage");
    assert!(output.contains("200"), "should show lines written");
    assert!(output.contains("20"), "should show lines churned");
}

#[test]
fn render_churn_pretty_global_summary() {
    colored::control::set_override(false);
    let reports = vec![
        make_churn_report("/repos/alpha", 200, 20, 14),
        make_churn_report("/repos/beta", 100, 30, 14),
    ];
    let output = render_churn_pretty(&reports);
    // Total: 300 written, 50 churned → 16.7%
    assert!(output.contains("Total:"), "should have total summary line");
    assert!(output.contains("300 lines written"), "total written should be aggregated");
    assert!(output.contains("50 churned"), "total churned should be aggregated");
    assert!(output.contains("16.7%"), "total percentage should be correct");
}

#[test]
fn render_churn_pretty_single_repo_no_churn() {
    colored::control::set_override(false);
    let reports = vec![make_churn_report("/repos/clean", 500, 0, 14)];
    let output = render_churn_pretty(&reports);
    assert!(output.contains("0.0%"), "zero churn should show 0.0%");
}

// --- Churn JSON output tests (US-011) ---

#[test]
fn render_churn_json_one_report_valid_json() {
    let reports = vec![make_churn_report("/home/user/myproject", 200, 20, 14)];
    let output = render_churn_json(&reports);
    let parsed: serde_json::Value = serde_json::from_str(&output).expect("should be valid JSON");
    let arr = parsed.as_array().expect("should be an array");
    assert_eq!(arr.len(), 1);

    let obj = &arr[0];
    assert_eq!(obj["repo_path"], "/home/user/myproject");
    assert_eq!(obj["repo_name"], "myproject");
    assert_eq!(obj["window_days"], 14);
    assert_eq!(obj["total_lines_written"], 200);
    assert_eq!(obj["churned_lines"], 20);
    assert!((obj["churn_rate_pct"].as_f64().unwrap() - 10.0).abs() < 0.01);
    assert_eq!(obj["commit_count"], 5);
    assert_eq!(obj["churn_event_count"], 2);
}

#[test]
fn render_churn_json_empty_reports() {
    let output = render_churn_json(&[]);
    let parsed: serde_json::Value = serde_json::from_str(&output).expect("should be valid JSON");
    let arr = parsed.as_array().expect("should be an array");
    assert!(arr.is_empty());
}

// --- Churn CSV output tests (US-011) ---

#[test]
fn render_churn_csv_one_report_header_and_values() {
    let reports = vec![make_churn_report("/repos/alpha", 200, 20, 7)];
    let output = render_churn_csv(&reports);
    let lines: Vec<&str> = output.lines().collect();
    assert!(lines.len() >= 2, "should have header + data row");
    assert_eq!(
        lines[0],
        "repo_path,repo_name,window_days,total_lines_written,churned_lines,churn_rate_pct,commit_count,churn_event_count"
    );
    // Data row
    assert!(lines[1].starts_with("/repos/alpha,alpha,7,200,20,"));
    assert!(lines[1].contains(",5,2"));
}

#[test]
fn render_churn_csv_empty_reports_has_header() {
    let output = render_churn_csv(&[]);
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 1, "empty should have header only");
    assert!(lines[0].contains("repo_path"), "header should be present");
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
        streak_days: 0,
        total_branch_switches: 0,
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
            presence_intervals: vec![],
            branch_switches: 0,
        }],
    };
    let output = render_standup(&summary);
    assert!(output.starts_with("**This Week"), "week standup should start with week header");
    assert!(output.contains(" - "), "week header should have date range with dash");
}

// --- Focus report tests ---

fn make_focus_summary(label: &str, switches: &[(&str, &str, usize)]) -> ActivitySummary {
    let total: usize = switches.iter().map(|(_, _, s)| s).sum();
    ActivitySummary {
        period_label: label.to_string(),
        total_commits: 0,
        total_reviews: 0,
        total_repos: switches.len(),
        total_estimated_time: Duration::zero(),
        total_ai_session_time: Duration::zero(),
        streak_days: 0,
        total_branch_switches: total,
        repos: switches.iter().map(|(path, name, bs)| RepoSummary {
            repo_path: path.to_string(),
            repo_name: name.to_string(),
            commits: 0,
            branches: vec![],
            estimated_time: Duration::zero(),
            events: vec![],
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: *bs,
        }).collect(),
    }
}

#[test]
fn focus_report_header_contains_period_label() {
    let summary = make_focus_summary("Today", &[("/code/a", "repo-a", 8)]);
    let output = render_focus_report(&summary);
    assert!(output.contains("=== Focus Report: Today ==="), "header should contain period label");
}

#[test]
fn focus_report_shows_total_switches_and_repos() {
    let summary = make_focus_summary("Today", &[
        ("/code/a", "repo-a", 8),
        ("/code/b", "repo-b", 4),
        ("/code/c", "repo-c", 2),
    ]);
    let output = render_focus_report(&summary);
    assert!(output.contains("14 branch switches across 3 repos"), "should show total switches and repo count");
}

#[test]
fn focus_report_shows_focus_cost() {
    let summary = make_focus_summary("Today", &[
        ("/code/a", "repo-a", 8),
        ("/code/b", "repo-b", 6),
    ]);
    let output = render_focus_report(&summary);
    // 14 * 23 = 322
    assert!(output.contains("~322m focus cost"), "should show focus cost");
}

#[test]
fn focus_report_per_repo_breakdown_sorted_desc() {
    let summary = make_focus_summary("Today", &[
        ("/code/a", "repo-a", 2),
        ("/code/b", "repo-b", 8),
        ("/code/c", "repo-c", 4),
    ]);
    let output = render_focus_report(&summary);
    let a_pos = output.find("repo-b: 8 switches").expect("repo-b line");
    let b_pos = output.find("repo-c: 4 switches").expect("repo-c line");
    let c_pos = output.find("repo-a: 2 switches").expect("repo-a line");
    assert!(a_pos < b_pos && b_pos < c_pos, "repos should be sorted by switches descending");
}

#[test]
fn focus_report_zero_switches_clean_day() {
    let summary = make_focus_summary("Today", &[]);
    let output = render_focus_report(&summary);
    assert!(output.contains("No branch switches recorded. Clean focus day."), "zero switches should show clean day message");
}

#[test]
fn focus_report_week_label() {
    let summary = make_focus_summary("This Week", &[("/code/a", "repo-a", 3)]);
    let output = render_focus_report(&summary);
    assert!(output.contains("=== Focus Report: This Week ==="), "should use week label");
}

#[test]
fn focus_report_singular_switch() {
    let summary = make_focus_summary("Today", &[("/code/a", "repo-a", 1)]);
    let output = render_focus_report(&summary);
    assert!(output.contains("1 branch switch across 1 repo"), "should use singular for 1 switch");
    assert!(output.contains("repo-a: 1 switch\n") || output.contains("repo-a: 1 switch"), "per-repo should use singular");
}

// --- US-009: tool field in output ---

fn make_summary_with_multi_tool_sessions() -> ActivitySummary {
    let started = Utc::now() - Duration::minutes(60);
    let ended = Utc::now();
    ActivitySummary {
        period_label: "Today".to_string(),
        total_commits: 1,
        total_reviews: 0,
        total_repos: 1,
        total_estimated_time: Duration::minutes(30),
        total_ai_session_time: Duration::minutes(120),
        streak_days: 0,
        total_branch_switches: 0,
        repos: vec![RepoSummary {
            repo_path: "/home/user/code/myproject".to_string(),
            repo_name: "myproject".to_string(),
            commits: 1,
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
                    tool: "claude-code".to_string(),
                    session_id: "sess-cc".to_string(),
                    started_at: started,
                    ended_at: Some(ended),
                    duration: ended - started,
                    turns: Some(10),
                },
                AiSessionInfo {
                    tool: "cursor".to_string(),
                    session_id: "sess-cur".to_string(),
                    started_at: started,
                    ended_at: None,
                    duration: Duration::minutes(30),
                    turns: None,
                },
            ],
            presence_intervals: vec![],
            branch_switches: 0,
        }],
    }
}

#[test]
fn json_output_includes_tool_field() {
    let summary = make_summary_with_multi_tool_sessions();
    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let sessions = v["repos"][0]["ai_sessions"].as_array().unwrap();
    assert_eq!(sessions[0]["tool"], "claude-code");
    assert_eq!(sessions[1]["tool"], "cursor");
}

#[test]
fn pretty_output_shows_tool_name() {
    colored::control::set_override(false);
    let summary = make_summary_with_multi_tool_sessions();
    let output = render_summary_to_string(&summary);
    assert!(output.contains("claude-code"), "pretty output should show claude-code tool name");
    assert!(output.contains("cursor"), "pretty output should show cursor tool name");
}

#[test]
fn csv_output_includes_tool_in_message() {
    let summary = make_summary_with_multi_tool_sessions();
    let csv_str = render_csv(&summary);
    assert!(csv_str.contains("claude-code"), "CSV should include claude-code tool name");
    assert!(csv_str.contains("cursor"), "CSV should include cursor tool name");
}
