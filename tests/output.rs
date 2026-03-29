use blackbox::output::{render_standup, render_summary_to_string, FOCUS_COST_PER_SWITCH_MINS};
use blackbox::query::{ActivityEvent, ActivitySummary, RepoSummary};
use chrono::{Duration, TimeZone, Utc};

fn make_event(event_type: &str, branch: Option<&str>, hash: Option<&str>, msg: Option<&str>, ts: chrono::DateTime<chrono::Utc>) -> ActivityEvent {
    ActivityEvent {
        event_type: event_type.to_string(),
        branch: branch.map(|s| s.to_string()),
        commit_hash: hash.map(|s| s.to_string()),
        message: msg.map(|s| s.to_string()),
        timestamp: ts,
    }
}

fn base_summary(repos: Vec<RepoSummary>) -> ActivitySummary {
    let total_commits: usize = repos.iter().map(|r| r.commits).sum();
    let total_switches: usize = repos.iter().map(|r| r.branch_switches).sum();
    ActivitySummary {
        period_label: "Today".to_string(),
        total_commits,
        total_reviews: 0,
        total_repos: repos.len(),
        total_estimated_time: Duration::minutes(120),
        total_ai_session_time: Duration::zero(),
        streak_days: 0,
        total_branch_switches: total_switches,
        repos,
    }
}

fn base_repo(name: &str, commits: usize, branch_switches: usize, events: Vec<ActivityEvent>) -> RepoSummary {
    RepoSummary {
        repo_path: format!("/tmp/{}", name),
        repo_name: name.to_string(),
        commits,
        branches: vec!["main".to_string()],
        estimated_time: Duration::minutes(60),
        events,
        pr_info: None,
        reviews: vec![],
        ai_sessions: vec![],
        presence_intervals: vec![],
        branch_switches,
    }
}

/// AC4: summary with 3 switches shows '3 branch switches (~69m focus cost)'
#[test]
fn summary_line_shows_branch_switches_with_focus_cost() {
    colored::control::set_override(false);
    let ts = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
    let events = vec![
        make_event("commit", Some("main"), Some("abc1234"), Some("init"), ts),
    ];
    let repo = base_repo("myrepo", 1, 3, events);
    let summary = base_summary(vec![repo]);

    let output = render_summary_to_string(&summary);
    assert!(output.contains("3 branch switches"), "expected branch switch count in summary line, got:\n{}", output);
    assert!(output.contains(&format!("~{}m focus cost", 3 * FOCUS_COST_PER_SWITCH_MINS)), "expected focus cost in summary line, got:\n{}", output);
}

/// AC1: summary line appends switch info when total_branch_switches > 0
#[test]
fn summary_line_includes_switch_suffix() {
    colored::control::set_override(false);
    let ts = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
    let events = vec![
        make_event("commit", Some("main"), Some("abc1234"), Some("init"), ts),
    ];
    let repo = base_repo("myrepo", 1, 5, events);
    let summary = base_summary(vec![repo]);

    let output = render_summary_to_string(&summary);
    // Should appear on the summary line with commits
    assert!(output.contains("5 branch switches (~115m focus cost)"), "got:\n{}", output);
}

/// AC3: nothing added when total_branch_switches == 0
#[test]
fn summary_line_no_switch_info_when_zero() {
    colored::control::set_override(false);
    let ts = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
    let events = vec![
        make_event("commit", Some("main"), Some("abc1234"), Some("init"), ts),
    ];
    let repo = base_repo("myrepo", 1, 0, events);
    let summary = base_summary(vec![repo]);

    let output = render_summary_to_string(&summary);
    assert!(!output.contains("branch switch"), "should not mention switches when 0, got:\n{}", output);
    assert!(!output.contains("focus cost"), "should not mention focus cost when 0, got:\n{}", output);
}

/// AC2: per-repo block shows switch count with breadcrumb
#[test]
fn per_repo_shows_switches_with_breadcrumb() {
    colored::control::set_override(false);
    let ts = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
    let events = vec![
        make_event("commit", Some("main"), Some("abc1234"), Some("init"), ts),
        make_event("branch_switch", Some("feature-a"), None, None, ts + Duration::minutes(10)),
        make_event("branch_switch", Some("main"), None, None, ts + Duration::minutes(20)),
        make_event("branch_switch", Some("feature-b"), None, None, ts + Duration::minutes(30)),
    ];
    let repo = base_repo("myrepo", 1, 3, events);
    let summary = base_summary(vec![repo]);

    let output = render_summary_to_string(&summary);
    assert!(output.contains("3 branch switches"), "expected per-repo switch count, got:\n{}", output);
    assert!(output.contains("feature-a->main->feature-b"), "expected breadcrumb trail, got:\n{}", output);
}

