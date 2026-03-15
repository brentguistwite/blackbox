use clap::Parser;
use blackbox::cli::{Cli, Commands};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init => {
            println!("blackbox init: not yet implemented");
        }
    }

    Ok(())
}
