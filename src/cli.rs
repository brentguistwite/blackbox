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
    Init,
}
