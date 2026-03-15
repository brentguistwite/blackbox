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
}
