use anyhow::Context;
use chrono::{DateTime, Utc};
use clap::{CommandFactory, Parser};
use blackbox::cli::{Cli, Commands};
use blackbox::output::OutputFormat;
use blackbox::query::ActivitySummary;

fn run_query(
    period_label: &str,
    range_fn: fn() -> (DateTime<Utc>, DateTime<Utc>),
    format: OutputFormat,
) -> anyhow::Result<()> {
    let config = blackbox::config::load_config()?;
    let data_dir = blackbox::config::data_dir()?;
    let db_path = data_dir.join("blackbox.db");
    let conn = blackbox::db::open_db(&db_path)
        .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;

    let (from, to) = range_fn();
    let mut repos = blackbox::query::query_activity(
        &conn,
        from,
        to,
        config.session_gap_minutes,
        config.first_commit_minutes,
    )?;

    blackbox::enrichment::enrich_with_prs(&mut repos);

    let total_commits: usize = repos.iter().map(|r| r.commits).sum();
    let total_time = repos
        .iter()
        .fold(chrono::Duration::zero(), |acc, r| acc + r.estimated_time);

    let summary = ActivitySummary {
        period_label: period_label.to_string(),
        total_commits,
        total_repos: repos.len(),
        total_estimated_time: total_time,
        repos,
    };

    match format {
        OutputFormat::Pretty => blackbox::output::render_summary(&summary),
        OutputFormat::Json => println!("{}", blackbox::output::render_json(&summary)),
        OutputFormat::Csv => println!("{}", blackbox::output::render_csv(&summary)),
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { watch_dirs, poll_interval } => {
            blackbox::config::run_init(watch_dirs, poll_interval)?;
        }
        Commands::Start => {
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            blackbox::daemon::start_daemon(config, &data_dir)?;
        }
        Commands::Stop => {
            let data_dir = blackbox::config::data_dir()?;
            blackbox::daemon::stop_daemon(&data_dir)?;
        }
        Commands::Status => {
            let data_dir = blackbox::config::data_dir()?;
            blackbox::daemon::daemon_status(&data_dir)?;
        }
        Commands::Today { format } => {
            run_query("Today", blackbox::query::today_range, format)?;
        }
        Commands::Week { format } => {
            run_query("This Week", blackbox::query::week_range, format)?;
        }
        Commands::Month { format } => {
            run_query("This Month", blackbox::query::month_range, format)?;
        }
        Commands::Install => {
            blackbox::service::install()?;
        }
        Commands::Uninstall => {
            blackbox::service::uninstall()?;
        }
        Commands::RunForeground => {
            let config = blackbox::config::load_config()?;
            blackbox::daemon::run_foreground(config)?;
        }
        Commands::Hook { shell } => {
            let script = blackbox::shell_hook::generate_hook(&shell)?;
            print!("{}", script);
        }
        Commands::NotifyDir { path } => {
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)?;
            blackbox::db::record_directory_presence(&conn, &path)?;
        }
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            clap_complete::aot::generate(shell, &mut cmd, "blackbox", &mut std::io::stdout());
        }
        Commands::Doctor => {
            let all_ok = blackbox::doctor::run_doctor()?;
            if !all_ok {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
