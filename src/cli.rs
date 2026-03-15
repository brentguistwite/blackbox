use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "blackbox", about = "Flight recorder for your dev day")]
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
    Today,
    /// Show this week's git activity
    Week,
    /// Show this month's git activity
    Month,
    /// Register as OS service (launchd/systemd)
    Install,
    /// Remove OS service registration
    Uninstall,
    /// Run poll loop in foreground (used by service manager)
    #[command(name = "run-foreground", hide = true)]
    RunForeground,
}