/// AC2: breadcrumb truncates with ... when more than 3 unique branches
#[test]
fn per_repo_breadcrumb_truncates_with_ellipsis() {
    colored::control::set_override(false);
    let ts = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
    let events = vec![
        make_event("commit", Some("main"), Some("abc1234"), Some("init"), ts),
        make_event("branch_switch", Some("a"), None, None, ts + Duration::minutes(10)),
        make_event("branch_switch", Some("b"), None, None, ts + Duration::minutes(20)),
        make_event("branch_switch", Some("c"), None, None, ts + Duration::minutes(30)),
        make_event("branch_switch", Some("d"), None, None, ts + Duration::minutes(40)),
    ];
    let repo = base_repo("myrepo", 1, 4, events);
    let summary = base_summary(vec![repo]);

    let output = render_summary_to_string(&summary);
    // Last 3 destinations: b, c, d — with ... prefix
    assert!(output.contains("...->b->c->d"), "expected truncated breadcrumb, got:\n{}", output);
}

/// Per-repo: no switch line when branch_switches == 0
#[test]
fn per_repo_no_switch_line_when_zero() {
    colored::control::set_override(false);
    let ts = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
    let events = vec![
        make_event("commit", Some("main"), Some("abc1234"), Some("init"), ts),
    ];
    let repo = base_repo("myrepo", 1, 0, events);
    let summary = base_summary(vec![repo]);

    let output = render_summary_to_string(&summary);
    assert!(!output.contains("branch switch"), "should not show switch line for repo with 0 switches, got:\n{}", output);
}

/// US-CS-08: standup includes context-switch line when total_branch_switches >= 5
#[test]
fn standup_includes_switch_line_when_above_threshold() {
    let ts = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
    let events = vec![
        make_event("commit", Some("main"), Some("abc1234"), Some("init"), ts),
    ];
    let repo = base_repo("myrepo", 1, 6, events);
    let summary = base_summary(vec![repo]);

    let output = render_standup(&summary);
    let focus_cost = 6 * FOCUS_COST_PER_SWITCH_MINS;
    assert!(
        output.contains(&format!("Context switches: 6 (est. ~{}m focus cost)", focus_cost)),
        "standup should include context-switch line for 6 switches, got:\n{}",
        output
    );
}

/// US-CS-08: standup omits context-switch line when total_branch_switches < 5
#[test]
fn standup_omits_switch_line_when_below_threshold() {
    let ts = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
    let events = vec![
        make_event("commit", Some("main"), Some("abc1234"), Some("init"), ts),
    ];
    let repo = base_repo("myrepo", 1, 4, events);
    let summary = base_summary(vec![repo]);

    let output = render_standup(&summary);
    assert!(
        !output.contains("Context switches"),
        "standup should NOT include context-switch line for 4 switches, got:\n{}",
        output
    );
}

/// Focus cost constant is 23 (Gloria Mark research)
#[test]
fn focus_cost_constant_is_23() {
    assert_eq!(FOCUS_COST_PER_SWITCH_MINS, 23);
}

/// US-CS-06 AC4: render_json includes total_branch_switches and per-repo branch_switches
#[test]
fn render_json_includes_branch_switch_counts() {
    use blackbox::output::render_json;

    let ts = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
    let events = vec![
        make_event("commit", Some("main"), Some("abc1234"), Some("init"), ts),
        make_event("branch_switch", Some("feature-a"), None, None, ts + Duration::minutes(10)),
        make_event("branch_switch", Some("main"), None, None, ts + Duration::minutes(20)),
    ];
    let repo = base_repo("myrepo", 1, 2, events);
    let summary = base_summary(vec![repo]);

    let json_str = render_json(&summary);
    let v: serde_json::Value = serde_json::from_str(&json_str).expect("valid JSON");

    assert_eq!(v["total_branch_switches"], 2, "top-level total_branch_switches");
    assert_eq!(v["repos"][0]["branch_switches"], 2, "per-repo branch_switches");
}
