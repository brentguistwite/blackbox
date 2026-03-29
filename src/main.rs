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
    summarize: bool,
) -> anyhow::Result<()> {
    let config = blackbox::config::load_config()?;
    let data_dir = blackbox::config::data_dir()?;
    let db_path = data_dir.join("blackbox.db");
    let conn = blackbox::db::open_db(&db_path)
        .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;

    let (from, to) = range_fn();
    let repos = blackbox::query::query_activity(
        &conn,
        from,
        to,
        config.session_gap_minutes,
        config.first_commit_minutes,
    )?;

    // PR data now sourced from pr_snapshots table via `blackbox prs` command.
    // No live gh subprocess calls during today/week/month queries.

    let streak_days = blackbox::query::query_streak(&conn, config.streak_exclude_weekends)
        .unwrap_or_else(|e| {
            log::warn!("Failed to compute streak: {}", e);
            0
        });

    let total_commits: usize = repos.iter().map(|r| r.commits).sum();
    let total_reviews: usize = repos.iter().map(|r| {
        r.reviews.iter().map(|rv| rv.pr_number).collect::<std::collections::BTreeSet<_>>().len()
    }).sum();
    let total_time = blackbox::query::global_estimated_time(&repos, config.session_gap_minutes, config.first_commit_minutes);
    let total_ai_session_time = repos.iter().fold(chrono::Duration::zero(), |acc, r| {
        acc + r.ai_sessions.iter().fold(chrono::Duration::zero(), |a, s| a + s.duration)
    });

    let summary = ActivitySummary {
        period_label: period_label.to_string(),
        total_commits,
        total_reviews,
        total_repos: repos.len(),
        total_estimated_time: total_time,
        total_ai_session_time,
        streak_days,
        total_branch_switches: repos.iter().map(|r| r.branch_switches).sum(),
        repos,
    };

    if summarize {
        let llm_config = blackbox::llm::build_llm_config(&config)?;
        let json = blackbox::output::render_json(&summary);
        blackbox::llm::summarize_activity(&llm_config, &json)?;
    } else {
        match format {
            OutputFormat::Pretty => blackbox::output::render_summary(&summary),
            OutputFormat::Json => println!("{}", blackbox::output::render_json(&summary)),
            OutputFormat::Csv => println!("{}", blackbox::output::render_csv(&summary)),
        }
    }
    Ok(())
}

