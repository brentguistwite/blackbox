use clap::Parser;
use blackbox::cli::{Cli, Commands};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { watch_dirs, poll_interval } => {
            blackbox::config::run_init(watch_dirs, poll_interval)?;
        }
    }

    Ok(())
}
