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
