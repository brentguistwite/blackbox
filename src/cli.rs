use clap::{Parser, Subcommand};
use clap_complete::Shell;
use crate::output::OutputFormat;

#[derive(Parser)]
#[command(name = "blackbox", version, about = "Flight recorder for your dev day")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create default config interactively
    Init {
        /// Comma-separated watch directories (skips interactive prompt)
        #[arg(long)]
        watch_dirs: Option<String>,
        /// Poll interval in seconds (skips interactive prompt)
        #[arg(long)]
        poll_interval: Option<u64>,
    },
    /// Start the background daemon
    Start,
    /// Stop the running daemon
    Stop,
    /// Show daemon status (running/stopped)
    Status {
        /// Output format: pretty, json
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
    },
    /// Show today's git activity
    Today {
        /// Output format: pretty, json, csv
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
        /// Output JSON. Shape: {period_label, total_commits, total_reviews, total_repos,
        /// total_estimated_minutes, total_ai_session_minutes,
        /// repos: [{repo_name, repo_path, commits, branches, estimated_minutes,
        ///          events, pr_info?, reviews, ai_sessions}]}
        #[arg(long, conflicts_with = "csv")]
        json: bool,
        /// Emit CSV output (shorthand for --format csv)
        #[arg(long, conflicts_with = "json")]
        csv: bool,
        /// Summarize activity using LLM
        #[arg(long)]
        summarize: bool,
    },
    /// Show this week's git activity
    Week {
        /// Output format: pretty, json, csv
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
        /// Output JSON. Shape: {period_label, total_commits, total_reviews, total_repos,
        /// total_estimated_minutes, total_ai_session_minutes,
        /// repos: [{repo_name, repo_path, commits, branches, estimated_minutes,
        ///          events, pr_info?, reviews, ai_sessions}]}
        #[arg(long, conflicts_with = "csv")]
        json: bool,
        /// Emit CSV output (shorthand for --format csv)
        #[arg(long, conflicts_with = "json")]
        csv: bool,
        /// Summarize activity using LLM
        #[arg(long)]
        summarize: bool,
    },
    /// Show this month's git activity
    Month {
        /// Output format: pretty, json, csv
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
        /// Output JSON. Shape: {period_label, total_commits, total_reviews, total_repos,
        /// total_estimated_minutes, total_ai_session_minutes,
        /// repos: [{repo_name, repo_path, commits, branches, estimated_minutes,
        ///          events, pr_info?, reviews, ai_sessions}]}
        #[arg(long, conflicts_with = "csv")]
        json: bool,
        /// Emit CSV output (shorthand for --format csv)
        #[arg(long, conflicts_with = "json")]
        csv: bool,
        /// Summarize activity using LLM
        #[arg(long)]
        summarize: bool,
    },
    /// Register as OS service (launchd/systemd)
    Install,
    /// Remove OS service registration
    Uninstall,
    /// Run poll loop in foreground (used by service manager)
    #[command(name = "run-foreground", hide = true)]
    RunForeground,
    /// Print shell hook script for eval
    Hook {
        /// Shell type: zsh, bash, or fish
        #[arg(value_parser = ["zsh", "bash", "fish"])]
        shell: String,
    },
    /// Record directory presence (called by shell hook)
    #[command(name = "_notify-dir", hide = true)]
    NotifyDir {
        /// Directory path
        path: String,
    },
    /// Generate shell completions
    Completions {
        /// Shell type
        #[arg(value_parser = clap::value_parser!(Shell))]
        shell: Shell,
    },
    /// Run health checks and report status
    Doctor,
    /// Interactive setup wizard (full onboarding)
    Setup,
    /// Live TUI dashboard
    Live,
    /// Show contribution heatmap (GitHub-style green squares)
    Heatmap {
        /// Number of weeks to display (1-260)
        #[arg(long, default_value_t = 52)]
        weeks: u32,
    },
    /// Show work rhythm patterns (commit timing analysis)
    Rhythm {
        /// Number of days to analyze (default 30)
        #[arg(long, default_value_t = 30)]
        days: u64,
        /// Output format: pretty, json
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
    },
    /// Output activity in Slack/Teams-friendly format
    Standup {
        /// Show this week's activity instead of today
        #[arg(long)]
        week: bool,
        /// Output JSON. Shape: {period_label, total_commits, total_reviews, total_repos,
        /// total_estimated_minutes, total_ai_session_minutes,
        /// repos: [{repo_name, repo_path, commits, branches, estimated_minutes,
        ///          events, pr_info?, reviews, ai_sessions}]}
        #[arg(long, conflicts_with = "csv")]
        json: bool,
        /// Emit CSV output (shorthand for --format csv)
        #[arg(long, conflicts_with = "json")]
        csv: bool,
        /// Summarize activity using LLM
        #[arg(long)]
        summarize: bool,
    },
    /// Send SIGHUP to running daemon to reload config
    Reload,
    /// Show context-switch focus report
    Focus {
        /// Show this week instead of today
        #[arg(long)]
        week: bool,
    },
    /// Show PR cycle time metrics
    Prs {
        /// Number of days to analyze (default 30)
        #[arg(long, default_value_t = 30)]
        days: u64,
        /// Filter to a specific repo path
        #[arg(long)]
        repo: Option<String>,
        /// Output format: pretty, json
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
    },
    /// Single-repo deep dive (language breakdown, top files, time invested, PR history)
    Repo {
        /// Path to the git repository
        path: String,
        /// Output format: pretty, json
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
    },
    /// Show code churn rate for tracked repos
    Churn {
        /// Time window in days to detect churn (default: from config)
        #[arg(long)]
        window: Option<u32>,
        /// Filter to a specific repo path
        #[arg(long)]
        repo: Option<String>,
        /// Output format: pretty, json, csv
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
    },
}

impl Commands {
    /// Commands that don't require a config file to exist.
    pub fn is_exempt_from_config_check(&self) -> bool {
        matches!(
            self,
            Commands::Init { .. }
                | Commands::Status { .. }
                | Commands::Setup
                | Commands::Completions { .. }
                | Commands::Hook { .. }
                | Commands::RunForeground
                | Commands::NotifyDir { .. }
        )
    }
}
