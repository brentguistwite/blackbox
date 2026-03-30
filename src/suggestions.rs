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

impl SuggestionCommand {
    /// Map run_query's period_label string to a SuggestionCommand.
    pub fn from_period_label(label: &str) -> Option<Self> {
        match label {
            "Today" => Some(Self::Today),
            "This Week" => Some(Self::Week),
            "This Month" => Some(Self::Month),
            _ => None,
        }
    }
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

    let cmd_name = match ctx.command {
        SuggestionCommand::Today => "today",
        SuggestionCommand::Week => "week",
        SuggestionCommand::Month => "month",
        SuggestionCommand::Standup => unreachable!(),
    };

    let mut hints: Vec<String> = Vec::new();

    if !ctx.daemon_running {
        hints.push("blackbox start".to_string());
    }

    if ctx.daemon_running && !ctx.has_activity {
        hints.push("blackbox doctor".to_string());
    }

    if ctx.has_activity && ctx.llm_configured {
        hints.push(format!("blackbox {} --summarize", cmd_name));
    }

    if ctx.has_activity {
        hints.push("blackbox live".to_string());
    }

    hints.truncate(3);
    hints
}