fn run_focus_query(week: bool) -> anyhow::Result<()> {
    let config = blackbox::config::load_config()?;
    let data_dir = blackbox::config::data_dir()?;
    let db_path = data_dir.join("blackbox.db");
    let conn = blackbox::db::open_db(&db_path)
        .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;

    let (label, range_fn): (&str, fn() -> (DateTime<Utc>, DateTime<Utc>)) = if week {
        ("This Week", blackbox::query::week_range)
    } else {
        ("Today", blackbox::query::today_range)
    };

    let (from, to) = range_fn();
    let repos = blackbox::query::query_activity(
        &conn, from, to, config.session_gap_minutes, config.first_commit_minutes,
    )?;

    let summary = ActivitySummary {
        period_label: label.to_string(),
        total_commits: repos.iter().map(|r| r.commits).sum(),
        total_reviews: 0,
        total_repos: repos.len(),
        total_estimated_time: chrono::Duration::zero(),
        total_ai_session_time: chrono::Duration::zero(),
        streak_days: 0,
        total_branch_switches: repos.iter().map(|r| r.branch_switches).sum(),
        repos,
    };

    println!("{}", blackbox::output::render_focus_report(&summary));
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Strip ANSI codes when stdout is not a TTY (piped/redirected)
    if !blackbox::output::is_tty() {
        colored::control::set_override(false);
    }

    // First-run detection: redirect to setup wizard if no config exists
    if !blackbox::config::config_exists() && !cli.command.is_exempt_from_config_check() {
        println!("Welcome to blackbox! No config found. Let's get you set up.\n");
        match blackbox::setup::run_setup() {
            Ok(()) => {
                println!("\nYou're all set! Run your command again to get started.");
                return Ok(());
            }
            Err(_) => {
                println!("\nNo worries! Set up manually anytime with: blackbox setup");
                return Ok(());
            }
        }
    }

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
        Commands::Reload => {
            let data_dir = blackbox::config::data_dir()?;
            blackbox::daemon::reload_daemon(&data_dir)?;
        }
        Commands::Today { format, json, csv, summarize } => {
            let fmt = blackbox::output::resolve_format(format, json, csv, blackbox::output::is_tty());
            run_query("Today", blackbox::query::today_range, fmt, summarize)?;
        }
        Commands::Week { format, json, csv, summarize } => {
            let fmt = blackbox::output::resolve_format(format, json, csv, blackbox::output::is_tty());
            run_query("This Week", blackbox::query::week_range, fmt, summarize)?;
        }
        Commands::Month { format, json, csv, summarize } => {
            let fmt = blackbox::output::resolve_format(format, json, csv, blackbox::output::is_tty());
            run_query("This Month", blackbox::query::month_range, fmt, summarize)?;
        }
        Commands::Install => {
            blackbox::service::install()?;
        }
        Commands::Uninstall => {
            blackbox::service::uninstall()?;
        }
        Commands::RunForeground => {
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            blackbox::daemon::run_foreground(config, &data_dir)?;
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
        Commands::Setup => {
            blackbox::setup::run_setup()?;
        }
        Commands::Live => {
            blackbox::tui::run_live()?;
        }
        Commands::Heatmap { weeks } => {
            blackbox::heatmap::run_heatmap(weeks)?;
        }
        Commands::Rhythm { days, format } => {
            blackbox::rhythm::run_rhythm(days, format)?;
        }
        Commands::Focus { week } => {
            run_focus_query(week)?;
        }
        Commands::Repo { path, format } => {
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)
                .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
            let dive = blackbox::repo_deep_dive::build_deep_dive(&path, &conn, &config)?;
            match format {
                OutputFormat::Pretty => blackbox::output::render_repo_pretty(&dive),
                OutputFormat::Json | OutputFormat::Csv => {
                    println!("{}", blackbox::output::render_repo_json(&dive));
                }
            }
        }
        Commands::Prs { days, repo, format } => {
            if days == 0 {
                anyhow::bail!("--days must be >= 1");
            }
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)?;
            let to = chrono::Utc::now();
            let from = to - chrono::Duration::days(days as i64);
            let stats = blackbox::query::query_pr_cycle_stats(&conn, repo.as_deref(), from, to)?;
            match format {
                OutputFormat::Pretty => println!("{}", blackbox::output::render_pr_cycle_stats(&stats)),
                OutputFormat::Json => println!("{}", blackbox::output::render_pr_cycle_json(&stats)),
                OutputFormat::Csv => anyhow::bail!("--format csv not supported for prs command"),
            }
        }
        Commands::Churn { window, repo, format } => {
            blackbox::churn::run_churn(window, repo, format)?;
        }
        Commands::Standup { week, json, csv, summarize } => {
            let label;
            let range_fn: fn() -> (DateTime<Utc>, DateTime<Utc>);
            if week {
                label = "This Week";
                range_fn = blackbox::query::week_range;
            } else {
                label = "Today";
                range_fn = blackbox::query::today_range;
            }
            let fmt = blackbox::output::resolve_format(OutputFormat::Pretty, json, csv, blackbox::output::is_tty());
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)
                .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
            let (from, to) = range_fn();
            let repos = blackbox::query::query_activity(
                &conn, from, to, config.session_gap_minutes, config.first_commit_minutes,
            )?;
            // PR data now sourced from pr_snapshots table via `blackbox prs` command.
            let total_commits: usize = repos.iter().map(|r| r.commits).sum();
            let total_reviews: usize = repos.iter().map(|r| {
        r.reviews.iter().map(|rv| rv.pr_number).collect::<std::collections::BTreeSet<_>>().len()
    }).sum();
            let total_time = blackbox::query::global_estimated_time(&repos, config.session_gap_minutes, config.first_commit_minutes);
            let total_ai_session_time = repos.iter().fold(chrono::Duration::zero(), |acc, r| {
                acc + r.ai_sessions.iter().fold(chrono::Duration::zero(), |a, s| a + s.duration)
            });
            let summary = ActivitySummary {
                period_label: label.to_string(),
                total_commits,
                total_reviews,
                total_repos: repos.len(),
                total_estimated_time: total_time,
                total_ai_session_time,
                streak_days: 0,
                total_branch_switches: repos.iter().map(|r| r.branch_switches).sum(),
                repos,
            };
            if summarize {
                let llm_config = blackbox::llm::build_llm_config(&config)?;
                let json = blackbox::output::render_json(&summary);
                blackbox::llm::summarize_activity(&llm_config, &json)?;
            } else {
                match fmt {
                    OutputFormat::Pretty => println!("{}", blackbox::output::render_standup(&summary)),
                    OutputFormat::Json => println!("{}", blackbox::output::render_json(&summary)),
                    OutputFormat::Csv => println!("{}", blackbox::output::render_csv(&summary)),
                }
            }
        }
    }

    Ok(())
}
