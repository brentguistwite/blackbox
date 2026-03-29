use crate::output::OutputFormat;

#[derive(Debug, Clone, PartialEq)]
pub enum SuggestionCommand {
    Today,
    Week,
    Month,
    Standup,
}

#[derive(Debug, Clone)]
pub struct SuggestionContext {
    pub command: SuggestionCommand,
    pub has_activity: bool,
    pub summarize_used: bool,
    pub daemon_running: bool,
    pub llm_configured: bool,
    pub format: OutputFormat,
}

/// Returns zero or more hint strings based on context.
/// Pure function — no I/O, no side effects.
pub fn generate_suggestions(ctx: &SuggestionContext) -> Vec<String> {
    if !matches!(ctx.format, OutputFormat::Pretty) || ctx.summarize_used {
        return vec![];
    }
    if ctx.command == SuggestionCommand::Standup {
        return vec![];
    }
    // Ruleset added in US-002
    vec![]
}
