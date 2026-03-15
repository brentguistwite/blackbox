use clap::{Parser, Subcommand};
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
    Status,
    /// Show today's git activity
    Today {
        /// Output format: pretty, json, csv
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
    },
    /// Show this week's git activity
    Week {
        /// Output format: pretty, json, csv
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
    },
    /// Show this month's git activity
    Month {
        /// Output format: pretty, json, csv
        #[arg(long, default_value = "pretty")]
        format: OutputFormat,
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
}
