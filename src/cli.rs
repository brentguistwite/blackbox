use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use chrono::{DateTime, TimeZone, Utc};
use crate::output::OutputFormat;
use crate::query::{today_range, yesterday_range, week_range, month_range};

/// Shared time range for analytics commands.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum QueryRange {
    Today,
    Yesterday,
    Week,
    #[default]
    Month,
    All,
}

impl QueryRange {
    pub fn to_range(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        match self {
            QueryRange::Today => today_range(),
            QueryRange::Yesterday => yesterday_range(),
            QueryRange::Week => week_range(),
            QueryRange::Month => month_range(),
            QueryRange::All => {
                let epoch = Utc.timestamp_opt(0, 0).single().expect("epoch");
                (epoch, Utc::now())
            }
        }
    }
}

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
    Status,
    /// Show today's git activity
    Today {
        /// Output format: pretty, json, csv
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
        /// Summarize activity using LLM
        #[arg(long)]
        summarize: bool,
    },
    /// Show this week's git activity
    Week {
        /// Output format: pretty, json, csv
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
        /// Summarize activity using LLM
        #[arg(long)]
        summarize: bool,
    },
    /// Show this month's git activity
    Month {
        /// Output format: pretty, json, csv
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
        /// Summarize activity using LLM
        #[arg(long)]
        summarize: bool,
    },
    /// Show yesterday's git activity
    Yesterday {
        /// Output format: pretty, json, csv
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
        /// Summarize activity using LLM
        #[arg(long)]
        summarize: bool,
    },
    /// Query git activity for a custom date range
    Query {
        /// Start date (YYYY-MM-DD)
        #[arg(long, requires = "to")]
        from: String,
        /// End date (YYYY-MM-DD)
        #[arg(long, requires = "from")]
        to: String,
        /// Output format: pretty, json, csv
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
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
    /// Output activity in Slack/Teams-friendly format
    Standup {
        /// Show this week's activity instead of today
        #[arg(long)]
        week: bool,
        /// Summarize activity using LLM
        #[arg(long)]
        summarize: bool,
    },
}

impl Commands {
    /// Commands that don't require a config file to exist.
    pub fn is_exempt_from_config_check(&self) -> bool {
        matches!(
            self,
            Commands::Init { .. }
                | Commands::Setup
                | Commands::Completions { .. }
                | Commands::Hook { .. }
                | Commands::RunForeground
                | Commands::NotifyDir { .. }
        )
    }
}
