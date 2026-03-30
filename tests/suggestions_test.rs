use blackbox::output::OutputFormat;
use blackbox::suggestions::{SuggestionCommand, SuggestionContext, generate_suggestions};

fn base_ctx() -> SuggestionContext {
    SuggestionContext {
        command: SuggestionCommand::Today,
        has_activity: true,
        summarize_used: false,
        daemon_running: true,
        llm_configured: false,
        format: OutputFormat::Pretty,
    }
}

#[test]
fn json_format_returns_empty() {
    let ctx = SuggestionContext {
        format: OutputFormat::Json,
        ..base_ctx()
    };
    assert!(generate_suggestions(&ctx).is_empty());
}

#[test]
fn csv_format_returns_empty() {
    let ctx = SuggestionContext {
        format: OutputFormat::Csv,
        ..base_ctx()
    };
    assert!(generate_suggestions(&ctx).is_empty());
}

#[test]
fn summarize_used_returns_empty() {
    let ctx = SuggestionContext {
        summarize_used: true,
        ..base_ctx()
    };
    assert!(generate_suggestions(&ctx).is_empty());
}

#[test]
fn standup_command_returns_empty() {
    let ctx = SuggestionContext {
        command: SuggestionCommand::Standup,
        ..base_ctx()
    };
    assert!(generate_suggestions(&ctx).is_empty());
}

#[test]
fn pretty_no_summarize_returns_vec() {
    // US-001: pure function returns vec (empty until US-002 adds rules)
    let ctx = base_ctx();
    let hints = generate_suggestions(&ctx);
    // Should be a vec (not panic). Content tested in US-002.
    assert!(hints.len() <= 3);
}

// --- US-002: Per-command ruleset ---

#[test]
fn today_daemon_not_running_includes_start() {
    let ctx = SuggestionContext {
        daemon_running: false,
        ..base_ctx()
    };
    let hints = generate_suggestions(&ctx);
    assert!(hints.iter().any(|h| h.contains("start")));
}

#[test]
fn daemon_not_running_hint_is_first() {
    let ctx = SuggestionContext {
        daemon_running: false,
        ..base_ctx()
    };
    let hints = generate_suggestions(&ctx);
    assert_eq!(hints[0], "blackbox start");
}

#[test]
fn today_daemon_running_has_activity_llm_configured_includes_summarize() {
    let ctx = SuggestionContext {
        llm_configured: true,
        ..base_ctx()
    };
    let hints = generate_suggestions(&ctx);
    assert!(hints.iter().any(|h| h == "blackbox today --summarize"));
}

#[test]
fn today_daemon_running_has_activity_includes_live() {
    let ctx = base_ctx();
    let hints = generate_suggestions(&ctx);
    assert!(hints.iter().any(|h| h == "blackbox live"));
}

#[test]
fn today_daemon_running_no_activity_includes_doctor() {
    let ctx = SuggestionContext {
        has_activity: false,
        ..base_ctx()
    };
    let hints = generate_suggestions(&ctx);
    assert!(hints.iter().any(|h| h.contains("doctor")));
}

#[test]
fn week_summarize_hint_uses_week_command() {
    let ctx = SuggestionContext {
        command: SuggestionCommand::Week,
        llm_configured: true,
        ..base_ctx()
    };
    let hints = generate_suggestions(&ctx);
    assert!(hints.iter().any(|h| h == "blackbox week --summarize"));
}

#[test]
fn month_summarize_hint_uses_month_command() {
    let ctx = SuggestionContext {
        command: SuggestionCommand::Month,
        llm_configured: true,
        ..base_ctx()
    };
    let hints = generate_suggestions(&ctx);
    assert!(hints.iter().any(|h| h == "blackbox month --summarize"));
}

#[test]
fn standup_always_empty() {
    // Daemon off, activity, LLM — still empty for standup
    let ctx = SuggestionContext {
        command: SuggestionCommand::Standup,
        daemon_running: false,
        llm_configured: true,
        ..base_ctx()
    };
    assert!(generate_suggestions(&ctx).is_empty());
}

#[test]
fn max_three_hints() {
    // daemon not running + has activity + llm configured = 3 candidates: start, summarize, live
    let ctx = SuggestionContext {
        daemon_running: false,
        llm_configured: true,
        ..base_ctx()
    };
    let hints = generate_suggestions(&ctx);
    assert!(hints.len() <= 3);
}

#[test]
fn no_doctor_when_has_activity() {
    let ctx = base_ctx(); // has_activity: true, daemon_running: true
    let hints = generate_suggestions(&ctx);
    assert!(!hints.iter().any(|h| h.contains("doctor")));
}

#[test]
fn no_summarize_hint_when_llm_not_configured() {
    let ctx = base_ctx(); // llm_configured: false
    let hints = generate_suggestions(&ctx);
    assert!(!hints.iter().any(|h| h.contains("--summarize")));
}

// --- US-004: period_label → SuggestionCommand mapping ---

#[test]
fn from_period_label_today() {
    assert_eq!(
        SuggestionCommand::from_period_label("Today"),
        Some(SuggestionCommand::Today)
    );
}

#[test]
fn from_period_label_this_week() {
    assert_eq!(
        SuggestionCommand::from_period_label("This Week"),
        Some(SuggestionCommand::Week)
    );
}

#[test]
fn from_period_label_this_month() {
    assert_eq!(
        SuggestionCommand::from_period_label("This Month"),
        Some(SuggestionCommand::Month)
    );
}

#[test]
fn from_period_label_unknown_returns_none() {
    assert_eq!(SuggestionCommand::from_period_label("Yesterday"), None);
}
